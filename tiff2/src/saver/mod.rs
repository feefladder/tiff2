//! # Tiff saving
//!
//! ## in-depth
//!
//! These are notes for future me, or people interested in the general design
//!
//! ### API layering
//!
//! This was written after Tiff loading in-depth and compares the differences.
//! Saving has much more state to handle, so loading is a baseline and then
//! saving adds complexity where needed.
//!
//! Imagine the reader diagram on the writer side:
//! ```text
//!                ?Writer? - good name?
//!           +----/  \--------------+
//! interwebs----Push Saver-..-Tiff  |
//!           |                  \   |
//!           +-------------------TileSaver
//! ```
//! But now when TileSaver writes data, this will change the TileOffsets/ByteCounts that is metadata
//! So... the simplest solution (and sort of what GDAL does) is to write dummy data to the IFD and then at the end overwrite it
//! GDAL writes zeroes
//! Since ghost data is technically allowed, I think it'd also be very possible to do something like this:
//! ```text
//!            /-=vec![NODATA_TILE.offset;n_tiles]
//! |42|Ifds|TileOffsets|TileByteCounts|NODATA_TILE|Actual data tiles|
//!                         \-=vec![NODATA_TILE.blen();n_tiles]
//! ```
//! And then the "invalid state" is just "not a COG, still a TIFF"
//! but that does still require writing one tile, which needs a TileSaver in the metadata layer
//! So the real problem is that metadata and data are not separate when saving
//! whereas in loading, load metadata -> metadata immutable -> load tiles
//! but in saving, plan metadata -> write some metadata -> write tiles -> update metadata
//! So I'm not too sure about the layering there...
//!

use std::collections::BTreeMap;
use std::error::Error;
use std::sync::Arc;

use bytes::Bytes;
use derive_more::{Display, Error};

use crate::saver::metadata::{TiffExtSaverRegistry, TiffSaver};
use crate::saver::tile::{EncoderRegistry, TileSaver};
use crate::structs::TileCoord;

mod metadata;
mod sync;
mod tile;

pub struct TiffMetaWriter<Push, Saver> {
    push: Push,
    saver: Saver,
    extension_registry: Arc<TiffExtSaverRegistry>,
}

pub struct TiffWriter<Push, Saver> {
    push: Push,
    saver: Saver,
    tile_savers: BTreeMap<u64, TileSaver>,
    encoder_registry: EncoderRegistry,
}

#[derive(Debug, Display, Error, Clone, PartialEq)]
pub struct TiffWriteError;
pub type TiffWriteResult<T> = exn::Result<T, TiffWriteError>;

pub trait SyncTiffWriter {
    /// Return which tile indices are not written yet for the selected ifd
    fn to_write_tiles<'a>(
        &'a self,
        ifd_idx: usize,
    ) -> impl Iterator<Item = TileCoord> + use<'a, Self>;
    /// Write tiles for the given indices
    ///
    /// Tile datas should be properly sized
    fn write_tiles(
        &mut self,
        ifd_idx: usize,
        coords: &[TileCoord],
        tile_datas: &[&[u8]],
    ) -> TiffWriteResult<()>;
    /// Patch the ifd with the tile offsets/byte counts actually written
    ///
    /// This should be called at the end. Not doing so will result in a broken tiff file.
    fn patch_ifd(&mut self, ifd_idx: usize) -> TiffWriteResult<()>;
}

#[derive(Debug, Display, Clone, PartialEq)]
pub struct PushError(pub String);
impl Error for PushError {}
pub type PushResult<T> = exn::Result<T, PushError>;

pub trait SyncPush {
    fn push_range(&self, offset: u64, data: Bytes) -> PushResult<()>;
    fn push_ranges(&self, offsets: &[u64], data: Vec<Bytes>) -> PushResult<()> {
        offsets
            .iter()
            .zip(data)
            .try_for_each(|(offset, data)| self.push_range(*offset, data))
    }
}

#[derive(Debug, Display, Error, Clone, PartialEq)]
pub struct MetaWriteError;
pub type MetaWriteResult<T> = exn::Result<T, MetaWriteError>;

pub trait SyncMetaWriter<Push: SyncPush, Saver: TiffSaver>: Sized {
    /// Open a file/resource for writing a tiff into
    /// This could write the header already, or not?
    fn open(
        push: Push,
        saver: Saver,
        extension_registry: Arc<TiffExtSaverRegistry>,
    ) -> MetaWriteResult<Self>;
    /// Finalize this IFD and start writing the next IFD's data
    fn next(&mut self) -> MetaWriteResult<Option<u64>>;
}

impl<Push, Saver: TiffSaver> TiffMetaWriter<Push, Saver> {
    pub fn finish(self, encoder_registry: EncoderRegistry) -> TiffWriter<Push, Saver> {
        TiffWriter {
            push: self.push,
            saver: self.saver,
            tile_savers: todo!(
                "Actually create tile savers (these are not on-demand as in loading)"
            ),
            encoder_registry,
        }
    }
}
