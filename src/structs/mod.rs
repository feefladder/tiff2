mod entry;
pub use entry::{Directory, IfdEntry, Offset, TagData};
/// IFD struct for non-images
mod ifd;
pub use ifd::Ifd;
/// IFD struct and functions for IFDs related to images
mod image;
pub use image::{ChunkOpts, Image, StripDecodeState, TileAttributes};
/// Tags: type, and important ones here
pub mod tags;
pub use tags::{Tag, TagType};
/// Tiff struct that can hold multiple images. This should be thin and ideally
/// re-implemented for more specific tiff types
pub mod tiff;
pub use tiff::Tiff;
/// Tag Value type and convenience functions
/// to be deprecated in favour of `BufferedEntry`
pub mod value;

pub struct Point<T> {
    x: T,
    y: T,
}

pub struct BBox<T> {
    top_left: Point<T>,
    bot_right: Point<T>,
}
