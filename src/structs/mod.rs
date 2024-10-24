mod entry;
pub use entry::{BufferedEntry, Directory, IfdEntry};
/// IFD struct for non-images
mod ifd;
pub use ifd::Ifd;
/// IFD struct and functions for IFDs related to images
mod image;
pub use image::{ChunkOpts, Image};
/// Tags: type, and important ones here
pub mod tags;
pub use tags::{Tag, TagType};
/// Tiff struct that can hold multiple images. This should be thin and ideally
/// re-implemented for more specific tiff types
pub mod tiff;
/// Tag Value type and convenience functions
/// to be deprecated in favour of `BufferedEntry`
pub mod value;
