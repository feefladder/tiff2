use std::io::{Cursor, Read};

use crate::{
    decoder::{
        predict_f32, predict_f64,
        reader::{DeflateReader, LZWReader, PackBitsReader},
    },
    error::{TiffError, TiffFormatError, TiffResult, TiffUnsupportedError},
    structs::{
        tags::{CompressionMethod, PhotometricInterpretation, Predictor, SampleFormat, Tag},
        ChunkOpts,
    },
    ColorType,
};

// /// Struct that decodes a single chunk
pub struct ChunkDecoder; //<'r> {
                         //     chunk_opts: Arc<ChunkOpts>,
                         //     reader: Box<dyn Read + 'r>,
                         // }

impl ChunkDecoder {
    /// Create this decoder to decode compressed_data into out_bufs
    ///
    // fn spawn(compressed_data: &[u8], out_bufs: &[&mut [u8]], chunk_opts: ChunkOpts, chunk_index: u32) -> TiffResult<()>{
    //     let reader = ChunkDecoder::create_reader(
    //         compressed_data,
    //         chunk_opts.photometric_interpretation,
    //         chunk_opts.compression_method,
    //         compressed_data.len().try_into()?,
    //         chunk_opts.jpeg_tables.as_deref().map(|a| &**a),
    //     )?;
    //     ChunkDecoder::expand_chunk(reader, out_bufs, chunk_opts.output_row_stride(chunk_index)?,chunk_opts);
    //     Ok(())
    // }

    /// create a reader for the compression type of our image
    fn create_reader<'r, R: 'r + Read>(
        reader: R,
        photometric_interpretation: PhotometricInterpretation,
        compression_method: CompressionMethod,
        compressed_length: u64,
        jpeg_tables: Option<&[u8]>,
    ) -> TiffResult<Box<dyn Read + 'r>> {
        Ok(match compression_method {
            CompressionMethod::None => Box::new(reader),
            CompressionMethod::LZW => {
                Box::new(LZWReader::new(reader, usize::try_from(compressed_length)?))
            }
            CompressionMethod::PackBits => Box::new(PackBitsReader::new(reader, compressed_length)),
            CompressionMethod::Deflate | CompressionMethod::OldDeflate => {
                Box::new(DeflateReader::new(reader))
            }
            CompressionMethod::ModernJPEG => {
                if jpeg_tables.is_some() && compressed_length < 2 {
                    return Err(TiffError::FormatError(
                        TiffFormatError::InvalidTagValueType(Tag::JPEGTables.to_u16()),
                    ));
                }

                // Construct new jpeg_reader wrapping a SmartReader.
                //
                // JPEG compression in TIFF allows saving quantization and/or huffman tables in one
                // central location. These `jpeg_tables` are simply prepended to the remaining jpeg image data.
                // Because these `jpeg_tables` start with a `SOI` (HEX: `0xFFD8`) or __start of image__ marker
                // which is also at the beginning of the remaining JPEG image data and would
                // confuse the JPEG renderer, one of these has to be taken off. In this case the first two
                // bytes of the remaining JPEG data is removed because it follows `jpeg_tables`.
                // Similary, `jpeg_tables` ends with a `EOI` (HEX: `0xFFD9`) or __end of image__ marker,
                // this has to be removed as well (last two bytes of `jpeg_tables`).
                let jpeg_reader = match jpeg_tables {
                    Some(jpeg_tables) => {
                        let mut reader = reader.take(compressed_length);
                        reader.read_exact(&mut [0; 2])?;

                        Box::new(
                            Cursor::new(&jpeg_tables[..jpeg_tables.len() - 2])
                                .chain(reader.take(compressed_length)),
                        ) as Box<dyn Read>
                    }
                    None => Box::new(reader.take(compressed_length)),
                };

                let mut decoder = jpeg::Decoder::new(jpeg_reader);

                match photometric_interpretation {
                    PhotometricInterpretation::RGB => {
                        decoder.set_color_transform(jpeg::ColorTransform::RGB)
                    }
                    PhotometricInterpretation::WhiteIsZero => {
                        decoder.set_color_transform(jpeg::ColorTransform::None)
                    }
                    PhotometricInterpretation::BlackIsZero => {
                        decoder.set_color_transform(jpeg::ColorTransform::None)
                    }
                    PhotometricInterpretation::TransparencyMask => {
                        decoder.set_color_transform(jpeg::ColorTransform::None)
                    }
                    PhotometricInterpretation::CMYK => {
                        decoder.set_color_transform(jpeg::ColorTransform::CMYK)
                    }
                    PhotometricInterpretation::YCbCr => {
                        decoder.set_color_transform(jpeg::ColorTransform::YCbCr)
                    }
                    photometric_interpretation => {
                        return Err(TiffError::UnsupportedError(
                            TiffUnsupportedError::UnsupportedInterpretation(
                                photometric_interpretation,
                            ),
                        ));
                    }
                }

                let data = decoder.decode()?;

                Box::new(Cursor::new(data))
            }
            method => {
                return Err(TiffError::UnsupportedError(
                    TiffUnsupportedError::UnsupportedCompressionMethod(method),
                ))
            }
        })
    }

    /// expand a chunk of compressed image data.
    ///
    /// ```
    ///
    /// ```
    ///
    /// ## Has these steps:
    /// 1. check:
    ///    - colortype and predictor
    ///    - predictor and sample_format
    ///    - // compressed_bytes (should be checked upstream)
    ///    - `chunk_opts.output_row_stride(chunk_index)` should be smaller than
    ///      chunk width
    ///    - buffer should be big enough
    /// 2. create reader for compression
    /// 3. predict and account for padding (which deps on whether we are float)
    ///
    /// ## TODO for COG bbox decoding:
    /// - make it work on a `&[&mut[u8]]` for interleaving images
    ///   - only costs an extra
    ///     input.chunks_exact_mut(output_row_stride).collect() if we're working
    ///     on contiguous data
    /// - make it work on a subview that possibly has top-bot/left-right padding
    pub fn expand_chunk(
        compressed_data: &[u8],
        buf: &mut [u8],
        chunk_opts: ChunkOpts,
        chunk_index: u32,
    ) -> TiffResult<()> {
        // Validate that the color type is supported.
        let color_type = chunk_opts.colortype()?;
        match color_type {
            ColorType::RGB(n)
            | ColorType::RGBA(n)
            | ColorType::CMYK(n)
            | ColorType::YCbCr(n)
            | ColorType::Gray(n)
            | ColorType::Multiband {
                bit_depth: n,
                num_samples: _,
            } if n == 8 || n == 16 || n == 32 || n == 64 => {}
            ColorType::Gray(n)
            | ColorType::Multiband {
                bit_depth: n,
                num_samples: _,
            } if n < 8 => match chunk_opts.predictor {
                Predictor::None => {}
                Predictor::Horizontal => {
                    return Err(TiffError::UnsupportedError(
                        TiffUnsupportedError::HorizontalPredictor(color_type),
                    ));
                }
                Predictor::FloatingPoint => {
                    return Err(TiffError::UnsupportedError(
                        TiffUnsupportedError::FloatingPointPredictor(color_type),
                    ));
                }
            },
            type_ => {
                return Err(TiffError::UnsupportedError(
                    TiffUnsupportedError::UnsupportedColorType(type_),
                ));
            }
        }

        // Validate that the predictor is supported for the sample type.
        match (chunk_opts.predictor, chunk_opts.sample_format) {
            (Predictor::Horizontal, SampleFormat::Int | SampleFormat::Uint) => {}
            (Predictor::Horizontal, _) => {
                return Err(TiffError::UnsupportedError(
                    TiffUnsupportedError::HorizontalPredictor(color_type),
                ));
            }
            (Predictor::FloatingPoint, SampleFormat::IEEEFP) => {}
            (Predictor::FloatingPoint, _) => {
                return Err(TiffError::UnsupportedError(
                    TiffUnsupportedError::FloatingPointPredictor(color_type),
                ));
            }
            _ => {}
        }

        // TODO: move this check somewhere upstream
        // let compressed_bytes =
        //     chunk_opts.chunk_bytes
        //         .get(chunk_index as usize)
        //         .ok_or(TiffError::FormatError(
        //             TiffFormatError::InconsistentSizesEncountered(chunk_opts.chu),
        //         ))?;
        // if *compressed_bytes > limits.intermediate_buffer_size as u64 {
        //     return Err(TiffError::LimitsExceeded);
        // }

        let compression_method = chunk_opts.compression_method;
        let photometric_interpretation = chunk_opts.photometric_interpretation;
        let predictor = chunk_opts.predictor;
        let samples = chunk_opts.samples_per_pixel();
        // full chunk dimenseions
        let chunk_dims = chunk_opts.chunk_dimensions()?;
        let data_dims = chunk_opts.chunk_data_dimensions(chunk_index)?;
        let output_row_stride = chunk_opts.output_row_stride(chunk_index)?;

        let chunk_row_bits = (u64::from(chunk_dims.0) * u64::from(chunk_opts.bits_per_sample))
            .checked_mul(samples as u64)
            .ok_or(TiffError::LimitsExceeded)?;
        let chunk_row_bytes: usize = ((chunk_row_bits + 7) / 8).try_into()?;

        let data_row_bits = (u64::from(data_dims.0) * u64::from(chunk_opts.bits_per_sample))
            .checked_mul(samples as u64)
            .ok_or(TiffError::LimitsExceeded)?;
        let data_row_bytes: usize = ((data_row_bits + 7) / 8).try_into()?;

        // TODO: Should these return errors instead? => YES
        if output_row_stride < data_row_bytes {
            return Err(TiffFormatError::RowStrideLargerThanWidth(
                output_row_stride,
                data_row_bytes,
            )
            .into());
        }
        // if // assert!(buf.len() >= output_row_stride * (data_dims.1 as usize - 1) + data_row_bytes);

        let mut reader = Self::create_reader(
            std::io::Cursor::new(compressed_data),
            photometric_interpretation,
            compression_method,
            u64::try_from(compressed_data.len())?,
            chunk_opts.jpeg_tables.as_deref().map(|a| &**a),
        )?;

        if output_row_stride == chunk_row_bytes {
            let tile = &mut buf[..chunk_row_bytes * data_dims.1 as usize];
            reader.read_exact(tile)?;

            for row in tile.chunks_mut(chunk_row_bytes) {
                super::fix_endianness_and_predict(
                    row,
                    color_type.bit_depth(),
                    samples,
                    chunk_opts.byte_order,
                    predictor,
                );
            }
            if photometric_interpretation == PhotometricInterpretation::WhiteIsZero {
                super::invert_colors(tile, color_type, chunk_opts.sample_format);
            }
        } else if chunk_row_bytes > data_row_bytes
            && chunk_opts.predictor == Predictor::FloatingPoint
        {
            // The floating point predictor shuffles the padding bytes into the encoded output, so
            // this case is handled specially when needed.
            let mut encoded = vec![0u8; chunk_row_bytes];
            // create row as reference to buffer chunk
            for row in buf.chunks_mut(output_row_stride).take(data_dims.1 as usize) {
                reader.read_exact(&mut encoded)?;

                let row = &mut row[..data_row_bytes];
                match color_type.bit_depth() {
                    32 => predict_f32(&mut encoded, row, samples),
                    64 => predict_f64(&mut encoded, row, samples),
                    _ => unreachable!(),
                }
                if photometric_interpretation == PhotometricInterpretation::WhiteIsZero {
                    super::invert_colors(row, color_type, chunk_opts.sample_format);
                }
            }
        } else {
            for (i, row) in buf
                .chunks_mut(output_row_stride)
                .take(data_dims.1 as usize)
                .enumerate()
            {
                let row = &mut row[..data_row_bytes];
                reader.read_exact(row)?;

                println!("chunk={chunk_index}, index={i}");

                // Skip horizontal padding
                if chunk_row_bytes > data_row_bytes {
                    let len = u64::try_from(chunk_row_bytes - data_row_bytes)?;
                    std::io::copy(&mut reader.by_ref().take(len), &mut std::io::sink())?;
                }

                super::fix_endianness_and_predict(
                    row,
                    color_type.bit_depth(),
                    samples,
                    chunk_opts.byte_order,
                    predictor,
                );
                if photometric_interpretation == PhotometricInterpretation::WhiteIsZero {
                    super::invert_colors(row, color_type, chunk_opts.sample_format);
                }
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod test {
    #[test]
    fn test_no_test() {
        todo!("tests for expand_chunk")
    }
}
