use std::{collections::BTreeMap, io::Cursor, ops::Range};

use crate::{
    decoder::EndianReader,
    error::{TiffError, TiffFormatError, TiffResult},
    structs::{Ifd, IfdEntry, Tag, TagData},
    ByteOrder,
};

pub mod error;
use error::MetaError;
use exn::{bail, ErrorExt, OptionExt, ResultExt};
pub type MetaResult<T> = exn::Result<T, MetaError>;

/// The size of an ifd entry
///
/// |field       |small|big|
/// |------------|:---:|:-:|
/// |tag         | 2   | 2 |
/// |type        | 2   | 2 |
/// |count       | 4   | 8 |
/// |value/offset| 4   | 8 |
/// |total       | 12  | 20|
#[inline]
#[must_use]
pub const fn entry_size(bigtiff: bool) -> u64 {
    if bigtiff {
        2 + 2 + 8 + 8
    } else {
        2 + 2 + 4 + 4
    }
}

/// The size of the `number of entries`
///
/// The start of an IFD is the number of entries in that IFD.
///
/// |small|big|
/// |-----|---|
/// |2    | 8 |
///
#[inline]
#[must_use]
pub const fn num_entries_size(bigtiff: bool) -> u64 {
    if bigtiff {
        8 // u64
    } else {
        2 // u16
    }
}

/// The size of an offset to an IFD
///
/// This is the same in the header as at the end of each IFD.
///
/// |small|big|
/// |-----|---|
/// |4    |8  |
///
pub const fn ifd_offset_size(bigtiff: bool) -> u64 {
    if bigtiff {
        8 // u64
    } else {
        4 // u32
    }
}

pub struct TiffLoader {
    bigtiff: bool,
    byte_order: ByteOrder,
    ifd_offsets: Vec<u64>,
    ifds: Vec<BTreeMap<Tag, IfdEntry>>,
}

/// So the idea is inversion-of-control, much like how `std::io::Copy` allows
/// one to build a writer/Loader in stead of a reader, and then people feed the
/// writer buffers. However, there's now also the thing of "Seek"-ing required,
/// since tiff files are conservatively characterized as icing sprinkled on a
/// cake, rather than a left-to-right coherent file format. Also, what would a nice api look like?
///
/// ```
/// let tiff_builder: TiffLoader = TiffLoader::new();
/// let SyncReader = File::new("some_path");
/// let AsyncReader = EHttpReader::new_async("https://example.com/file.tiff");
///
/// let tiff = tiff2::Copy(SyncReader, TiffLoader).read_all();
/// let tiff = tiff2::AsyncCopy(AsyncReader, TiffLoader).read_all();
/// ```
///
/// ...or something
///
/// where the (Async)Copy looks like
/// ```
/// let ranges = reader.fetch_ranges([0..1024*16]);
/// while let Err(RequiredRangesNotLoaded(req_ranges: &[Range<u64>])) = loader.load_tiff(ranges) {
///     ranges = reader.fetch_ranges(req_ranges)(.await)?;
/// }
/// ```
///
/// but then the question is, how to give the user more control? Like ideally,
/// it'd be some sort of iterator:
///
/// ```
/// // this opens the tiff, consolidating byte_order, bigtiff and next_ifd
/// builder.open(reader.fetch(tiff_builder.start()).await?);
///
/// // this only loads deferred
/// builder.load_ifd(reader.fetch(builder.next_ifd()).await?);
/// // so we'd need to
/// builder.load_tags(reader.fetch(builder.deferred_tags()).await?);
/// // I'm kind of thinking this may not be the most ergonomic, but...
///
/// builder.load_ifd(reader.fetch(builder.next_ifd())).await?);
/// builder
///     .load_tags(reader.fetch(
///         builder
///             .deferred_tags()
///             .filter(|tag| tag != Tag::TileOffsets && tag != Tag::TileByteCounts)
///     ).await?)
/// ```
///
/// and how to middlewares? like how to add some intermediate (sync) cache?
///
/// ```
/// let builder = TiffLoader::new().with_cache(CogCache::new())
/// ```
///
/// Or basically, have some `retry_sync` `retry_async` methods:
///
/// ```
/// let f = File::open("some.tiff");
/// builder.open().retry_with(|ranges| f.fetch(ranges));
/// drop(f);
/// let f = tokio::fs::File::open("some.tiff");
/// builder
///     .next_ifd()
///     .retry_with_async(async |ranges| f.fetch(ranges));
/// ```
impl TiffLoader {
    /// The range required to parse the header
    ///
    /// This is the required range for a bigtiff file
    pub fn header_range() -> Range<u64> {
        0..16
    }

    pub fn load_header(buf: &[u8]) -> MetaResult<Self> {
        if buf.len() < 16 {
            bail!(MetaError::missing_range(
                Self::header_range(),
                format!("could not load header with buffer size of {}", buf.len()),
            ));
        }
        let byte_order = match <&[u8; 2]>::try_from(&buf[0..2]) {
            Ok(b"II") => ByteOrder::LittleEndian,
            Ok(b"MM") => ByteOrder::BigEndian,
            e => {
                bail!(MetaError::permanent(format!(
                    "Failed to parse byte order mark, found {:?}",
                    e
                )));
            }
        };
        let mut r = EndianReader::wrap(std::io::Cursor::new(&buf[2..32]), byte_order);
        let bigtiff = match r
            .read_u16()
            .or_raise(|| MetaError::permanent(format!("failed to read magic number")))?
        {
            42 => false,
            43 => {
                if r.read_u16().or_raise(|| {
                    MetaError::permanent("Failed to read bigtiff offset bytesize".to_string())
                })? != 8
                {
                    bail!(MetaError::permanent(
                        "bigtiff offset byte size not 8".to_string()
                    ));
                }
                if r.read_u16().or_raise(|| {
                    MetaError::permanent("failed to read bigtiff reserved '0'".to_string())
                })? != 0
                {
                    bail!(MetaError::permanent(
                        "bigtiff reserved '0' not 0".to_string()
                    ))
                }
                true
            }
            v => {
                return Err(MetaError::permanent(format!(
                    "magic number {v:?} should be either 42 or 43"
                ))
                .into())
            }
        };
        let ifd_offsets = vec![if bigtiff {
            r.read_u64()
                .or_raise(|| MetaError::permanent("failed to read next ifd offset".to_string()))?
        } else {
            u64::from(
                r.read_u32().or_raise(|| {
                    MetaError::permanent("failed to read next ifd offset".to_string())
                })?,
            )
        }];
        let res = Self {
            bigtiff,
            byte_order,
            ifd_offsets,
            ifds: Vec::new(),
        };
        Ok(res)
    }

    /// Get the range that holds the next ifd's count value
    pub fn next_ifd_entry_count_range(&self) -> Range<u64> {
        let offset = *self.ifd_offsets.last().unwrap();
        offset..offset + if self.bigtiff { 8 } else { 2 }
    }

    /// given a buffer holding the count value, get the range of the full ifd
    pub fn ifd_range(&self, buf: &[u8], buf_start: u64) -> MetaResult<Range<u64>> {
        let mut r = EndianReader::wrap(Cursor::new(buf), self.byte_order);
        let count = if self.bigtiff {
            r.read_u64().or_raise(|| {
                MetaError::missing_range(
                    buf_start..buf_start + 8,
                    format!("could not read number of entries in ifd"),
                )
            })?
        } else {
            u64::from(r.read_u16().or_raise(|| {
                MetaError::missing_range(
                    buf_start..buf_start + 2,
                    format!("could not read number of entries in ifd"),
                )
            })?)
        };
        Ok(buf_start + num_entries_size(self.bigtiff)
            ..buf_start + num_entries_size(self.bigtiff) + entry_size(self.bigtiff) * count)
    }

    /// Given a buffer holding the ifd, get the underlying ifd
    pub fn load_ifd(&mut self, ifd_buf: &[u8], buf_start: u64, count: u64) -> MetaResult<()> {
        if u64::try_from(ifd_buf.len()).unwrap()
            < count * entry_size(self.bigtiff) + ifd_offset_size(self.bigtiff)
        {
            bail!(MetaError::missing_range(
                buf_start
                    ..buf_start + count * entry_size(self.bigtiff) + ifd_offset_size(self.bigtiff),
                format!("could not load IFD at offset {buf_start}")
            ))
        }
        let (ifd, next_offset) = Ifd::from_buffer(ifd_buf, count, self.byte_order, self.bigtiff)
            .expect("all reads should be in-range");
        if self.ifd_offsets.contains(&next_offset) {
            bail!(MetaError::permanent(format!(
                "cycle in offsets detected at offset {next_offset}"
            )));
        }
        self.ifd_offsets.push(next_offset);
        self.ifds.push(ifd.data);
        Ok(())
    }

    pub fn load_ifd_values(
        &mut self,
        ifd_offset: u64,
        bufs: &mut dyn Iterator<Item = (Tag, &[u8])>,
    ) -> MetaResult<()> {
        let ifd = &mut self.ifds[self
            .ifd_offsets
            .iter()
            .position(|v| *v == ifd_offset)
            .ok_or_raise(|| {
                MetaError::permanent(format!("No ifd at position {ifd_offset} loaded"))
            })?];
        // ah, so this is currently in get_tags() function of reader, which is super ugly...
        for (tag, buf) in bufs {
            let entry = ifd.get_mut(&tag).ok_or_raise(|| {
                MetaError::permanent(format!(
                    "no pre-existing entry (offset) for {tag:?} in ifd at {ifd_offset}"
                ))
            })?;
            *entry = IfdEntry::Value(match entry {
                IfdEntry::Offset(o) => {
                    TagData::from_buffer(buf, o.tag_type, o.count as usize, self.byte_order)
                }
                IfdEntry::Value(data) => {
                    TagData::from_buffer(buf, data.tag_type(), data.len(), self.byte_order)
                }
            });
        }
        Ok(())
    }
}
