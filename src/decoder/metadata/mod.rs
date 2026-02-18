use std::collections::BTreeMap;
use std::io::Cursor;
use std::ops::Range;

use crate::decoder::metadata::ifd::IfdLoader;
use crate::decoder::EndianReader;
use crate::error::{TiffError, TiffFormatError, TiffResult};
use crate::structs::{Ifd, IfdEntry, Tag, TagData};
use crate::ByteOrder;

mod cog;
pub mod error;
mod ifd;
use error::MetaError;
use exn::{bail, ErrorExt, OptionExt, ResultExt};
pub type MetaResult<T> = exn::Result<T, MetaError>;

pub struct Tiff {
    /// Whether we are big or small tiff
    bigtiff: bool,
    /// byte_order of the tiff file
    byte_order: ByteOrder,
    /// offsets to ifds
    ifd_offsets: Vec<u64>,
    /// all current ifds
    ifds: BTreeMap<u64, Ifd>,
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
impl Tiff {
    /// The range required to parse the header
    ///
    /// This is the required range for a bigtiff file
    pub fn header_range() -> Range<u64> {
        0..16
    }

    pub fn from_header(buf: &[u8]) -> MetaResult<Self> {
        if buf.len() < 16 {
            bail!(MetaError::invalid_buffer(
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
            ifds: BTreeMap::new(),
        };
        Ok(res)
    }

    /// Get the range that holds the next ifd's count value
    ///
    /// TODO: shold this return None on 0? e.g. at the end of the iterator?
    pub fn next_ifd_offset(&self) -> u64 {
        *self.ifd_offsets.last().unwrap()
    }

    pub fn ifd_loader(&mut self, buf: &[u8], offset: u64) -> MetaResult<IfdLoader> {
        let (ifd_loader, next_ifd) =
            IfdLoader::load_ifd(buf, offset, self.bigtiff, self.byte_order)?;
        if self.ifd_offsets.contains(&next_ifd) {
            bail!(MetaError::permanent(format!(
                "Cycle in offset detected at ifd {next_ifd}"
            )));
        }
        self.ifd_offsets.push(next_ifd);
        Ok(ifd_loader)
    }

    pub fn insert_ifd(&mut self, offset: u64, ifd: Ifd) {
        if !self.ifd_offsets.contains(&offset) {
            self.ifd_offsets.push(offset);
        }
        self.ifds.insert(offset, ifd);
    }
}
