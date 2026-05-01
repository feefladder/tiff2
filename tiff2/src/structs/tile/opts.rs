use std::error::Error;

use derive_more::Display;
use exn::{bail, ResultExt};

use super::TileCoord;
use crate::structs::metadata::tags::{
    CompressionMethod, PhotometricInterpretation, PlanarConfiguration, Predictor, SampleFormat,
};
use crate::structs::TileDataType;
use crate::{ByteOrder, ColorType};

/// Struct that holds all relevant metadata that is needed to encode/decode a chunk
/// (strip or tile).
/// this does not include chunkoffsets or -bytes, since loading of the tile/strip is separate
///
/// Cheaply cloneable
#[derive(Debug, PartialEq, Clone)]
pub struct TileOpts {
    /// tiff byte order
    pub byte_order: ByteOrder,
    /// width of the image in pixels
    pub image_width: u32,
    /// height of the image in pixels
    pub image_height: u32,
    /// bits per sample
    pub bits_per_sample: u16,
    /// samples per pixel
    pub samples_per_pixel: u16,
    /// datatype of samples
    pub sample_format: SampleFormat,
    /// photometric interpretation
    pub photometric_interpretation: PhotometricInterpretation,
    /// compression method
    ///
    pub compression_method: CompressionMethod,
    /// horizontal predictor type
    ///
    /// Allows for more efficient compression
    pub predictor: Predictor,
    /// Jpeg tables
    ///
    /// In case of ModernJPEG compression, the compression infomation _can_ be
    /// in this tag, where it is prepended to chunks before decoding.
    pub jpeg_tables: Option<Vec<u8>>,
    /// Planar configuration:
    ///
    /// example: RGB
    /// - Chunky: [RGBRGBRGB]
    /// - Planar: [RRR] [GGG] [BBB]
    pub planar_config: PlanarConfiguration,
    /// Chunk width in pixels
    ///
    /// If this is a stripped tiff, `chunk_width=image_width`
    pub tile_width: u32,
    /// Chunk height in pixels
    ///
    /// If this is a stripped tiff, `chunk_height=rows_per_strip`
    pub tile_height: u32,
}

pub type TileOptsResult<T> = exn::Result<T, TileOptsError>;
#[non_exhaustive]
#[derive(Debug, Display, Clone, PartialEq)]
#[display("TileOptsError: {message}")]
pub struct TileOptsError {
    pub message: String,
}
impl Error for TileOptsError {}

fn fatal(message: String) -> TileOptsError {
    TileOptsError { message }
}
fn index_error(given: usize, max: usize) -> TileOptsError {
    TileOptsError {
        message: format!("index {given} exceeds max {max}"),
    }
}

fn invalid_datatype(sample_format: SampleFormat, bit_depth: u16) -> TileOptsError {
    TileOptsError {
        message: format!("sample format {sample_format:?} with bit depth {bit_depth} unsupported"),
    }
}

impl TileOpts {
    /// Converts a tile's x and y coordinate to a flat index.
    pub(crate) fn coord2i(&self, coord: TileCoord) -> TileOptsResult<usize> {
        self.check_coord(coord)
            .or_raise(|| fatal("Could not convert coordinate".into()))?;
        let chacross = usize::try_from(self.chunks_across()).unwrap();
        let chdown = usize::try_from(self.chunks_down()).unwrap();
        Ok(usize::try_from(coord.x).unwrap()
            + usize::try_from(coord.y).unwrap() * chacross
            + usize::from(coord.band) * chacross * chdown)
    }

    pub fn planes(&self) -> u16 {
        match self.planar_config {
            PlanarConfiguration::Chunky => 1,
            PlanarConfiguration::Planar => self.samples_per_pixel,
        }
    }

    pub fn check_coord(&self, coord: TileCoord) -> TileOptsResult<()> {
        if coord.x < self.chunks_across()
            && coord.y < self.chunks_down()
            && coord.band < self.planes()
        {
            Ok(())
        } else {
            Err(TileOptsError {
                message: format!("coordinate {coord:?} invalid"),
            }
            .into())
        }
    }

    /// Get the underlying datatype for options
    ///
    /// Only supports full-byte-width data
    pub(crate) fn dtype(&self) -> TileOptsResult<TileDataType> {
        Ok(match (self.sample_format, self.bits_per_sample) {
            (SampleFormat::Uint, 8) => TileDataType::U8,
            (SampleFormat::Uint, 16) => TileDataType::U16,
            (SampleFormat::Uint, 32) => TileDataType::U32,
            (SampleFormat::Uint, 64) => TileDataType::U64,
            (SampleFormat::Int, 8) => TileDataType::I8,
            (SampleFormat::Int, 16) => TileDataType::I16,
            (SampleFormat::Int, 32) => TileDataType::I32,
            (SampleFormat::Int, 64) => TileDataType::I64,
            (SampleFormat::IEEEFP, 32) => TileDataType::F32,
            (SampleFormat::IEEEFP, 64) => TileDataType::F64,
            (sample_format, bit_depth) => {
                bail!(invalid_datatype(sample_format, bit_depth));
            }
        })
    }

    pub(crate) fn input_row_stride(&self, x: u32) -> TileOptsResult<usize> {
        match self.predictor {
            Predictor::FloatingPoint => {
                Ok((self.tile_width as usize).saturating_mul(self.bits_per_pixel() / 8))
            }
            _ => self.output_row_stride(x),
        }
    }
    /// The length of a chunk row in bytes, taking padding into account.
    ///
    pub(crate) fn output_row_stride(&self, x: u32) -> TileOptsResult<usize> {
        Ok((self.chunk_width_pixels(x)? as usize).saturating_mul(self.bits_per_pixel()) / 8)
    }

    pub fn bits_per_pixel(&self) -> usize {
        match self.planar_config {
            PlanarConfiguration::Chunky => {
                self.bits_per_sample as usize * self.samples_per_pixel as usize
            }
            PlanarConfiguration::Planar => self.bits_per_sample as usize,
        }
    }
    pub fn chunks_across(&self) -> u32 {
        self.image_width.div_ceil(self.tile_width)
    }
    pub fn chunks_down(&self) -> u32 {
        self.image_height.div_ceil(self.tile_height)
    }
    pub fn chunk_width_pixels(&self, x: u32) -> TileOptsResult<u32> {
        let chunks_across = self.chunks_across();
        if x >= chunks_across {
            Err(index_error(x as _, chunks_across as _).into())
        } else if x == chunks_across - 1 {
            Ok(self.image_width - self.tile_width * x)
        } else {
            Ok(self.tile_width)
        }
    }
    pub fn chunk_height_pixels(&self, y: u32) -> TileOptsResult<u32> {
        let chunks_down = self.chunks_down();
        if y >= chunks_down {
            Err(index_error(y as _, chunks_down as _).into())
        } else if y == chunks_down - 1 {
            Ok(self.image_height - self.tile_height * y)
        } else {
            Ok(self.tile_height)
        }
    }
    /// dimensions of a chunk, not taking padding into account.
    ///
    /// Can be directly deduced from ChunkType and corresponding data
    pub fn chunk_dimensions(&self) -> (u32, u32) {
        (self.tile_width, self.tile_height)
    }

    /// TODO: this is wrong, planar has separate chunks, so always the same number of rows
    pub fn output_rows(&self, y: u32) -> TileOptsResult<usize> {
        match self.planar_config {
            PlanarConfiguration::Chunky => Ok(self.chunk_height_pixels(y)? as usize),
            PlanarConfiguration::Planar => {
                Ok((self.chunk_height_pixels(y)? as usize)
                    .saturating_mul(self.samples_per_pixel as _))
            }
        }
    }

    /// return the dimensions of an expanded chunk, taking into account padding
    /// at the bottom and right side of the file.
    ///
    pub fn chunk_data_dimensions(&self, x: u32, y: u32) -> TileOptsResult<(u32, u32)> {
        Ok((self.chunk_width_pixels(x)?, self.chunk_height_pixels(y)?))
    }

    /// Derive colortype from info
    ///
    /// ## TODO: fix:
    /// - RGB++
    /// - TransparencyMask
    /// - [CIELab](https://en.wikipedia.org/wiki/CIELAB_color_space)
    pub fn colortype(&self) -> TileOptsResult<ColorType> {
        let invalid_color = || {
            Err(TileOptsError {
                message: format!(
                    "no colortype for {:?} with bit depth {} and {} samples per pixel",
                    self.photometric_interpretation, self.bits_per_sample, self.samples_per_pixel
                ),
            }
            .into())
        };
        match self.photometric_interpretation {
            PhotometricInterpretation::RGB => match self.samples_per_pixel {
                3 => Ok(ColorType::RGB(self.bits_per_sample)),
                4 => Ok(ColorType::RGBA(self.bits_per_sample)),
                // FIXME: We should _ignore_ other components. In particular:
                // > Beware of extra components. Some TIFF files may have more components per pixel
                // than you think. A Baseline TIFF reader must skip over them gracefully,using the
                // values of the SamplesPerPixel and BitsPerSample fields.
                // > -- TIFF 6.0 Specification, Section 7, Additional Baseline requirements.
                _ => invalid_color(),
            },
            PhotometricInterpretation::CMYK => match self.samples_per_pixel {
                4 => Ok(ColorType::CMYK(self.bits_per_sample)),
                _ => invalid_color(),
            },
            PhotometricInterpretation::YCbCr => match self.samples_per_pixel {
                3 => Ok(ColorType::YCbCr(self.bits_per_sample)),
                _ => invalid_color(),
            },
            PhotometricInterpretation::BlackIsZero | PhotometricInterpretation::WhiteIsZero => {
                match self.samples_per_pixel {
                    1 => Ok(ColorType::Gray(self.bits_per_sample)),
                    _ => Ok(ColorType::Multiband {
                        bit_depth: self.bits_per_sample,
                        num_samples: self.samples_per_pixel,
                    }),
                }
            }
            // TODO: this is bad we should not fail at this point
            PhotometricInterpretation::RGBPalette
            | PhotometricInterpretation::TransparencyMask
            | PhotometricInterpretation::CIELab => invalid_color(),
        }
    }
}
