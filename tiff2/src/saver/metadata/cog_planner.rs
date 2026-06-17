//! Planner struct internal layer thingy that manages where IFDs and such go
//!
//! This is an example of how an intermediary layer could work, as well as a TODO: fully-fledged COG writer
//!

use std::collections::BTreeMap;

use crate::saver::metadata::TiffSaver;
use crate::saver::tile::TileSaver;
use crate::structs::Tiff;

/// Plans ifd locations and ghost areas
/// Tiles will be placed after those
/// This writes a COG out-of-order (inserts data in the middle of the file)
/// So I guess it's similar to GDAL's mosaicking thing?
pub struct CogPlanner<Saver> {
    /// Where does each ifd go?
    // This is actually in tiff
    // so not too sure
    // but it'd be nice to have this be visual in some way with like Ratatui at some point
    ifd_plan: BTreeMap<usize, u64>,
    /// The idea is that we'd save a single all-nodata tile to make all TileOffsets point to
    single_tile_hack_saver: TileSaver,
    /// The underlying tiff (should be TiffSaver)
    saver: Saver,
}

impl<Saver: TiffSaver> TiffSaver for CogPlanner<Saver> {
    fn tiff(&self) -> &Tiff {
        self.saver.tiff()
    }
    fn tiff_mut(&mut self) -> &mut Tiff {
        self.saver.tiff_mut()
    }
    fn save_header(&mut self, buf: &mut [u8]) -> super::TiffSaveResult<super::TiffSaveResponse> {
        self.saver.save_header(buf)
    }
    fn ifd_saver(
        &mut self,
        ifd_offset: u64,
        extension_registry: std::sync::Arc<super::extension::TiffExtSaverRegistry>,
    ) -> super::TiffSaveResult<super::IfdSaveResponse> {
        self.saver.ifd_saver(ifd_offset, extension_registry)
    }
    fn resume_saver(
        &mut self,
        ranges: Vec<std::ops::Range<u64>>,
        buffers: Vec<&mut [u8]>,
        saver: super::ifd::IfdSaver,
    ) -> super::TiffSaveResult<super::IfdSaveResponse> {
        self.saver.resume_saver(ranges, buffers, saver)
    }
}
