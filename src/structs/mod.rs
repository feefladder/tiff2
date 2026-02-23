mod entry;
pub use entry::{Directory, IfdEntry, Offset, TagData};
/// IFD struct for non-images
mod ifd;
pub use ifd::{entry_size, num_entries_size, offset_size, Ifd};
/// IFD struct and functions for IFDs related to images
mod image;
pub use image::{Image, StripDecodeState, TileAttributes};
/// Tags: type, and important ones here
pub mod tags;
pub use tags::{Tag, TagType};
/// Tiff struct that can hold multiple images. This should be thin and ideally
/// re-implemented for more specific tiff types
pub mod tiff;
pub use tiff::Tiff;

pub struct Point<T> {
    pub x: T,
    pub y: T,
}

pub struct BBox<T> {
    pub top_left: Point<T>,
    pub bot_right: Point<T>,
}
