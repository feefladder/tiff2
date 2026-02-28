use std::collections::BTreeMap;
use std::io::Cursor;
use std::ops::Range;

use crate::loader::EndianReader;
use crate::structs::tiff::header_size;
use crate::structs::Ifd;
use crate::ByteOrder;
use crate::{loader::metadata::ifd::IfdLoader, structs::Tiff};

mod cache;
pub mod error;
mod ifd;
use bytes::Bytes;
use error::MetaError;
use exn::{bail, ResultExt};

pub type MetaResult<T> = exn::Result<T, MetaError>;

// #[derive(Debug, Clone)]
// // TODO: should this be called `TiffLoader`? Should I like add additional methods to the bare Tiff struct?
// pub struct Tiff {
//     /// Whether we are big or small tiff
//     bigtiff: bool,
//     /// byte_order of the tiff file
//     byte_order: ByteOrder,
//     /// offsets to ifds
//     ifd_offsets: Vec<u64>,
//     /// all current ifds, indexed by offsets
//     ///
//     /// ```
//     /// let ifd_5 = self.ifds[self.ifd_offsets[5]]
//     /// ```
//     ifds: BTreeMap<u64, Ifd>,
// }

/// The main trait for a tiff loader to implement
///
/// Any loader that implements this trait can be used with [`async_load`] or
/// [`sync_load`] functions to create a functional loader
///
/// TODO: should there be a intermediate trait?
///
/// Anyways, some functions of the intermediate:
/// ```
/// trait TiffLoader {
///     fn next_ifd_offset(&self) -> Option<u64>;
///     /// get the if loader and next offset
///     fn ifd_loader(&self, buf: &[u8], offset: u64) -> MetaResult<(IfdLoader, u64)>;
///     /// insert this ifd into the tiff
///     fn insert_ifd(&mut self, offset: u64, ifd: Ifd, next_offset: u64)-> MetaResult<()>;
/// }
/// ```
///
/// but then it doesn't make super much sense to layer these? Or does it?
/// Actually it is quite possible to layer, but ideally there'd be only one
/// trait that says "I'm a loader"
pub trait TiffLoader: Sized + Send + Sync {
    /// Create this loader from a buffer containing at least the header
    ///
    fn from_header(buf: Bytes) -> MetaResult<Self>;

    /// Read this Ifd
    ///
    /// On Error, no state should be changed
    fn try_next(&mut self) -> MetaResult<Option<u64>>;

    /// Skip n ifds
    ///
    /// This method may actually load intermittent ifds from a cache
    ///
    /// And also it will always error on [`Tiff`]
    fn skip(&mut self, n: usize) -> MetaResult<Option<u64>>;
}

impl TiffLoader for Tiff {
    fn from_header(buf: Bytes) -> MetaResult<Self> {
        Tiff::from_header(&buf)
    }

    fn try_next(&mut self) -> MetaResult<Option<u64>> {
        self.next_ifd_offset()
            .map(|o| Err(self.ifd_loader(&[], o).unwrap_err()))
            .transpose()
    }

    fn skip(&mut self, _n: usize) -> MetaResult<Option<u64>> {
        self.try_next()
    }
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
    /// This is the required range for a bigtiff file, since we don't know whether it's big or small
    pub fn header_range() -> Range<u64> {
        0..header_size(true)
    }

    pub fn from_header(buf: &[u8]) -> MetaResult<Self> {
        if u64::try_from(buf.len()).unwrap() < header_size(true) {
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
        let mut r = EndianReader::wrap(std::io::Cursor::new(&buf[2..]), byte_order);
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
    /// Returns `None` if the value is `0` (indicating the end of the chain)
    pub fn next_ifd_offset(&self) -> Option<u64> {
        match self.ifd_offsets.last() {
            None => unreachable!("proper opening should add at least one ifd offset"),
            Some(0) => None,
            Some(v) => Some(*v),
        }
    }

    /// Get the ifd loader and insert the next offset into self
    ///
    /// Why do we have this ifd_loader concept at all? I mean the whole point
    /// (sort of) is to be able to also fix ifds after the fact, so maybe those
    /// functions are then exposed to two places anyways and ideally that'd be ?here?
    ///
    /// The main reason there's the [`IfdLoader`] is to have parity with
    /// async-tiff, and split the metadata loading from the tiff that does
    /// stuff... Maybe make it possible to re-create an ifdloader from an ifd?
    ///
    /// I think re-creating the loader is like totally acceptable. It is kind of
    /// nice to have the ifd struct which just holds data and the loader that
    /// knows how to load it??
    pub fn ifd_loader(&self, buf: &[u8], offset: u64) -> MetaResult<(IfdLoader, u64)> {
        let (ifd_loader, next_ifd) =
            IfdLoader::load_ifd(buf, offset, self.bigtiff, self.byte_order)?;
        if self.ifd_offsets.contains(&next_ifd) {
            bail!(MetaError::permanent(format!(
                "Cycle in offsets detected at ifd {next_ifd}"
            )));
        }
        Ok((ifd_loader, next_ifd))
    }

    /// Insert the ifd corresponding to the given offset
    pub fn insert_ifd(&mut self, offset: u64, ifd: Ifd, next_offset: u64) {
        if !self.ifd_offsets.contains(&offset) {
            // this is actaully bad a
            self.ifd_offsets.push(offset);
        }
        self.ifds.insert(offset, ifd);
        self.ifd_offsets.push(next_offset);
    }
}
