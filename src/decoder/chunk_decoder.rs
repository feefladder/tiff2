use log::{debug, error};
use std::io::{Cursor, Read};

use crate::decoder::reader::{DeflateReader, LZWReader, PackBitsReader};
use crate::decoder::{unpredict_f32, unpredict_f64};
use crate::error::{TiffError, TiffFormatError, TiffResult, TiffUnsupportedError};
use crate::structs::tags::{
    CompressionMethod, PhotometricInterpretation, Predictor, SampleFormat, Tag,
};
use crate::structs::ChunkOpts;
use crate::ColorType;

// /// Struct that decodes a single chunk
pub struct ChunkDecoder; //<'r> {
                         //     chunk_opts: Arc<ChunkOpts>,
                         //     reader: Box<dyn Read + 'r>,
                         // }

impl ChunkDecoder {
    // /// Create this decoder to decode compressed_data into out_bufs
    // ///
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
        buf: &mut [&mut [u8]],
        chunk_opts: &ChunkOpts,
        chunk_index: u32,
    ) -> TiffResult<()> {
        debug!(
            "Expanding chunk {chunk_index:?} with shape {:?}x{:?}",
            buf.len(),
            buf[0].len()
        );

        let compression_method = chunk_opts.compression_method;
        let photometric_interpretation = chunk_opts.photometric_interpretation;
        let predictor = chunk_opts.predictor;
        let samples = chunk_opts.samples_per_pixel();
        // full chunk dimenseions
        let chunk_dims = chunk_opts.chunk_dimensions();
        let data_dims = chunk_opts.chunk_data_dimensions(chunk_index);
        let output_row_stride = chunk_opts.output_row_stride(chunk_index)?;

        let chunk_row_bits = (u64::from(chunk_dims.0) * u64::from(chunk_opts.bits_per_sample))
            .checked_mul(samples as u64)
            .ok_or(TiffError::LimitsExceeded)?;
        let chunk_row_bytes: usize = chunk_row_bits.div_ceil(8).try_into()?;

        let data_row_bits = (u64::from(data_dims.0) * u64::from(chunk_opts.bits_per_sample))
            .checked_mul(samples as u64)
            .ok_or(TiffError::LimitsExceeded)?;
        let data_row_bytes: usize = data_row_bits.div_ceil(8).try_into()?;

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
            // let tile = &mut buf[..chunk_row_bytes];
            for row in buf {
                reader.read_exact(row)?;
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
        } else if chunk_row_bytes > data_row_bytes
            && chunk_opts.predictor == Predictor::FloatingPoint
        {
            // The floating point predictor shuffles the padding bytes into the encoded output, so
            // this case is handled specially when needed.
            let mut encoded = vec![0u8; chunk_row_bytes];
            // create row as reference to buffer chunk
            for row in buf {
                //.chunks_mut(output_row_stride).take(data_dims.1 as usize) {
                reader.read_exact(&mut encoded)?;

                let row = &mut row[..data_row_bytes];
                match color_type.bit_depth() {
                    32 => unpredict_f32(&mut encoded, row, samples),
                    64 => unpredict_f64(&mut encoded, row, samples),
                    _ => unreachable!(),
                }
                if photometric_interpretation == PhotometricInterpretation::WhiteIsZero {
                    super::invert_colors(row, color_type, chunk_opts.sample_format);
                }
            }
        } else {
            for row in buf.iter_mut()
            // .chunks_mut(output_row_stride)
            // .take(data_dims.1 as usize)
            // .enumerate()
            {
                let row = &mut &mut row[..data_row_bytes];
                reader.read_exact(row)?;

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
