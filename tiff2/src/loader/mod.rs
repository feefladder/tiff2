//! # Tiff loading
//!
//!
//! ## in-depth
//!
//! These are notes for future me, or people interested in the general design.
//!
//! ### API layering
//!
//! This crate uses "inversion of control".
//!
//! That is, in stead of the `TiffLoader` holding a `Fetch` and acting as a
//! reader, the `Loader` holds a tiff and acts similar to a writer: you feed it
//! buffers and it writes a `Tiff` datastructure. This allows for a very thin
//! `async` layer and non-locking decoding. The `Reader` acts as a slightly
//! fancier [`std::io::Copy`] implementation.
//!
//! ```
//!               Reader
//!           +----/  \--------------+
//! interwebs---Fetch Loader-..-Tiff |
//!           |                   \  |
//!           +------------------TileLoader
//! ```
//!
//! #### Cache layer
//!
//! It is possible to insert a cache layer as a `Loader` in between the `Reader` and `Tiff`.
//!
//! ### internal data structures
//!
//! Tag data goes through three stages, all with their corresponding backing store
//!
//! ```text
//! - range request ->   `Bytes`
//!                         | fix endianness
//!                         | ensure alignment
//! - inside IFD -> `TagData(SmallVec<T>)`
//!                         | cast to same value, regardless of encoded type
//! - inside TileLoader -> `Cow<'a, [T]>`
//! ```
//!
//! That is: a `TileLoader` always has `TileByteCounts` as `Cow<'a, [u64]>`
//! where `TagData` could have been `u16`,`u32` or `u64`.
//!
//! #### reasoning
//!
//! - `Bytes` because it is zero-copy among slices (cache/memmap)
//! - `SmallVec` reads from `Bytes` are often misaligned, this allows bytemucking and mirrors tiff layout
//! - `Cow<'a, [T]>` Saves a copy in the happy path and is a vector otherwise
//!
//! This does mean that the `Tiff` needs to be kept alive inside `Reader`s, but that's ok
//!

use std::collections::BTreeMap;
use std::error::Error;
use std::ops::Range;
use std::sync::Arc;

#[cfg(feature = "async")]
use async_trait::async_trait;
use bytes::Bytes;
use derive_more::Display;

use crate::structs::{Ifd, Tiff, TileCoord, TileData, TileOpts};

pub(crate) mod metadata;
pub use metadata::{
    cache, CacheMiss, IfdLoader, TiffExtLoader, TiffExtLoaderFactory, TiffExtLoaderRegistry,
    TiffLoadError, TiffLoadResult, TiffLoader,
};
#[cfg(feature = "async")]
mod r#async;
#[cfg(feature = "sync")]
mod sync;
pub(crate) mod tile;
pub use tile::{CodingResult, DecoderRegistry, TileLoader};

pub type FetchResult<T> = exn::Result<T, FetchError>;
pub type MetaReadResult<T> = exn::Result<T, MetaReadError>;
pub type IfdReadResult<T> = exn::Result<T, IfdReadError>;
pub type ReadResult<T> = exn::Result<T, ReadError>;

#[cfg(feature = "sync")]
pub trait SyncFetch {
    fn fetch_range(&self, range: Range<u64>) -> FetchResult<Bytes>;
    fn fetch_ranges(&self, ranges: &[Range<u64>]) -> FetchResult<Vec<Bytes>> {
        ranges
            .iter()
            .map(|range| self.fetch_range(range.clone()))
            .collect::<FetchResult<Vec<Bytes>>>()
    }
}

#[cfg(feature = "async")]
#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
pub trait AsyncFetch: Send + Sync {
    async fn fetch_range(&self, range: Range<u64>) -> FetchResult<Bytes>;
    async fn fetch_ranges(&self, ranges: &[Range<u64>]) -> FetchResult<Vec<Bytes>> {
        let mut results = Vec::with_capacity(ranges.len());
        for range in ranges {
            let result = self.fetch_range(range.clone()).await?;
            results.push(result);
        }
        Ok(results)
    }
}

pub struct TiffMetaReader<Fetch, Loader> {
    fetch: Fetch,
    loader: Loader,
    /// The extension registry that loads extensions into loaders
    extension_registry: Arc<TiffExtLoaderRegistry>,
}

pub struct TiffIfdReader<'a, Fetch> {
    fetch: &'a Fetch,
    ifd_loader: IfdLoader,
}

#[cfg(feature = "sync")]
pub trait SyncMetaReader<Fetch: SyncFetch>: Sized {
    fn open(
        fetch: Fetch,
        prefetch: u64,
        extension_registry: Arc<TiffExtLoaderRegistry>,
    ) -> MetaReadResult<Self>;
    fn next(&mut self) -> MetaReadResult<Option<u64>>;
    fn skip(&mut self, n: usize) -> MetaReadResult<Option<u64>>;
}

pub trait IfdReader<'a, Fetch> {
    /// Create this reader from an Ifd
    fn wrap(fetch: &'a Fetch, ifd_loader: IfdLoader) -> Self;
    fn finish(self) -> Ifd;
}

impl<'a, Fetch> IfdReader<'a, Fetch> for TiffIfdReader<'a, Fetch> {
    fn wrap(fetch: &'a Fetch, ifd_loader: IfdLoader) -> Self {
        Self { fetch, ifd_loader }
    }
    fn finish(self) -> Ifd {
        self.ifd_loader.finish()
    }
}

#[cfg(feature = "sync")]
pub trait SyncIfdReader<'a, Fetch: SyncFetch>: IfdReader<'a, Fetch> + Sized {
    fn fill_deferred(&mut self) -> IfdReadResult<()>;
}

#[cfg(feature = "async")]
#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
pub trait AsyncIfdReader<'a, Fetch: AsyncFetch>:
    IfdReader<'a, Fetch> + Sized + Send + Sync
{
    async fn fill_deferred(&mut self) -> IfdReadResult<()>;
}

#[cfg(feature = "async")]
#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
pub trait AsyncMetaReader<Fetch: AsyncFetch, Loader: TiffLoader>: Sized + Send + Sync {
    async fn open(
        fetch: Fetch,
        prefetch: u64,
        extension_registry: Arc<TiffExtLoaderRegistry>,
    ) -> MetaReadResult<Self>;
    async fn next(&mut self) -> MetaReadResult<Option<u64>>;
    async fn skip(&mut self, n: usize) -> MetaReadResult<Option<u64>>;
}

impl<Fetch, Loader: TiffLoader> TiffMetaReader<Fetch, Loader> {
    pub fn into_inner(self) -> (Fetch, Loader) {
        (self.fetch, self.loader)
    }

    pub fn finish(self, decoder_registry: DecoderRegistry) -> TiffReader<Fetch> {
        TiffReader {
            fetch: self.fetch,
            tiff: self.loader.into_tiff(),
            tile_loaders: BTreeMap::new(),
            decoder_registry,
        }
    }
}

pub struct TiffReader<Fetch> {
    fetch: Fetch,
    // TODO: should this become a TiffLoader? or even Box<dyn TiffLoader>?
    // In any case, that'll allow reading
    tiff: Tiff,
    tile_loaders: BTreeMap<u64, TileLoader>,
    decoder_registry: DecoderRegistry,
}

impl<Fetch> TiffReader<Fetch> {
    pub fn tiff(&self) -> &Tiff {
        &self.tiff
    }

    pub fn tile_opts(&self, idx: usize) -> Option<&TileOpts> {
        self.tiff
            .ifd_offsets
            .get(idx)
            .and_then(|offset| self.tile_loaders.get(offset).map(|tl| &tl.tile_opts))
    }
}

#[cfg(feature = "sync")]
pub trait SyncReader {
    fn get_tiles(
        &self,
        ifd_idx: usize,
        coords: &[TileCoord],
    ) -> ReadResult<Vec<(TileCoord, CodingResult<TileData>)>>;
    fn prep_ifd(&mut self, ifd_idx: usize) -> ReadResult<()>;
}

#[cfg(feature = "async")]
#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
pub trait AsyncReader {
    async fn get_tiles(
        &self,
        ifd_idx: usize,
        coords: &[TileCoord],
    ) -> ReadResult<Vec<(TileCoord, CodingResult<TileData>)>>;
    async fn prep_ifd(&mut self, ifd_idx: usize) -> ReadResult<()>;
}

#[derive(Debug, Display, Clone, PartialEq)]
#[display("fetch erorr: {}", self.0)]
pub struct FetchError(pub String);
impl Error for FetchError {}

#[derive(Debug, Display, Clone, PartialEq)]
#[display("read error: {}", self.0)]
pub struct MetaReadError(pub String);
impl Error for MetaReadError {}

impl MetaReadError {
    pub fn fetch_error(message: String) -> Self {
        Self(message)
    }
}

#[derive(Debug, Display, Clone, PartialEq)]
#[display("ifd read error: {}", self.0)]
pub struct IfdReadError(String);
impl Error for IfdReadError {}

#[derive(Debug, Display, Clone, PartialEq)]
#[display("{kind}: {message}")]
pub struct ReadError {
    pub kind: ReadErrorKind,
    pub message: String,
}
impl Error for ReadError {}
#[derive(Debug, Display, Clone, PartialEq)]
pub enum ReadErrorKind {
    #[display("fatal")]
    Fatal,
    #[display("fetch error")]
    FetchError,
    #[display("no tile loader for {ifd_offset}")]
    NoTileLoader { ifd_offset: u64 },
}

impl ReadError {
    fn fetch_error(message: String) -> Self {
        Self {
            kind: ReadErrorKind::FetchError,
            message,
        }
    }

    fn fatal(message: String) -> Self {
        Self {
            kind: ReadErrorKind::Fatal,
            message,
        }
    }
}

#[cfg(test)]
mod test {
    use std::ops::Deref;

    use bytes::Bytes;
    use exn::Exn;

    use super::*;

    #[tokio::test]
    async fn test_circular_tiff() {
        #[async_trait]
        impl AsyncFetch for Bytes {
            async fn fetch_range(&self, range: Range<u64>) -> FetchResult<Bytes> {
                Ok(self.slice(range.start as usize..self.len().min(range.end as usize)))
            }
        }
        let tiff = Bytes::from_owner([
            //    0     1    2  3
            b'I', b'I', 42, 0, // header
            //  4 5 6 7
            8, 0, 0, 0, // first ifd offset, u32
            //  8 9
            0, 0, // first ifd entry count, u16
            //   A B C D
            14, 0, 0, 0, // next ifd offset, u32
            0, 0, // second ifd entry count, u16
            8, 0, 0, 0, // next ifd offset (points to 0)
        ]);
        let prefetch = tiff.len() as u64;
        let mut reader = <TiffMetaReader<Bytes, Tiff> as AsyncMetaReader<Bytes, Tiff>>::open(
            tiff,
            prefetch,
            Arc::new(Vec::new().into()),
        )
        .await
        .unwrap();
        for _ in 0..1 {
            assert!(reader.next().await.unwrap().is_some())
        }
        assert_eq!(&reader.next().await.unwrap_err().to_string(), &"hello")
    }
}
