use std::borrow::Cow;
use std::ops::Range;

use bytes::Bytes;

use crate::decoder::decoding_result::DataType;
use crate::decoder::tile::predictor::{unpredict_float, unpredict_hdiff};
use crate::decoder::tile::registry::DecoderRegistry;
use crate::decoder::TileData;
use crate::error::{TiffError, TiffResult, TiffUnsupportedError, UsageError};
use crate::structs::tags::{
    CompressionMethod, PhotometricInterpretation, PlanarConfiguration, Predictor, SampleFormat,
};
use crate::util::fix_endianness;
use crate::{ByteOrder, ColorType, NATIVE_ENDIAN};

mod error;
mod predictor;
mod registry;

pub struct TileServer<'a> {
    dtype: DataType,
    chunk_opts: ChunkOpts,
    tile_offsets: Cow<'a, [u64]>,
    tile_byte_counts: Cow<'a, [u32]>,
}

impl<'a> TileServer<'a> {
    pub fn tile_range(&self, x: u32, y: u32) -> Range<u64> {
        let i = self.chunk_opts.xy2i(x, y);
        let start = self.tile_offsets[i];
        start..start + u64::from(self.tile_byte_counts[i])
    }

    pub fn tiles_ranges<'b>(
        &'b self,
        coords: impl Iterator<Item = (u32, u32)> + 'b,
    ) -> impl Iterator<Item = (u32, u32, Range<u64>)> + 'b {
        coords.map(|(x, y)| (x, y, self.tile_range(x, y)))
    }

    pub fn get_tiles<'b>(
        &'b self,
        tile_datas: impl Iterator<Item = (u32, u32, Bytes)> + 'b,
    ) -> impl Iterator<Item = Tile> + 'b {
        tile_datas.map(|(x, y, buf)| Tile {
            x,
            y,
            dtype: self.dtype,
            chunk_opts: self.chunk_opts.clone(),
            compressed_bytes: buf,
        })
    }
}

pub struct Tile {
    x: u32,
    y: u32,
    dtype: DataType,
    chunk_opts: ChunkOpts,
    compressed_bytes: Bytes,
}

impl Tile {
    pub fn decode(self, decoder_registry: &DecoderRegistry) -> TiffResult<TileData> {
        // output size in number of samples
        let output_size = usize::try_from(self.chunk_opts.chunk_width_pixels(self.x)?)?
            .saturating_mul(usize::try_from(
                self.chunk_opts.chunk_height_pixels(self.y)?,
            )?)
            .saturating_mul(self.chunk_opts.samples_per_pixel.into());
        let mut res = TileData::new(output_size, self.dtype);
        self.decode_into(decoder_registry, res.as_buffer(0))?;
        Ok(res)
    }

    pub fn decode_into<'a>(
        self,
        decoder_registry: &DecoderRegistry,
        out_buf: &mut [u8],
    ) -> TiffResult<()> {
        let decoder = decoder_registry
            .as_ref()
            .get(&self.chunk_opts.compression_method)
            .ok_or(TiffError::UnsupportedError(
                TiffUnsupportedError::UnsupportedCompressionMethod(
                    self.chunk_opts.compression_method,
                ),
            ))?;
        match self.chunk_opts.predictor {
            Predictor::None => {
                decoder.decode_chunk(&self.compressed_bytes, out_buf, &self.chunk_opts)?;
                fix_endianness(
                    out_buf,
                    self.chunk_opts.byte_order,
                    NATIVE_ENDIAN,
                    self.chunk_opts.bits_per_sample,
                );
            }
            Predictor::Horizontal => {
                decoder.decode_chunk(&self.compressed_bytes, out_buf, &self.chunk_opts)?;
                unpredict_hdiff(out_buf, &self.chunk_opts, self.x)?;
            }
            Predictor::FloatingPoint => {
                let mut temp_buf = vec![
                    0u8;
                    self.chunk_opts.input_row_stride(self.x)?
                        * self.chunk_opts.chunk_height as usize
                ];
                decoder.decode_chunk(&self.compressed_bytes, &mut temp_buf, &self.chunk_opts)?;
                unpredict_float(&mut temp_buf, out_buf, &self.chunk_opts, self.x, self.y)?;
            }
        }
        Ok(())
    }
}

/// Struct that holds all relevant metadata that is needed to ecnode/decode a chunk
/// (strip or tile).
/// this does not include chunkoffsets or -bytes, since those may be partial and
/// then mutated. once we implement partial tags
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
    pub chunk_width: u32,
    /// Chunk height in pixels
    ///
    /// If this is a stripped tiff, `chunk_height=rows_per_strip`
    pub chunk_height: u32,
}

impl ChunkOpts {
    /// Converts a tile's x and y coordinate to a flat index.
    ///
    ///
    fn xy2i(&self, x: u32, y: u32) -> usize {
        usize::try_from(x).unwrap() + usize::try_from(y * self.chunks_across()).unwrap()
    }
    // /// Samples per pixel within chunk.
    // ///
    // /// In planar config, samples are stored in separate strips/chunks, also called bands.
    // ///
    // /// Example with `bits_per_sample = [8, 8, 8]` and `PhotometricInterpretation::RGB`:
    // /// * `PlanarConfiguration::Chunky` -> 3 (RGBRGBRGB...)
    // /// * `PlanarConfiguration::Planar` -> 1 (RRR...) (GGG...) (BBB...)
    // pub fn samples_per_pixel(&self) -> usize {
    //     match self.planar_config {
    //         PlanarConfiguration::Chunky => self.samples.into(),
    //         PlanarConfiguration::Planar => 1,
    //     }
    // }

    pub fn input_row_stride(&self, x: u32) -> TiffResult<usize> {
        match self.predictor {
            Predictor::FloatingPoint => {
                Ok((self.chunk_width as usize).saturating_mul(self.bits_per_pixel() / 8))
            }
            _ => self.output_row_stride(x),
        }
    }
    /// The length of a chunk row in bytes, taking padding into account.
    ///
    pub fn output_row_stride(&self, x: u32) -> TiffResult<usize> {
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
        self.image_width.div_ceil(self.chunk_width)
    }
    pub fn chunks_down(&self) -> u32 {
        self.image_height.div_ceil(self.chunk_height)
    }
    pub fn chunk_width_pixels(&self, x: u32) -> TiffResult<u32> {
        let chunks_across = self.chunks_across();
        if x >= chunks_across {
            Err(TiffError::UsageError(UsageError::InvalidChunkIndex(x)))
        } else if x == chunks_across - 1 {
            Ok(self.image_width - self.chunk_width * x)
        } else {
            Ok(self.chunk_width)
        }
    }
    pub fn chunk_height_pixels(&self, y: u32) -> TiffResult<u32> {
        let chunks_down = self.chunks_down();
        if y >= chunks_down {
            Err(TiffError::UsageError(UsageError::InvalidChunkIndex(y)))
        } else if y == chunks_down - 1 {
            Ok(self.image_height - self.chunk_height * y)
        } else {
            Ok(self.chunk_height)
        }
    }
    /// dimensions of a chunk, not taking padding into account.
    ///
    /// Can be directly deduced from ChunkType and corresponding data
    pub fn chunk_dimensions(&self) -> (u32, u32) {
        (self.chunk_width, self.chunk_height)
    }

    pub fn output_rows(&self, y: u32) -> TiffResult<usize> {
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
    pub fn chunk_data_dimensions(&self, x: u32, y: u32) -> TiffResult<(u32, u32)> {
        Ok((self.chunk_width_pixels(x)?, self.chunk_height_pixels(y)?))
    }

    /// Derive colortype from info
    ///
    /// ## TODO: fix:
    /// - RGB++
    /// - TransparencyMask
    /// - [CIELab](https://en.wikipedia.org/wiki/CIELAB_color_space)
    pub fn colortype(&self) -> TiffResult<ColorType> {
        match self.photometric_interpretation {
            PhotometricInterpretation::RGB => match self.samples_per_pixel {
                3 => Ok(ColorType::RGB(self.bits_per_sample)),
                4 => Ok(ColorType::RGBA(self.bits_per_sample)),
                // FIXME: We should _ignore_ other components. In particular:
                // > Beware of extra components. Some TIFF files may have more components per pixel
                // than you think. A Baseline TIFF reader must skip over them gracefully,using the
                // values of the SamplesPerPixel and BitsPerSample fields.
                // > -- TIFF 6.0 Specification, Section 7, Additional Baseline requirements.
                _ => Err(TiffError::UnsupportedError(
                    TiffUnsupportedError::InterpretationWithBits(
                        self.photometric_interpretation,
                        vec![self.bits_per_sample; self.samples_per_pixel as usize],
                    ),
                )),
            },
            PhotometricInterpretation::CMYK => match self.samples_per_pixel {
                4 => Ok(ColorType::CMYK(self.bits_per_sample)),
                _ => Err(TiffError::UnsupportedError(
                    TiffUnsupportedError::InterpretationWithBits(
                        self.photometric_interpretation,
                        vec![self.bits_per_sample; self.samples_per_pixel as usize],
                    ),
                )),
            },
            PhotometricInterpretation::YCbCr => match self.samples_per_pixel {
                3 => Ok(ColorType::YCbCr(self.bits_per_sample)),
                _ => Err(TiffError::UnsupportedError(
                    TiffUnsupportedError::InterpretationWithBits(
                        self.photometric_interpretation,
                        vec![self.bits_per_sample; self.samples_per_pixel as usize],
                    ),
                )),
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
            | PhotometricInterpretation::CIELab => Err(TiffError::UnsupportedError(
                TiffUnsupportedError::InterpretationWithBits(
                    self.photometric_interpretation,
                    vec![self.bits_per_sample; self.samples_per_pixel as usize],
                ),
            )),
        }
    }
}
