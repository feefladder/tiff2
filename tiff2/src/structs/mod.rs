/// IFD struct for non-images
pub mod metadata;
pub(crate) use metadata::offset_tag_type;
pub use metadata::{
    entry_size, num_entries_size, offset_size, Ifd, IfdEntry, Offset, Tag, TagData, TagType,
};
/// IFD struct and functions for IFDs related to images
// mod image;
// pub use image::{Image, StripDecodeState, TileAttributes};
/// Tags: type, and important ones here

/// Tiff struct that can hold multiple images. This should be thin and ideally
/// re-implemented for more specific tiff types
pub mod tiff;
pub use tiff::Tiff;
pub mod error;
mod tile;
pub use tile::{TileCoord, TileData, TileDataType, TileOpts};

pub struct Point<T> {
    pub x: T,
    pub y: T,
}

pub struct BBox<T> {
    pub top_left: Point<T>,
    pub bot_right: Point<T>,
}
