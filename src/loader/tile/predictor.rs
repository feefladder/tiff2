use crate::error::{TiffError, TiffResult, TiffUnsupportedError};
use crate::loader::tile::ChunkOpts;
use crate::util::fix_endianness;
use crate::NATIVE_ENDIAN;

/// reverse horizontal prediction
// from image-tiff
///
/// Horizontal prediction uses a horizontal differencing scheme (on full values)
/// That
fn rev_hpredict_nsamp(buf: &mut [u8], bit_depth: u8, samples: usize) {
    match bit_depth {
        0..=8 => {
            for i in samples..buf.len() {
                buf[i] = buf[i].wrapping_add(buf[i - samples]);
            }
        }
        9..=16 => {
            for i in (samples * 2..buf.len()).step_by(2) {
                let v = u16::from_ne_bytes(buf[i..][..2].try_into().unwrap());
                let p = u16::from_ne_bytes(buf[i - 2 * samples..][..2].try_into().unwrap());
                buf[i..][..2].copy_from_slice(&(v.wrapping_add(p)).to_ne_bytes());
            }
        }
        17..=32 => {
            for i in (samples * 4..buf.len()).step_by(4) {
                let v = u32::from_ne_bytes(buf[i..][..4].try_into().unwrap());
                let p = u32::from_ne_bytes(buf[i - 4 * samples..][..4].try_into().unwrap());
                buf[i..][..4].copy_from_slice(&(v.wrapping_add(p)).to_ne_bytes());
            }
        }
        33..=64 => {
            for i in (samples * 8..buf.len()).step_by(8) {
                let v = u64::from_ne_bytes(buf[i..][..8].try_into().unwrap());
                let p = u64::from_ne_bytes(buf[i - 8 * samples..][..8].try_into().unwrap());
                buf[i..][..8].copy_from_slice(&(v.wrapping_add(p)).to_ne_bytes());
            }
        }
        _ => {
            unreachable!("Caller should have validated arguments. Please file a bug.")
        }
    }
}

/// reverse horizontal predictor
///
/// fixes byte order before reversing differencing
pub(crate) fn unpredict_hdiff<'a>(
    buffer: &mut [u8],
    predictor_info: &ChunkOpts,
    tile_x: u32,
) -> TiffResult<()> {
    let output_row_stride = predictor_info.output_row_stride(tile_x)?;
    let samples = predictor_info.samples_per_pixel as usize;
    let bit_depth = predictor_info.bits_per_sample;

    for buf in buffer.chunks_exact_mut(output_row_stride) {
        if buf.len() != output_row_stride {
            return Err(TiffError::UsageError(
                crate::error::UsageError::InvalidBufferSize(output_row_stride, buf.len()),
            ));
        }
        fix_endianness(buf, predictor_info.byte_order, NATIVE_ENDIAN, bit_depth);
        rev_hpredict_nsamp(buf, bit_depth, samples);
    }
    Ok(())
}

/// Reverse a floating-point prediction
///
/// According to [the spec](http://chriscox.org/TIFFTN3d1.pdf), no external
/// byte-ordering should be done.
///
/// If the tile has horizontal padding, it will shorten the output.
pub(crate) fn unpredict_float<'a>(
    in_buf: &mut [u8],
    out_buf: &mut [u8],
    predictor_info: &ChunkOpts,
    x: u32,
    y: u32,
) -> TiffResult<()> {
    let output_row_stride = predictor_info.output_row_stride(x)?;
    let bit_depth = predictor_info.bits_per_sample;
    if predictor_info.chunk_width_pixels(x)? == predictor_info.chunk_width {
        // no special padding handling
        for (input, output) in in_buf
            .chunks_exact_mut(output_row_stride)
            .zip(out_buf.chunks_exact_mut(output_row_stride))
        {
            match bit_depth {
                16 => rev_predict_f16(input, output, predictor_info.samples_per_pixel as _),
                32 => rev_predict_f32(input, output, predictor_info.samples_per_pixel as _),
                64 => rev_predict_f64(input, output, predictor_info.samples_per_pixel as _),
                depth => {
                    return Err(TiffError::UnsupportedError(
                        TiffUnsupportedError::UnsupportedSampleDepth(depth),
                    ))
                }
            }
        }
    } else {
        // specially handle padding bytes
        let input_row_stride = predictor_info.input_row_stride(x)?;

        // allocate here so we can re-use
        let mut out_row = vec![0u8; input_row_stride];

        for (input, output) in in_buf
            .chunks_exact_mut(input_row_stride)
            .zip(out_buf.chunks_exact_mut(output_row_stride))
        {
            match bit_depth {
                16 => rev_predict_f16(input, &mut out_row, predictor_info.samples_per_pixel as _),
                32 => rev_predict_f32(input, &mut out_row, predictor_info.samples_per_pixel as _),
                64 => rev_predict_f64(input, &mut out_row, predictor_info.samples_per_pixel as _),
                depth => {
                    return Err(TiffError::UnsupportedError(
                        TiffUnsupportedError::UnsupportedSampleDepth(depth),
                    ))
                }
            }
            // remove the padding bytes
            output.copy_from_slice(&out_row[..output_row_stride]);
        }
    }
    Ok(())
}

/// Reverse floating point prediction
///
/// floating point prediction first shuffles the bytes and then uses horizontal
/// differencing
/// also performs byte-order conversion if needed.
pub fn rev_predict_f16(input: &mut [u8], output: &mut [u8], samples: usize) {
    // reverse horizontal differencing
    for i in samples..input.len() {
        input[i] = input[i].wrapping_add(input[i - samples]);
    }
    // reverse byte shuffle and fix endianness
    for (i, chunk) in output.chunks_exact_mut(2).enumerate() {
        chunk.copy_from_slice(&u16::to_ne_bytes(
            // convert to native-endian
            // floating predictor is be-like
            u16::from_be_bytes([input[i], input[input.len() / 2 + i]]),
        ));
    }
}

/// Reverse floating point prediction
///
/// floating point prediction first shuffles the bytes and then uses horizontal
/// differencing
/// also performs byte-order conversion if needed.
pub fn rev_predict_f32(input: &mut [u8], output: &mut [u8], samples: usize) {
    // reverse horizontal differencing
    for i in samples..input.len() {
        input[i] = input[i].wrapping_add(input[i - samples]);
    }
    // reverse byte shuffle and fix endianness
    for (i, chunk) in output.chunks_exact_mut(4).enumerate() {
        chunk.copy_from_slice(
            // convert to native-endian
            &u32::to_ne_bytes(
                // floating predictor is be-like
                u32::from_be_bytes([
                    input[i],
                    input[input.len() / 4 + i],
                    input[input.len() / 4 * 2 + i],
                    input[input.len() / 4 * 3 + i],
                ]),
            ),
        );
    }
}

/// Reverse floating point prediction
///
/// floating point prediction first shuffles the bytes and then uses horizontal
/// differencing
/// Also fixes byte order if needed (tiff's->native)
pub fn rev_predict_f64(input: &mut [u8], output: &mut [u8], samples: usize) {
    for i in samples..input.len() {
        input[i] = input[i].wrapping_add(input[i - samples]);
    }

    for (i, chunk) in output.chunks_exact_mut(8).enumerate() {
        chunk.copy_from_slice(
            // convert to native-endian
            &u64::to_ne_bytes(
                // floating predictor is be-like
                u64::from_be_bytes([
                    input[i],
                    input[input.len() / 8 + i],
                    input[input.len() / 8 * 2 + i],
                    input[input.len() / 8 * 3 + i],
                    input[input.len() / 8 * 4 + i],
                    input[input.len() / 8 * 5 + i],
                    input[input.len() / 8 * 6 + i],
                    input[input.len() / 8 * 7 + i],
                ]),
            ),
        );
    }
}

#[cfg(test)]
mod test {
    use std::vec;

    use super::*;
    use crate::structs::tags::{
        CompressionMethod, PhotometricInterpretation, PlanarConfiguration, Predictor, SampleFormat,
    };
    use crate::ByteOrder;

    const CHOPTS: ChunkOpts = ChunkOpts {
        byte_order: ByteOrder::LittleEndian,
        image_width: 7,
        image_height: 7,
        chunk_width: 4,
        chunk_height: 4,
        bits_per_sample: 8,
        samples_per_pixel: 1,
        sample_format: SampleFormat::Void,
        photometric_interpretation: PhotometricInterpretation::BlackIsZero,
        compression_method: CompressionMethod::Deflate,
        predictor: Predictor::FloatingPoint,
        jpeg_tables: None,
        planar_config: PlanarConfiguration::Chunky,
    };
    #[rustfmt::skip]
    const RES: [u8;16] = [
        0,1, 2,3,
        1,0, 1,2,

        2,1, 0,1,
        3,2, 1,0,
        ];
    #[rustfmt::skip]
    const RES_RIGHT: [u8;12] = [
        0,1, 2,
        1,0, 1,

        2,1, 0,
        3,2, 1,
        ];
    #[rustfmt::skip]
    const RES_BOT: [u8;12] = [
        0,1,2, 3,
        1,0,1, 2,

        2,1,0, 1,
        ];
    #[rustfmt::skip]
    const RES_BOT_RIGHT: [u8;9] = [
        0,1, 2,
        1,0, 1,

        2,1, 0,
        ];

    #[test]
    fn test_chunk_width_pixels() {
        let info = CHOPTS;
        assert_eq!(info.chunks_across(), 2);
        assert_eq!(info.chunks_down(), 2);
        assert_eq!(info.chunk_width_pixels(0).unwrap(), info.chunk_width);
        assert_eq!(info.chunk_width_pixels(1).unwrap(), 3);
        info.chunk_width_pixels(2).unwrap_err();
        assert_eq!(info.chunk_height_pixels(0).unwrap(), info.chunk_height);
        assert_eq!(info.chunk_height_pixels(1).unwrap(), 3);
        info.chunk_height_pixels(2).unwrap_err();
    }

    #[test]
    fn test_output_row_stride() {
        let mut info = CHOPTS;
        assert_eq!(info.output_row_stride(0).unwrap(), 4);
        assert_eq!(info.output_row_stride(1).unwrap(), 3);
        info.output_row_stride(2).unwrap_err();
        info.samples_per_pixel = 2;
        assert_eq!(info.output_row_stride(0).unwrap(), 8);
        assert_eq!(info.output_row_stride(1).unwrap(), 6);
        info.bits_per_sample = 16;
        assert_eq!(info.output_row_stride(0).unwrap(), 16);
        assert_eq!(info.output_row_stride(1).unwrap(), 12);
        info.planar_config = PlanarConfiguration::Planar;
        assert_eq!(info.output_row_stride(0).unwrap(), 8);
        assert_eq!(info.output_row_stride(1).unwrap(), 6);
    }

    #[test]
    fn test_output_rows() {
        let mut info = CHOPTS;
        info.samples_per_pixel = 2;
        assert_eq!(info.output_rows(0).unwrap(), 4);
        assert_eq!(info.output_rows(1).unwrap(), 3);
        info.output_rows(2).unwrap_err();
        info.planar_config = PlanarConfiguration::Planar;
        assert_eq!(info.output_rows(0).unwrap(), 8);
        assert_eq!(info.output_rows(1).unwrap(), 6);
    }

    // #[rustfmt::skip]
    // #[test]
    // fn test_no_predict() {
    //     let cases = [
    //         (0,0, Bytes::from_static(&RES[..]),           Bytes::from_static(&RES[..])          ),
    //         (0,1, Bytes::from_static(&RES_BOT[..]),       Bytes::from_static(&RES_BOT[..])      ),
    //         (1,0, Bytes::from_static(&RES_RIGHT[..]),     Bytes::from_static(&RES_RIGHT[..])    ),
    //         (1,1, Bytes::from_static(&RES_BOT_RIGHT[..]), Bytes::from_static(&RES_BOT_RIGHT[..]))
    //     ];
    //     for (x,y, input, expected) in cases {
    //         assert_eq!(fix_endianness(input, &CHOPTS, x, y).unwrap(), expected);
    //     }
    // }

    #[rustfmt::skip]
    #[test]
    fn test_hdiff_unpredict() {
        let mut predictor_info = CHOPTS;
        let cases = [
            (0,0, vec![
                0i32, 1, 1, 1,
                1,-1, 1, 1,
                2,-1,-1, 1,
                3,-1,-1,-1,
            ], Vec::from(&RES[..])),
            (0,1, vec![
                0, 1, 1, 1,
                1,-1, 1, 1,
                2,-1,-1, 1,
            ], Vec::from(&RES_BOT[..])),
            (1,0, vec![
                0, 1, 1,
                1,-1, 1,
                2,-1,-1,
                3,-1,-1,
            ], Vec::from(&RES_RIGHT[..])),
            (1,1, vec![
                0, 1, 1,
                1,-1, 1,
                2,-1,-1,
            ], Vec::from(&RES_BOT_RIGHT[..])),
        ];
        for (x,_y, input, expected) in cases {
            println!("uints littleendian");
            predictor_info.byte_order = ByteOrder::LittleEndian;
            predictor_info.bits_per_sample = 8;
            assert_eq!(-1i32 as u8, 255);
            println!("testing u8");
            let mut buffer= input.iter().map(|v| *v as u8).collect::<Vec<_>>();
            let res = expected.clone();
            unpredict_hdiff(&mut buffer, &predictor_info, x).unwrap();
            assert_eq!(buffer, res);
            assert_eq!(-1i32 as u16, u16::MAX);
            println!("testing u16");
            predictor_info.bits_per_sample = 16;
            let mut buffer= input.iter().flat_map(|v| (*v as u16).to_le_bytes()).collect::<Vec<_>>();
            let res = expected.iter().flat_map(|v| (*v as u16).to_ne_bytes()).collect::<Vec<_>>();
             unpredict_hdiff(&mut buffer, &predictor_info, x).unwrap();
            assert_eq!(buffer, res);
            assert_eq!(-1i32 as u32, u32::MAX);
            println!("testing u32");
            predictor_info.bits_per_sample = 32;
            let mut buffer= input.iter().flat_map(|v| (*v as u32).to_le_bytes()).collect::<Vec<_>>();
            let res = expected.iter().flat_map(|v| (*v as u32).to_ne_bytes()).collect::<Vec<_>>();
             unpredict_hdiff(&mut buffer, &predictor_info, x).unwrap();
            assert_eq!(buffer, res);
            assert_eq!(-1i32 as u64, u64::MAX);
            println!("testing u64");
            predictor_info.bits_per_sample = 64;
            let mut buffer= input.iter().flat_map(|v| (*v as u64).to_le_bytes()).collect::<Vec<_>>();
            let res = expected.iter().flat_map(|v| (*v as u64).to_ne_bytes()).collect::<Vec<_>>();
             unpredict_hdiff(&mut buffer, &predictor_info, x).unwrap();
            assert_eq!(buffer, res);

            println!("ints littleendian");
            predictor_info.bits_per_sample = 8;
            println!("testing i8");
            let mut buffer= input.iter().flat_map(|v| (*v as i8).to_le_bytes()).collect::<Vec<_>>();
            println!("{:?}", &buffer[..]);
            let res = expected.clone();
             unpredict_hdiff(&mut buffer, &predictor_info, x).unwrap();
            assert_eq!(buffer, res);
            println!("testing i16");
            predictor_info.bits_per_sample = 16;
            let mut buffer= input.iter().flat_map(|v| (*v as i16).to_le_bytes()).collect::<Vec<_>>();
            let res = expected.iter().flat_map(|v| (*v as i16).to_ne_bytes()).collect::<Vec<_>>();
             unpredict_hdiff(&mut buffer, &predictor_info, x).unwrap();
            assert_eq!(buffer, res);
            println!("testing i32");
            predictor_info.bits_per_sample = 32;
            let mut buffer= input.iter().flat_map(|v| v.to_le_bytes()).collect::<Vec<_>>();
            let res = expected.iter().flat_map(|v| (*v as i32).to_ne_bytes()).collect::<Vec<_>>();
             unpredict_hdiff(&mut buffer, &predictor_info, x).unwrap();
            assert_eq!(buffer, res);
            println!("testing i64");
            predictor_info.bits_per_sample = 64;
            let mut buffer= input.iter().flat_map(|v| (*v as i64).to_le_bytes()).collect::<Vec<_>>();
            let res = expected.iter().flat_map(|v| (*v as i64).to_ne_bytes()).collect::<Vec<_>>()   ;
             unpredict_hdiff(&mut buffer, &predictor_info, x).unwrap();
            assert_eq!(buffer, res);

            println!("uints bigendian");
            predictor_info.byte_order = ByteOrder::BigEndian;
            predictor_info.bits_per_sample = 8;
            assert_eq!(-1i32 as u8, 255);
            println!("testing u8");
            let mut buffer= input.iter().map(|v| *v as u8).collect::<Vec<_>>();
            let res = expected.clone();
             unpredict_hdiff(&mut buffer, &predictor_info, x).unwrap();
            assert_eq!(buffer, res);
            assert_eq!(-1i32 as u16, u16::MAX);
            println!("testing u16");
            predictor_info.bits_per_sample = 16;
            let mut buffer= input.iter().flat_map(|v| (*v as u16).to_be_bytes()).collect::<Vec<_>>();
            let res = expected.iter().flat_map(|v| (*v as u16).to_ne_bytes()).collect::<Vec<_>>();
            println!("buffer: {:?}", &buffer[..]);
             unpredict_hdiff(&mut buffer, &predictor_info, x).unwrap();
            assert_eq!(buffer, res);
            assert_eq!(-1i32 as u32, u32::MAX);
            println!("testing u32");
            predictor_info.bits_per_sample = 32;
            let mut buffer= input.iter().flat_map(|v| (*v as u32).to_be_bytes()).collect::<Vec<_>>();
            let res = expected.iter().flat_map(|v| (*v as u32).to_ne_bytes()).collect::<Vec<_>>();
             unpredict_hdiff(&mut buffer, &predictor_info, x).unwrap();
            assert_eq!(buffer, res);
            assert_eq!(-1i32 as u64, u64::MAX);
            println!("testing u64");
            predictor_info.bits_per_sample = 64;
            let mut buffer= input.iter().flat_map(|v| (*v as u64).to_be_bytes()).collect::<Vec<_>>();
            let res = expected.iter().flat_map(|v| (*v as u64).to_ne_bytes()).collect::<Vec<_>>();
             unpredict_hdiff(&mut buffer, &predictor_info, x).unwrap();
            assert_eq!(buffer, res);

            println!("ints bigendian");
            predictor_info.bits_per_sample = 8;
            assert_eq!(-1i32 as u8, 255);
            println!("testing i8");
            let mut buffer= input.iter().flat_map(|v| (*v as i8).to_be_bytes()).collect::<Vec<_>>();
            let res = expected.clone();
            unpredict_hdiff(&mut buffer, &predictor_info, x).unwrap();
            assert_eq!(buffer, res);
            assert_eq!(-1i32 as u16, u16::MAX);
            println!("testing i16");
            predictor_info.bits_per_sample = 16;
            let mut buffer= input.iter().flat_map(|v| (*v as i16).to_be_bytes()).collect::<Vec<_>>();
            let res = expected.iter().flat_map(|v| (*v as i16).to_ne_bytes()).collect::<Vec<_>>();
             unpredict_hdiff(&mut buffer, &predictor_info, x).unwrap();
            assert_eq!(buffer, res);
            assert_eq!(-1i32 as u32, u32::MAX);
            println!("testing i32");
            predictor_info.bits_per_sample = 32;
            let mut buffer= input.iter().flat_map(|v| v.to_be_bytes()).collect::<Vec<_>>();
            let res = expected.iter().flat_map(|v| (*v as i32).to_ne_bytes()).collect::<Vec<_>>();
             unpredict_hdiff(&mut buffer, &predictor_info, x).unwrap();
            assert_eq!(buffer, res);
            assert_eq!(-1i32 as u64, u64::MAX);
            println!("testing i64");
            predictor_info.bits_per_sample = 64;
            let mut buffer= input.iter().flat_map(|v| (*v as i64).to_be_bytes()).collect::<Vec<_>>();
            let res = expected.iter().flat_map(|v| (*v as i64).to_ne_bytes()).collect::<Vec<_>>();
             unpredict_hdiff(&mut buffer, &predictor_info, x).unwrap();
            assert_eq!(buffer, res);
        }
    }

    #[rustfmt::skip]
    #[test]
    fn test_predict_f16() {
        // take a 4-value image
        let expect_le = [1,0,3,2,5,4,7,6u8];
        let _expected = [0,1,2,3,4,5,6,7u8];
        //                              0       1
        //                            0       1
        //                          0       1
        //                        0       1
        let _shuffled = [0,2,4,6,1,3,5,7u8];
        let diffed = [0,2,2,2,251,2,2,2];
        let info = ChunkOpts {
            byte_order: ByteOrder::LittleEndian,
            image_width: 4+4,
            image_height: 4+1,
            chunk_width: 4,
            chunk_height: 4,
            bits_per_sample: 16,
            samples_per_pixel: 1,
            planar_config: PlanarConfiguration::Chunky,
            // bs
            sample_format: SampleFormat::Void,
            photometric_interpretation: PhotometricInterpretation::BlackIsZero,
            compression_method: CompressionMethod::Deflate,
            predictor: Predictor::FloatingPoint,
            jpeg_tables: None,
        };
        let mut input = diffed.to_vec();
        let mut output = vec![0;diffed.len()];
        unpredict_float(
            &mut input,
            &mut output,
            &info,
            1,
            1,
        )
        .unwrap();
        assert_eq!(
            &output,
            &expect_le
        )
    }

    #[rustfmt::skip]
    #[test]
    fn test_predict_f16_padding() {
        // take a 4-pixel image with 2 padding pixels
        let expect_le = [1,0,3,2u8]; // no padding
        let _expected = [0,1,2,3,0,0,0,0u8]; //padding added
        //                              0       1
        //                            0       1
        //                          0       1
        //                        0       1
        let _shuffled = [0,2,0,0,1,3,0,0u8];
        let diffed = [0,2,254,0,1,2,253,0];
        let info = ChunkOpts {
            byte_order: ByteOrder::LittleEndian,
            image_width: 4+2,
            image_height: 4+1,
            chunk_width: 4,
            chunk_height: 4,
            bits_per_sample: 16,
            samples_per_pixel: 1,
            planar_config: PlanarConfiguration::Chunky,
            // bs
            sample_format: SampleFormat::Void,
            photometric_interpretation: PhotometricInterpretation::BlackIsZero,
            compression_method: CompressionMethod::Deflate,
            predictor: Predictor::FloatingPoint,
            jpeg_tables: None,
        };
        let mut input = diffed.to_vec();
        let mut output = vec![0;expect_le.len()];
        unpredict_float(
            &mut input,
            &mut output,
            &info,
            1,
            1,
        )
        .unwrap();
        assert_eq!(
            &output,
            &expect_le
        )
    }

    #[rustfmt::skip]
    #[test]
    fn test_fpredict_f32() {
        // let's take this 2-value image where we only look at bytes
        let expect_le  = [3,2,  1,0,  7,6,  5,4];
        let _expected  = [0,1,  2,3,  4,5,  6,7u8];
        //                  0     1     2     3   \_ de-shuffling indices
        //                0     1     2     3     /  (the one the function uses)
        let _shuffled  = [0,4,  1,5,  2,6,  3,7u8];
        let diffed     = [0,4,253,4,253,4,253,4u8];
        println!("expected: {expect_le:?}");
        let info = ChunkOpts {
            byte_order: ByteOrder::LittleEndian,
            image_width: 2,
            image_height: 2 + 1,
            chunk_width: 2,
            chunk_height: 2,
            bits_per_sample: 32,
            samples_per_pixel: 1,
            planar_config: PlanarConfiguration::Chunky,
            // bs
            sample_format: SampleFormat::Void,
            photometric_interpretation: PhotometricInterpretation::BlackIsZero,
            compression_method: CompressionMethod::Deflate,
            predictor: Predictor::FloatingPoint,
            jpeg_tables: None,
        };
        let mut input = diffed.to_vec();
        let mut output = vec![0;diffed.len()];
        unpredict_float(
            &mut input,
            &mut output,
            &info,
            0,
            0,
        )
        .unwrap();
        assert_eq!(
            &output,
            &expect_le
        )
    }

    #[test]
    fn test_fpredict_f64() {
        assert_eq!(
            f64::from_le_bytes([7, 6, 5, 4, 3, 2, 1, 0]),
            f64::from_bits(0x00_01_02_03_04_05_06_07)
        );
        // let's take this 2-value image
        let expect_be = [7, 6, 5, 4, 3, 2, 1, 0, 15, 14, 13, 12, 11, 10, 9, 8];
        let _expected = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15u8];
        //                           0   1    2    3    4     5     6     7
        //                         0   1   2    3    4     5     6     7
        let _shuffled = [0, 8, 1, 9, 2, 10, 3, 11, 4, 12, 5, 13, 6, 14, 7, 15u8];
        let diffed = [
            0, 8, 249, 8, 249, 8, 249, 8, 249, 8, 249, 8, 249, 8, 249, 8u8,
        ];
        let info = ChunkOpts {
            byte_order: ByteOrder::LittleEndian,
            image_width: 2,
            image_height: 2 + 1,
            chunk_width: 2,
            chunk_height: 2,
            bits_per_sample: 64,
            samples_per_pixel: 1,
            planar_config: PlanarConfiguration::Chunky,
            // bs
            sample_format: SampleFormat::Void,
            photometric_interpretation: PhotometricInterpretation::BlackIsZero,
            compression_method: CompressionMethod::Deflate,
            predictor: Predictor::FloatingPoint,
            jpeg_tables: None,
        };
        let mut input = diffed.to_vec();
        let mut output = vec![0; diffed.len()];
        unpredict_float(&mut input, &mut output, &info, 0, 0).unwrap();
        assert_eq!(&output, &expect_be);
    }
}
