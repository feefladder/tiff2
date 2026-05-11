use crate::structs::TileOpts;

mod predictor;
mod registry;
pub use registry::EncoderRegistry;

pub struct TileSaver {
    pub(crate) tile_opts: TileOpts,
    pub(crate) tile_offsets: Vec<u64>,
    pub(crate) tile_byte_counts: Vec<u32>,
}
