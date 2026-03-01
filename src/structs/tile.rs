use bytes::Bytes;
use exn::bail;

use crate::{
    structs::{
        error::ChunkOptsError,
        tags::{
            CompressionMethod, PhotometricInterpretation, PlanarConfiguration, Predictor,
            SampleFormat,
        },
    },
    ByteOrder,
};

type ChunkOptsResult<T> = exn::Result<T, ChunkOptsError>;

#[derive(Debug, Clone, Copy)]
pub enum TileDataType {
    U8,
    U16,
    U32,
    U64,
    I8,
    I16,
    I32,
    I64,
    F32,
    F64,
}

/// Result of a decoding process
#[derive(Debug)]
pub enum TileData {
    /// A vector of unsigned bytes
    U8(Vec<u8>),
    /// A vector of unsigned words
    U16(Vec<u16>),
    /// A vector of 32 bit unsigned ints
    U32(Vec<u32>),
    /// A vector of 64 bit unsigned ints
    U64(Vec<u64>),
    /// A vector of 8 bit signed ints
    I8(Vec<i8>),
    /// A vector of 16 bit signed ints
    I16(Vec<i16>),
    /// A vector of 32 bit signed ints
    I32(Vec<i32>),
    /// A vector of 64 bit signed ints
    I64(Vec<i64>),
    /// A vector of 32 bit IEEE floats
    F32(Vec<f32>),
    /// A vector of 64 bit IEEE floats
    F64(Vec<f64>),
}

impl TileData {
    pub fn new(size: usize, dtype: TileDataType) -> TileData {
        match dtype {
            TileDataType::U8 => TileData::U8(vec![0; size]),
            TileDataType::U16 => TileData::U16(vec![0; size]),
            TileDataType::U32 => TileData::U32(vec![0; size]),
            TileDataType::U64 => TileData::U64(vec![0; size]),
            TileDataType::I8 => TileData::I8(vec![0; size]),
            TileDataType::I16 => TileData::I16(vec![0; size]),
            TileDataType::I32 => TileData::I32(vec![0; size]),
            TileDataType::I64 => TileData::I64(vec![0; size]),
            TileDataType::F32 => TileData::F32(vec![0.0; size]),
            TileDataType::F64 => TileData::F64(vec![0.0; size]),
        }
    }

    pub fn len(&self) -> usize {
        match self {
            TileData::U8(v) => v.len(),
            TileData::U16(v) => v.len(),
            TileData::U32(v) => v.len(),
            TileData::U64(v) => v.len(),
            TileData::F32(v) => v.len(),
            TileData::F64(v) => v.len(),
            TileData::I8(v) => v.len(),
            TileData::I16(v) => v.len(),
            TileData::I32(v) => v.len(),
            TileData::I64(v) => v.len(),
        }
    }

    pub fn is_empty(&self) -> bool {
        match self {
            TileData::U8(v) => v.is_empty(),
            TileData::U16(v) => v.is_empty(),
            TileData::U32(v) => v.is_empty(),
            TileData::U64(v) => v.is_empty(),
            TileData::F32(v) => v.is_empty(),
            TileData::F64(v) => v.is_empty(),
            TileData::I8(v) => v.is_empty(),
            TileData::I16(v) => v.is_empty(),
            TileData::I32(v) => v.is_empty(),
            TileData::I64(v) => v.is_empty(),
        }
    }
}

impl AsMut<[u8]> for TileData {
    fn as_mut(&mut self) -> &mut [u8] {
        match self {
            TileData::U8(buf) => bytemuck::cast_slice_mut(buf),
            TileData::U16(buf) => bytemuck::cast_slice_mut(buf),
            TileData::U32(buf) => bytemuck::cast_slice_mut(buf),
            TileData::U64(buf) => bytemuck::cast_slice_mut(buf),
            TileData::F32(buf) => bytemuck::cast_slice_mut(buf),
            TileData::F64(buf) => bytemuck::cast_slice_mut(buf),
            TileData::I8(buf) => bytemuck::cast_slice_mut(buf),
            TileData::I16(buf) => bytemuck::cast_slice_mut(buf),
            TileData::I32(buf) => bytemuck::cast_slice_mut(buf),
            TileData::I64(buf) => bytemuck::cast_slice_mut(buf),
        }
    }
}

/// Struct that holds all relevant metadata that is needed to encode/decode a chunk
/// (strip or tile).
/// this does not include chunkoffsets or -bytes, since loading of the tile/strip is separate
///
/// Cheaply cloneable
#[derive(Debug, PartialEq, Clone)]
pub struct ChunkOpts {
    /// tiff byte order
    pub byte_order: ByteOrder,
    /// width of the image in pixels
    pub image_width: u32,
    /// height of the image in pixels
    pub image_height: u32,
    /// bits per sample
    pub bits_per_sample: u8,
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
    pub jpeg_tables: Option<Bytes>,
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

impl ChunkOpts {
    /// Converts a tile's x and y coordinate to a flat index.
    ///
    ///
    pub(crate) fn xy2i(&self, x: u32, y: u32) -> usize {
        usize::try_from(x).unwrap() + usize::try_from(y * self.chunks_across()).unwrap()
    }

    /// Get the underlying datatype for options
    ///
    /// Only supports full-byte-width data
    pub(crate) fn dtype(&self) -> ChunkOptsResult<TileDataType> {
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
                bail!(ChunkOptsError::invalid_datatype(sample_format, bit_depth));
            }
        })
    }

    pub(crate) fn input_row_stride(&self, x: u32) -> ChunkOptsResult<usize> {
        match self.predictor {
            Predictor::FloatingPoint => {
                Ok((self.tile_width as usize).saturating_mul(self.bits_per_pixel() / 8))
            }
            _ => self.output_row_stride(x),
        }
    }
    /// The length of a chunk row in bytes, taking padding into account.
    ///
    pub(crate) fn output_row_stride(&self, x: u32) -> ChunkOptsResult<usize> {
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
    pub fn chunk_width_pixels(&self, x: u32) -> ChunkOptsResult<u32> {
        let chunks_across = self.chunks_across();
        if x >= chunks_across {
            Err(ChunkOptsError::index_error(x as _, chunks_across as _).into())
        } else if x == chunks_across - 1 {
            Ok(self.image_width - self.tile_width * x)
        } else {
            Ok(self.tile_width)
        }
    }
    pub fn chunk_height_pixels(&self, y: u32) -> ChunkOptsResult<u32> {
        let chunks_down = self.chunks_down();
        if y >= chunks_down {
            Err(ChunkOptsError::index_error(y as _, chunks_down as _).into())
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

    pub fn output_rows(&self, y: u32) -> ChunkOptsResult<usize> {
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
    pub fn chunk_data_dimensions(&self, x: u32, y: u32) -> ChunkOptsResult<(u32, u32)> {
        Ok((self.chunk_width_pixels(x)?, self.chunk_height_pixels(y)?))
    }

    // /// Derive colortype from info
    // ///
    // /// ## TODO: fix:
    // /// - RGB++
    // /// - TransparencyMask
    // /// - [CIELab](https://en.wikipedia.org/wiki/CIELAB_color_space)
    // pub fn colortype(&self) -> ChunkOptsResult<ColorType> {
    //     match self.photometric_interpretation {
    //         PhotometricInterpretation::RGB => match self.samples_per_pixel {
    //             3 => Ok(ColorType::RGB(self.bits_per_sample)),
    //             4 => Ok(ColorType::RGBA(self.bits_per_sample)),
    //             // FIXME: We should _ignore_ other components. In particular:
    //             // > Beware of extra components. Some TIFF files may have more components per pixel
    //             // than you think. A Baseline TIFF reader must skip over them gracefully,using the
    //             // values of the SamplesPerPixel and BitsPerSample fields.
    //             // > -- TIFF 6.0 Specification, Section 7, Additional Baseline requirements.
    //             _ => Err(TiffError::UnsupportedError(
    //                 TiffUnsupportedError::InterpretationWithBits(
    //                     self.photometric_interpretation,
    //                     vec![self.bits_per_sample; self.samples_per_pixel as usize],
    //                 ),
    //             )),
    //         },
    //         PhotometricInterpretation::CMYK => match self.samples_per_pixel {
    //             4 => Ok(ColorType::CMYK(self.bits_per_sample)),
    //             _ => Err(TiffError::UnsupportedError(
    //                 TiffUnsupportedError::InterpretationWithBits(
    //                     self.photometric_interpretation,
    //                     vec![self.bits_per_sample; self.samples_per_pixel as usize],
    //                 ),
    //             )),
    //         },
    //         PhotometricInterpretation::YCbCr => match self.samples_per_pixel {
    //             3 => Ok(ColorType::YCbCr(self.bits_per_sample)),
    //             _ => Err(TiffError::UnsupportedError(
    //                 TiffUnsupportedError::InterpretationWithBits(
    //                     self.photometric_interpretation,
    //                     vec![self.bits_per_sample; self.samples_per_pixel as usize],
    //                 ),
    //             )),
    //         },
    //         PhotometricInterpretation::BlackIsZero | PhotometricInterpretation::WhiteIsZero => {
    //             match self.samples_per_pixel {
    //                 1 => Ok(ColorType::Gray(self.bits_per_sample)),
    //                 _ => Ok(ColorType::Multiband {
    //                     bit_depth: self.bits_per_sample,
    //                     num_samples: self.samples_per_pixel,
    //                 }),
    //             }
    //         }
    //         // TODO: this is bad we should not fail at this point
    //         PhotometricInterpretation::RGBPalette
    //         | PhotometricInterpretation::TransparencyMask
    //         | PhotometricInterpretation::CIELab => Err(TiffError::UnsupportedError(
    //             TiffUnsupportedError::InterpretationWithBits(
    //                 self.photometric_interpretation,
    //                 vec![self.bits_per_sample; self.samples_per_pixel as usize],
    //             ),
    //         )),
    //     }
    // }
}
