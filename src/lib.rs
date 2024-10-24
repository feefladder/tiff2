//!
//!

/// for byte casting. Not sure if we can actually stomp in bytemuck as dependency.
pub mod bytecast;
/// Errors
pub mod error;
/// Generic utility functions that can be used for both decoding and encoding
pub mod util;

pub mod structs;

/// static decoding functions to be used with the Tiff/Image struct. Additionally an
/// opinionated decoder, optimized for COGs (without the geo part).
pub mod decoder;
/// static encoding functions to be used with Tiff/Image struct. Additionally,
/// opinionated COG-building encoder
pub mod encoder;

#[derive(Debug, Copy, Clone, PartialEq)]
pub enum ByteOrder {
    BigEndian,
    LittleEndian,
}

macro_rules! cast_fn {
    ($name:ident, $type:ty, $length:literal) => {
        /// cast a $lenght-byte array to $type, respecting byte order
        #[inline(always)]
        pub fn $name(&self, bytes: [u8; $length]) -> $type {
            match self {
                ByteOrder::LittleEndian => <$type>::from_le_bytes(bytes),
                ByteOrder::BigEndian => <$type>::from_be_bytes(bytes),
            }
        }
    };
}

impl ByteOrder {
    cast_fn!(u8, u8, 1);
    cast_fn!(i8, i8, 1);
    cast_fn!(u16, u16, 2);
    cast_fn!(i16, i16, 2);
    cast_fn!(u32, u32, 4);
    cast_fn!(i32, i32, 4);
    cast_fn!(u64, u64, 8);
    cast_fn!(i64, i64, 8);

    cast_fn!(f32, f32, 4);
    cast_fn!(f64, f64, 8);
}

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
    Gray(u8),

    /// Pixel contains R, G and B channels
    RGB(u8),

    /// Pixel is an index into a color palette
    Palette(u8),

    /// Pixel is grayscale with an alpha channel
    GrayA(u8),

    /// Pixel is RGB with an alpha channel
    RGBA(u8),

    /// Pixel is CMYK
    CMYK(u8),

    /// Pixel is YCbCr
    YCbCr(u8),

    /// Pixel has multiple bands/channels
    Multiband { bit_depth: u8, num_samples: u16 },
}

impl ColorType {
    fn bit_depth(&self) -> u8 {
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
