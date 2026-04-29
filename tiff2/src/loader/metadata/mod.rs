use std::collections::BTreeMap;
use std::fmt::Debug;
use std::ops::{Deref, Range};

use bytes::Bytes;
use exn::{bail, ensure};
use smallvec::smallvec;

use crate::structs::tiff::header_size;
use crate::structs::{Ifd, TagData, TagType, Tiff};
use crate::ByteOrder;

pub mod cache;
mod error;
pub use error::{CacheMiss, TiffLoadError};
mod ifd;
pub use ifd::IfdLoader;

pub type TiffLoadResult<T> = exn::Result<T, TiffLoadError>;

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
/// ```ignore
/// trait TiffLoader {
///     fn next_ifd_offset(&self) -> Option<u64>;
///     /// get the if loader and next offset
///     fn ifd_loader(&self, buf: &[u8], offset: u64) -> TiffLoadResult<(IfdLoader, u64)>;
///     /// insert this ifd into the tiff
///     fn insert_ifd(&mut self, offset: u64, ifd: Ifd, next_offset: u64)-> TiffLoadResult<()>;
/// }
/// ```
///
/// but then it doesn't make super much sense to layer these? Or does it?
/// Actually it is quite possible to layer, but ideally there'd be only one
/// trait that says "I'm a loader"
pub trait TiffLoader: Sized + Send + Sync {
    /// Create this loader from a buffer containing at least the header
    ///
    fn from_header(buf: Bytes) -> TiffLoadResult<TiffLoadResponse<Self>>;

    /// get a mutable reference to the underlying tiff
    fn tiff_mut(&mut self) -> &mut Tiff;

    /// Get an immutable reference to the underlying tiff
    fn tiff(&self) -> &Tiff;

    /// Get the IfdLoader for the given offset
    ///
    fn ifd_loader(&mut self, buf: Bytes, offset: u64) -> TiffLoadResult<IfdLoadResponse>;

    /// Give more data to the loader
    ///
    /// This does nothing on `Tiff`, but is needed for caching
    fn give_more_data(&mut self, ranges: Vec<Range<u64>>, data: Vec<Bytes>);

    /// Destroy the loader and get the inderlying tiff
    fn into_tiff(self) -> Tiff;
}

#[derive(Debug, Clone, PartialEq)]
pub enum TiffLoadResponse<T> {
    NeedData(Range<u64>),
    Complete(T),
}

impl<T> TiffLoadResponse<T> {
    pub fn unwrap(self) -> T {
        match self {
            Self::Complete(v) => v,
            Self::NeedData(d) => panic!("Called unwrap on TiffLoadResponse that needed {d:?}"),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum IfdLoadResponse {
    /// Data is needed to be able to read the full in-line ifd data
    NeedData(Range<u64>),
    /// in-line ifd data was read, but some tags may be missing.
    ///
    /// needed_data is a signal to higher layers that that data is requested by this layer.
    /// Therefore, it signals "I can further fill the ifd/tiff if you give me this data"
    Partial {
        ifd_loader: IfdLoader,
        next_ifd_offset: u64,
        needed_data: Vec<Range<u64>>,
    },
    /// All ifd tags were read successfully
    Complete(Ifd, u64),
}

impl TiffLoader for Tiff {
    fn tiff(&self) -> &Tiff {
        self
    }
    fn tiff_mut(&mut self) -> &mut Tiff {
        self
    }
    fn into_tiff(self) -> Tiff {
        self
    }
    fn from_header(buf: Bytes) -> TiffLoadResult<TiffLoadResponse<Tiff>> {
        if u64::try_from(buf.len()).unwrap() < header_size(false) {
            return Ok(TiffLoadResponse::NeedData(0..header_size(false)));
        }
        let byte_order = match <&[u8; 2]>::try_from(&buf[0..2]).unwrap() {
            b"II" => ByteOrder::LittleEndian,
            b"MM" => ByteOrder::BigEndian,
            e => {
                bail!(TiffLoadError::permanent(format!(
                    "failed to parse byte order mark, found {:x?}",
                    e
                )));
            }
        };
        let bigtiff = match u16::try_from(
            TagData::from_buffer(&buf[2..], TagType::SHORT, 1, byte_order).unwrap(),
        )
        .unwrap()
        {
            42 => false,
            43 => {
                if u64::try_from(buf.len()).unwrap() < header_size(true) {
                    return Ok(TiffLoadResponse::NeedData(0..header_size(true)));
                }
                let osize_zero =
                    TagData::from_buffer(&buf[4..], TagType::SHORT, 2, byte_order).unwrap();
                ensure!(
                    osize_zero == TagData::Short(smallvec![8, 0]),
                    TiffLoadError::permanent(format!(
                        "[offset_size, 0] should be [8,0], was {osize_zero:?}"
                    ))
                );
                true
            }
            v => bail!(TiffLoadError::permanent(format!(
                "magic number {v:?} should be either 42 or 43"
            ))),
        };
        let ifd_offsets = vec![if bigtiff {
            u64::try_from(TagData::from_buffer(&buf[8..], TagType::LONG8, 1, byte_order).unwrap())
                .unwrap()
        } else {
            u64::try_from(TagData::from_buffer(&buf[4..], TagType::LONG, 1, byte_order).unwrap())
                .unwrap()
        }];
        Ok(TiffLoadResponse::Complete(Tiff {
            bigtiff,
            byte_order,
            ifd_offsets,
            ifds: BTreeMap::new(),
        }))
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
    fn ifd_loader(&mut self, buf: Bytes, offset: u64) -> TiffLoadResult<IfdLoadResponse> {
        let (ifd_loader, next_ifd_offset) =
            IfdLoader::from_buffer(&buf, offset, self.bigtiff, self.byte_order)
                .map_err(|e| e.deref().clone())?;
        if self.ifd_offsets.contains(&next_ifd_offset) {
            Err(TiffLoadError::permanent(format!(
                "Cycle in offsets detected at ifd {next_ifd_offset}"
            ))
            .into())
        } else {
            Ok(IfdLoadResponse::Partial {
                ifd_loader,
                next_ifd_offset,
                needed_data: Vec::new(),
            })
        }
    }

    /// Does nothing, we do not have a cache or smart strategies
    fn give_more_data(&mut self, ranges: Vec<Range<u64>>, data: Vec<Bytes>) {
        eprintln!("give more data for ranges {ranges:?} bubbled down to Tiff")
    }
}

impl Tiff {
    pub fn next_ifd_offset(&self) -> Option<u64> {
        match self.ifd_offsets.last() {
            None => unreachable!("proper opening should add at least one ifd offset"),
            Some(0) => None,
            Some(v) => Some(*v),
        }
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

/// So the idea is inversion-of-control, much like how `std::io::Copy` allows
/// one to build a writer/Loader in stead of a reader, and then people feed the
/// writer buffers. However, there's now also the thing of "Seek"-ing required,
/// since tiff files are conservatively characterized as icing sprinkled on a
/// cake, rather than a left-to-right coherent file format. Also, what would a nice api look like?
///
/// ```ignore
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
/// ```ignore
/// let ranges = reader.fetch_ranges([0..1024*16]);
/// while let Err(RequiredRangesNotLoaded(req_ranges: &[Range<u64>])) = loader.load_tiff(ranges) {
///     ranges = reader.fetch_ranges(req_ranges)(.await)?;
/// }
/// ```
///
/// but then the question is, how to give the user more control? Like ideally,
/// it'd be some sort of iterator:
///
/// ```ignore
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
/// ```ignore
/// let builder = TiffLoader::new().with_cache(CogCache::new())
/// ```
///
/// Or basically, have some `retry_sync` `retry_async` methods:
///
/// ```ignore
/// let f = File::open("some.tiff");
/// builder.open().retry_with(|ranges| f.fetch(ranges));
/// drop(f);
/// let f = tokio::fs::File::open("some.tiff");
/// builder
///     .next_ifd()
///     .retry_with_async(async |ranges| f.fetch(ranges));
/// ```

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_from_header_small_le() {
        assert_eq!(
            Tiff::from_header(Bytes::copy_from_slice(&[
                b'I',
                b'I',
                42,
                0,
                header_size(false) as u8,
                0,
                0,
                0,
            ]))
            .unwrap()
            .unwrap(),
            Tiff {
                ifds: BTreeMap::new(),
                ifd_offsets: vec![header_size(false)],
                bigtiff: false,
                byte_order: ByteOrder::LittleEndian,
            }
        );
    }

    #[test]
    fn test_from_header_small_be() {
        assert_eq!(
            Tiff::from_header(Bytes::copy_from_slice(&[
                b'M',
                b'M',
                0,
                42,
                0,
                0,
                0,
                header_size(false) as u8
            ]))
            .unwrap()
            .unwrap(),
            Tiff {
                ifds: BTreeMap::new(),
                ifd_offsets: vec![header_size(false)],
                bigtiff: false,
                byte_order: ByteOrder::BigEndian,
            }
        );
    }

    #[test]
    #[rustfmt::skip]
    fn test_from_header_big_le() {
        assert_eq!(
            Tiff::from_header(Bytes::copy_from_slice(&[
                b'I',b'I',
                43,0,
                8,0,
                0,0,
                header_size(true) as u8,0,0,0,0,0,0,0,
            ])).unwrap().unwrap(),
            Tiff {
                ifds: BTreeMap::new(),
                ifd_offsets: vec![header_size(true)],
                bigtiff: true,
                byte_order: ByteOrder::LittleEndian,
            }
        );
    }

    #[test]
    #[rustfmt::skip]
    fn test_from_header_big_be() {
        assert_eq!(
            Tiff::from_header(Bytes::copy_from_slice(&[
                b'M',b'M',
                0,43,
                0,8,
                0,0,
                0,0,0,0,0,0,0,header_size(true) as u8
            ])).unwrap().unwrap(),
            Tiff {
                ifds: BTreeMap::new(),
                ifd_offsets: vec![header_size(true)],
                bigtiff: true,
                byte_order: ByteOrder::BigEndian,
            }
        );
    }

    #[test]
    fn test_too_short_buf_small() {
        assert_eq!(
            Tiff::from_header(Bytes::new()).unwrap(),
            TiffLoadResponse::NeedData(0..header_size(false))
        )
    }

    #[test]
    fn test_invalid_bom() {
        assert_eq!(
            Tiff::from_header(Bytes::copy_from_slice(&[0; header_size(false) as _]))
                .unwrap_err()
                .deref(),
            &TiffLoadError::permanent("failed to parse byte order mark, found [0, 0]".into())
        );
    }

    #[test]
    fn test_invalid_magic() {
        assert_eq!(
            //                                         |   bom  | |magic||  offset  |
            Tiff::from_header(Bytes::copy_from_slice(&[b'I', b'I', 41, 0, 0, 0, 0, 0,]))
                .unwrap_err()
                .deref(),
            &TiffLoadError::permanent("magic number 41 should be either 42 or 43".into())
        );
    }

    #[test]
    fn test_too_short_buf_big() {
        assert_eq!(
            //                                         |   bom  | |magic||osize_zero|
            Tiff::from_header(Bytes::copy_from_slice(&[b'I', b'I', 43, 0, 8, 0, 0, 0,])).unwrap(),
            TiffLoadResponse::NeedData(0..header_size(true))
        );
    }

    #[test]
    fn test_invalid_osize_zero() {
        assert_eq!(
            Tiff::from_header(Bytes::copy_from_slice(&[
                //  bom  | |magic||osize_zero||0  1  2  3  4  5  6  7|
                b'I', b'I', 43, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0
            ]))
            .unwrap_err()
            .deref(),
            &TiffLoadError::permanent("[offset_size, 0] should be [8,0], was Short([0, 0])".into())
        );
    }
}
