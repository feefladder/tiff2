//! Decoding and Encoding of TIFF Images, specifically made extensible,
//! well-tested an asynchronous (for COGs)
//!
//! TIFF (Tagged Image File Format) is a versatile image format that supports
//! lossless and lossy compression.
//!
//! # Related Links
//! * <https://web.archive.org/web/20210108073850/https://www.adobe.io/open/standards/TIFF.html> - The TIFF specification
//! * <https://download.osgeo.org/libtiff/doc/TIFF6.pdf> - Tiff spec as PDF

// #![allow(dead_code, unused_variables)]

/// Generic utility functions that can be used for both decoding and encoding
pub mod util;

pub mod structs;

/// static decoding functions to be used with the Tiff/Image struct. Additionally an
/// opinionated decoder, optimized for COGs (without the geo part).
pub mod loader;
/// static encoding functions to be used with Tiff/Image struct. Additionally,
/// opinionated COG-building encoder
pub mod saver;

#[derive(Debug, Copy, Clone, PartialEq)]
pub enum ByteOrder {
    BigEndian,
    LittleEndian,
}

#[cfg(target_endian = "big")]
pub const NATIVE_ENDIAN: ByteOrder = ByteOrder::BigEndian;
#[cfg(target_endian = "little")]
pub const NATIVE_ENDIAN: ByteOrder = ByteOrder::LittleEndian;

#[derive(Debug, Copy, Clone, PartialEq)]
/// Chunk type of the internal representation
pub enum ChunkType {
    Strip,
    Tile,
}

/// An enumeration over supported color types and their bit depths
#[derive(Copy, PartialEq, Eq, Debug, Clone, Hash)]
pub enum ColorType {
    /// Pixel is grayscale
    Gray(u16),

    /// Pixel contains R, G and B channels
    RGB(u16),

    /// Pixel is an index into a color palette
    Palette(u16),

    /// Pixel is grayscale with an alpha channel
    GrayA(u16),

    /// Pixel is RGB with an alpha channel
    RGBA(u16),

    /// Pixel is CMYK
    CMYK(u16),

    /// Pixel is YCbCr
    YCbCr(u16),

    /// Pixel has multiple bands/channels
    Multiband { bit_depth: u16, num_samples: u16 },
}

impl ColorType {
    fn bit_depth(&self) -> u16 {
        match *self {
            ColorType::Gray(b)
            | ColorType::RGB(b)
            | ColorType::Palette(b)
            | ColorType::GrayA(b)
            | ColorType::RGBA(b)
            | ColorType::CMYK(b)
            | ColorType::YCbCr(b)
            | ColorType::Multiband { bit_depth: b, .. } => b,
        }
    }
}
