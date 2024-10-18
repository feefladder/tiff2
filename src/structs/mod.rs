pub mod entry;
/// IFD struct for non-images
pub mod ifd;
/// IFD struct and functions for IFDs related to images
pub mod image;
/// Tags: type, and important ones here
pub mod tags;
/// Tiff struct that can hold multiple images. This should be thin and ideally
/// re-implemented for more specific tiff types
pub mod tiff;
/// Tag Value type and convenience functions
///
pub mod value;
