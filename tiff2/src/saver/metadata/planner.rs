//! Planner struct internal layer thingy that manages where IFDs and such go
//!

use std::collections::BTreeMap;

use crate::{
    saver::{metadata::TiffSaver, tile::TileSaver},
    structs::Tiff,
};

// TODO: make this generic at some point
/// Plans ifd locations and ghost areas
/// Tiles will be placed after those
/// This writes a COG out-of-order (inserts data in the middle of the file)
/// So I guess it's similar to GDAL's mosaicking thing?
pub struct CogPlanner {
    /// Where does each ifd go?
    // This is actually in tiff
    // so not too sure
    // but it'd be nice to have this be visual in some way with like Ratatui at some point
    ifd_plan: BTreeMap<usize, u64>,
    single_tile_hack_saver: TileSaver,
    tiff: Tiff,
}

impl TiffSaver for CogPlanner {
    fn tiff(&self) -> &Tiff {
        &self.tiff
    }
    fn tiff_mut(&mut self) -> &mut Tiff {
        &mut self.tiff
    }
    fn save_header(&mut self, buf: &mut [u8]) -> super::TiffSaveResult<super::TiffSaveResponse> {
        self.tiff.save_header(buf)
    }
    fn ifd_saver(
        &mut self,
        ifd_offset: u64,
        extension_registry: std::sync::Arc<super::extension::TiffExtSaverRegistry>,
    ) -> super::TiffSaveResult<super::IfdSaveResponse> {
        self.tiff.ifd_saver(ifd_offset, extension_registry)
    }
    fn resume_saver(
        &mut self,
        ranges: Vec<std::ops::Range<u64>>,
        buffers: Vec<&mut [u8]>,
        saver: super::ifd::IfdSaver,
    ) -> super::TiffSaveResult<super::IfdSaveResponse> {
        self.tiff.resume_saver(ranges, buffers, saver)
    }
}
