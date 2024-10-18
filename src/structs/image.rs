use crate::{
    ifd::Directory,
    tags::{
        CompressionMethod, PhotometricInterpretation, PlanarConfiguration, Predictor, SampleFormat,
    },
    ChunkType,
};

use std::sync::Arc;

#[derive(Debug, Clone)]
pub struct StripDecodeState {
    pub rows_per_strip: u32,
}

#[derive(Debug, Clone)]
/// Computed values useful for tile decoding
pub struct TileAttributes {
    pub image_width: usize,
    pub image_height: usize,

    pub tile_width: usize,
    pub tile_length: usize,
}

impl TileAttributes {
    pub fn tiles_across(&self) -> usize {
        (self.image_width + self.tile_width - 1) / self.tile_width
    }
    pub fn tiles_down(&self) -> usize {
        (self.image_height + self.tile_length - 1) / self.tile_length
    }
    fn padding_right(&self) -> usize {
        (self.tile_width - self.image_width % self.tile_width) % self.tile_width
    }
    fn padding_down(&self) -> usize {
        (self.tile_length - self.image_height % self.tile_length) % self.tile_length
    }
    pub fn get_padding(&self, tile: usize) -> (usize, usize) {
        let row = tile / self.tiles_across();
        let column = tile % self.tiles_across();

        let padding_right = if column == self.tiles_across() - 1 {
            self.padding_right()
        } else {
            0
        };

        let padding_down = if row == self.tiles_down() - 1 {
            self.padding_down()
        } else {
            0
        };

        (padding_right, padding_down)
    }
}

/// Image struct that holds all relevant metadata for locating an image's data in the file and which decoding method to use
pub struct Image {
    pub ifd: Option<Directory>,
    pub width: u32,
    pub height: u32,
    pub bits_per_sample: u8,
    pub samples: u16,
    pub sample_format: SampleFormat,
    pub photometric_interpretation: PhotometricInterpretation,
    pub compression_method: CompressionMethod,
    pub predictor: Predictor,
    pub jpeg_tables: Option<Arc<Vec<u8>>>,
    pub chunk_type: ChunkType,
    pub planar_config: PlanarConfiguration,
    pub strip_decoder: Option<StripDecodeState>,
    pub tile_attributes: Option<TileAttributes>,
    pub chunk_offsets: Vec<u64>,
    pub chunk_bytes: Vec<u64>,
}
