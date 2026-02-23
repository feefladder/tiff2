use exn::bail;

use crate::{
    error::{TiffError, TiffResult, TiffUnsupportedError},
    loader::tile::ChunkOpts,
};

pub(crate) fn predict_hdiff(
    buffer: &mut [u8],
    predictor_info: &ChunkOpts,
    tile_: u32,
) -> TiffResult<()> {
    todo!()
}

pub(crate) fn predict_float(
    in_buf: &mut [u8],
    out_buf: &mut [u8],
    predictor_info: &ChunkOpts,
    x: u32,
    y: u32,
) -> TiffResult<()> {
    // Ok, so the names "input" and "output" break down here,
    // for a floating-point predictor, there is the "image" or [RGBRGBRGB]
    // and the predicted part, where floating-point bits are merged together with padding
    // I'm not sure, but I think padding bits should be set to 0?
    let output_row_stride = predictor_info.input_row_stride(x)?;
    let bit_depth = predictor_info.bits_per_sample;
    if predictor_info.chunk_width_pixels(x)? == predictor_info.chunk_width {
        // no special padding handling
        for (input, output) in in_buf
            .chunks_exact_mut(output_row_stride)
            .zip(out_buf.chunks_exact_mut(output_row_stride))
        {
            match bit_depth {
                16 => predict_f16(input, output, predictor_info.samples_per_pixel as _),
                32 => predict_f32(input, output, predictor_info.samples_per_pixel as _),
                64 => predict_f64(input, output, predictor_info.samples_per_pixel as _),
                depth => {
                    return Err(TiffError::UnsupportedError(
                        TiffUnsupportedError::UnsupportedSampleDepth(depth),
                    ))
                }
            }
        }
    } else {
        let input_row_stride = predictor_info.output_row_stride(x)?;

        let mut out_row = vec![0u8; output_row_stride];
        for (input, output) in in_buf
            .chunks_exact_mut(input_row_stride)
            .zip(out_buf.chunks_exact_mut(output_row_stride))
        {
            out_row.fill(0);
            out_row[..input_row_stride].copy_from_slice(input);
            match bit_depth {
                16 => predict_f16(input, &mut out_row, predictor_info.samples_per_pixel as _),
                32 => predict_f32(input, &mut out_row, predictor_info.samples_per_pixel as _),
                64 => predict_f64(input, &mut out_row, predictor_info.samples_per_pixel as _),
                depth => {
                    return Err(TiffError::UnsupportedError(
                        TiffUnsupportedError::UnsupportedSampleDepth(depth),
                    ))
                }
            }
        }
    }
    Ok(())
}

fn predict_f16(input: &mut [u8], output: &mut [u8], samples_per_pixel: usize) {
    todo!()
}

fn predict_f32(input: &[u8], output: &mut [u8], samples: usize) {
    let n_samp = input.len() / 4;
    for (i, chunk) in input.chunks_exact(4).enumerate() {
        output[i + 0 * n_samp] = chunk[0];
        output[i + 1 * n_samp] = chunk[1];
        output[i + 2 * n_samp] = chunk[2];
        output[i + 3 * n_samp] = chunk[3];
    }
    for i in (samples..input.len()).rev() {
        output[i] = output[i].wrapping_sub(output[i - samples]);
    }
}

fn predict_f64(input: &mut [u8], output: &mut [u8], samples: usize) {
    let n_samp = input.len() / 8;
    for (i, chunk) in input.chunks_exact(8).enumerate() {
        output[i + 0 * n_samp] = chunk[0];
        output[i + 1 * n_samp] = chunk[1];
        output[i + 2 * n_samp] = chunk[2];
        output[i + 3 * n_samp] = chunk[3];
        output[i + 4 * n_samp] = chunk[4];
        output[i + 5 * n_samp] = chunk[5];
        output[i + 6 * n_samp] = chunk[6];
        output[i + 7 * n_samp] = chunk[7];
    }
    for i in (samples..input.len()).rev() {
        output[i] = output[i].wrapping_sub(output[i - samples]);
    }
}

#[cfg(test)]
mod test {
    use std::process::Output;
    use std::vec;

    use super::*;
    use crate::loader::{unpredict_f32, unpredict_f64};
    use crate::structs::tags::{
        CompressionMethod, PhotometricInterpretation, PlanarConfiguration, Predictor, SampleFormat,
    };
    use crate::util::fix_endianness;
    use crate::{ByteOrder, NATIVE_ENDIAN};

    fn shuffle(input: &[u8], output: &mut [u8]) {
        assert_eq!(input.len(), output.len());
        for (i, chunk) in input.chunks_exact(4).enumerate() {
            output[i] = chunk[0];
            output[i + 1 * input.len() / 4] = chunk[1];
            output[i + 2 * input.len() / 4] = chunk[2];
            output[i + 3 * input.len() / 4] = chunk[3];
        }
    }
    // copied from unpredict_f32
    fn unshuffle(input: &[u8], output: &mut [u8]) {
        for (i, chunk) in output.chunks_exact_mut(4).enumerate() {
            chunk.copy_from_slice(
                // convert to be-like
                &u32::to_ne_bytes(u32::from_be_bytes([
                    input[i],
                    input[i + 1 * input.len() / 4],
                    input[i + 2 * input.len() / 4],
                    input[i + 3 * input.len() / 4],
                ])),
            );
        }
    }

    #[test]
    fn test_roundtrip_shuffling_32_1() {
        let mut input = vec![0, 1, 2, 3];
        let mut output = vec![0u8; input.len()];
        // copied from predict_f32

        let input_le = [3, 2, 1, 0];
        shuffle(&input, &mut output);
        // so this is the endianness conversion

        let shuffled = [3, 2, 1, 0];
        // but we get
        // [6,4,2,0,7,5,3,1]
        // so this is the required input to make unshuffle happy
        assert_eq!(&output, &shuffled);
        //
        // unshuffle(&[0, 4, 1, 5, 2, 6, 3, 7], &mut input);
        unshuffle(&shuffled, &mut input);
        assert_eq!(&input, &[0, 1, 2, 3]);
    }

    #[test]
    fn test_roundtrip_shuffling_32_2() {
        let mut input = vec![0, 1, 2, 3, 4, 5, 6, 7u8];
        //                            0  4  1  5  2  6  3  7
        //
        let mut output = vec![0u8; input.len()];
        // copied from predict_f32

        // so this is the endianness conversion
        let mut input_le = input.clone();
        fix_endianness(&mut input_le, ByteOrder::BigEndian, NATIVE_ENDIAN, 32);
        shuffle(&input_le, &mut output);
        assert_eq!(input_le, [3, 2, 1, 0, 7, 6, 5, 4]);
        //                      numbers correspond to indices
        // let shuffled = [0, 4, 1, 5, 2, 6, 3, 7];
        let shuffled = [3, 7, 2, 6, 1, 5, 0, 4];
        // but we get
        // [6,4,2,0,7,5,3,1]
        // so this is the required input to make unshuffle happy
        println!("{input:?}");
        assert_eq!(&output, &shuffled);
        // unshuffle(&[0, 4, 1, 5, 2, 6, 3, 7], &mut input);
        unshuffle(&shuffled, &mut input);
        assert_eq!(&input, &[0, 1, 2, 3, 4, 5, 6, 7]);
    }

    #[test]
    fn test_roundtrip_shuffling_32_3() {
        let mut input = vec![0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11];
        //                            0  4  8  1  5  9  2  6 10  3   7  11
        //
        let mut output = vec![0u8; input.len()];
        // copied from predict_f32

        // so this is the endianness conversion
        let mut input_le = input.clone();
        fix_endianness(&mut input_le, ByteOrder::BigEndian, NATIVE_ENDIAN, 32);
        shuffle(&input, &mut output);
        assert_eq!(input_le, [3, 2, 1, 0, 7, 6, 5, 4]);
        //                      numbers correspond to indices
        // let shuffled = [0, 4, 1, 5, 2, 6, 3, 7];
        let shuffled = [3, 7, 2, 6, 1, 5, 0, 4];
        // but we get
        // [6,4,2,0,7,5,3,1]
        // so this is the required input to make unshuffle happy
        assert_eq!(&output, &shuffled);
        // unshuffle(&[0, 4, 1, 5, 2, 6, 3, 7], &mut input);
        unshuffle(&shuffled, &mut input);
        assert_eq!(&input, &input);
    }

    #[test]
    fn test_predict_f32() {
        let mut input = [0, 1, 2, 3, 4, 5, 6, 7];
        fix_endianness(&mut input, NATIVE_ENDIAN, ByteOrder::BigEndian, 32);
        let mut output = vec![0; input.len()];
        predict_f32(&mut input, &mut output, 1);
        // copied from unpredict
        let diffed = [0, 4, 253, 4, 253, 4, 253, 4u8];
    }

    #[test]
    fn test_roundtrip_predict_f32_42() {
        let mut floats = vec![42f32; 42];
        let og = bytemuck::cast_slice(&floats).to_vec();
        let spp = 1;
        let mut predicted = vec![0; og.len()];
        fix_endianness(
            bytemuck::cast_slice_mut(&mut floats),
            NATIVE_ENDIAN,
            ByteOrder::BigEndian,
            32,
        );
        predict_f32(bytemuck::cast_slice_mut(&mut floats), &mut predicted, spp);
        let mut unpredicted = vec![0; og.len()];
        unpredict_f32(&mut predicted, &mut unpredicted, spp);
        assert_eq!(unpredicted, og)
    }

    #[test]
    fn test_roundtrip_predict_f64() {
        let mut floats = vec![42f64; 42];
        let og = bytemuck::cast_slice(&floats).to_vec();
        let spp = 1;
        let mut predicted = vec![0; og.len()];
        fix_endianness(
            bytemuck::cast_slice_mut(&mut floats),
            NATIVE_ENDIAN,
            ByteOrder::BigEndian,
            64,
        );
        predict_f64(bytemuck::cast_slice_mut(&mut floats), &mut predicted, spp);
        let mut unpredicted = vec![0; og.len()];
        unpredict_f64(&mut predicted, &mut unpredicted, spp);
        assert_eq!(unpredicted, og)
    }

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

    #[rustfmt::skip]
    #[test]
    fn test_hdiff_predict() {
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
        for (x,_y, expected, input) in cases {
            println!("uints littleendian");
            predictor_info.byte_order = ByteOrder::LittleEndian;
            predictor_info.bits_per_sample = 8;
            assert_eq!(-1i32 as u8, 255);
            println!("testing u8");
            let mut buffer= input.iter().map(|v| *v as u8).collect::<Vec<_>>();
            let res = expected.iter().map(|v| *v as u8).collect::<Vec<_>>();
            predict_hdiff(&mut buffer, &predictor_info, x).unwrap();
            assert_eq!(buffer, res);
            assert_eq!(-1i32 as u16, u16::MAX);
            println!("testing u16");
            predictor_info.bits_per_sample = 16;
            let mut buffer= input.iter().flat_map(|v| (*v as u16).to_le_bytes()).collect::<Vec<_>>();
            let res = expected.iter().flat_map(|v| (*v as u16).to_ne_bytes()).collect::<Vec<_>>();
            predict_hdiff(&mut buffer, &predictor_info, x).unwrap();
            assert_eq!(buffer, res);
            assert_eq!(-1i32 as u32, u32::MAX);
            println!("testing u32");
            predictor_info.bits_per_sample = 32;
            let mut buffer= input.iter().flat_map(|v| (*v as u32).to_le_bytes()).collect::<Vec<_>>();
            let res = expected.iter().flat_map(|v| (*v as u32).to_ne_bytes()).collect::<Vec<_>>();
             predict_hdiff(&mut buffer, &predictor_info, x).unwrap();
            assert_eq!(buffer, res);
            assert_eq!(-1i32 as u64, u64::MAX);
            println!("testing u64");
            predictor_info.bits_per_sample = 64;
            let mut buffer= input.iter().flat_map(|v| (*v as u64).to_le_bytes()).collect::<Vec<_>>();
            let res = expected.iter().flat_map(|v| (*v as u64).to_ne_bytes()).collect::<Vec<_>>();
             predict_hdiff(&mut buffer, &predictor_info, x).unwrap();
            assert_eq!(buffer, res);

            println!("ints littleendian");
            predictor_info.bits_per_sample = 8;
            println!("testing i8");
            let mut buffer= input.iter().flat_map(|v| (*v as i8).to_le_bytes()).collect::<Vec<_>>();
            println!("{:?}", &buffer[..]);
            let res = expected.iter().flat_map(|v| (*v as i8).to_le_bytes()).collect::<Vec<_>>();
             predict_hdiff(&mut buffer, &predictor_info, x).unwrap();
            assert_eq!(buffer, res);
            println!("testing i16");
            predictor_info.bits_per_sample = 16;
            let mut buffer= input.iter().flat_map(|v| (*v as i16).to_le_bytes()).collect::<Vec<_>>();
            let res = expected.iter().flat_map(|v| (*v as i16).to_ne_bytes()).collect::<Vec<_>>();
             predict_hdiff(&mut buffer, &predictor_info, x).unwrap();
            assert_eq!(buffer, res);
            println!("testing i32");
            predictor_info.bits_per_sample = 32;
            let mut buffer= input.iter().flat_map(|v| v.to_le_bytes()).collect::<Vec<_>>();
            let res = expected.iter().flat_map(|v| (*v as i32).to_ne_bytes()).collect::<Vec<_>>();
             predict_hdiff(&mut buffer, &predictor_info, x).unwrap();
            assert_eq!(buffer, res);
            println!("testing i64");
            predictor_info.bits_per_sample = 64;
            let mut buffer= input.iter().flat_map(|v| (*v as i64).to_le_bytes()).collect::<Vec<_>>();
            let res = expected.iter().flat_map(|v| (*v as i64).to_ne_bytes()).collect::<Vec<_>>()   ;
             predict_hdiff(&mut buffer, &predictor_info, x).unwrap();
            assert_eq!(buffer, res);

            println!("uints bigendian");
            predictor_info.byte_order = ByteOrder::BigEndian;
            predictor_info.bits_per_sample = 8;
            assert_eq!(-1i32 as u8, 255);
            println!("testing u8");
            let mut buffer= input.iter().map(|v| *v as u8).collect::<Vec<_>>();
            let res = expected.iter().map(|v| *v as u8).collect::<Vec<_>>();
             predict_hdiff(&mut buffer, &predictor_info, x).unwrap();
            assert_eq!(buffer, res);
            assert_eq!(-1i32 as u16, u16::MAX);
            println!("testing u16");
            predictor_info.bits_per_sample = 16;
            let mut buffer= input.iter().flat_map(|v| (*v as u16).to_be_bytes()).collect::<Vec<_>>();
            let res = expected.iter().flat_map(|v| (*v as u16).to_ne_bytes()).collect::<Vec<_>>();
            println!("buffer: {:?}", &buffer[..]);
             predict_hdiff(&mut buffer, &predictor_info, x).unwrap();
            assert_eq!(buffer, res);
            assert_eq!(-1i32 as u32, u32::MAX);
            println!("testing u32");
            predictor_info.bits_per_sample = 32;
            let mut buffer= input.iter().flat_map(|v| (*v as u32).to_be_bytes()).collect::<Vec<_>>();
            let res = expected.iter().flat_map(|v| (*v as u32).to_ne_bytes()).collect::<Vec<_>>();
             predict_hdiff(&mut buffer, &predictor_info, x).unwrap();
            assert_eq!(buffer, res);
            assert_eq!(-1i32 as u64, u64::MAX);
            println!("testing u64");
            predictor_info.bits_per_sample = 64;
            let mut buffer= input.iter().flat_map(|v| (*v as u64).to_be_bytes()).collect::<Vec<_>>();
            let res = expected.iter().flat_map(|v| (*v as u64).to_ne_bytes()).collect::<Vec<_>>();
             predict_hdiff(&mut buffer, &predictor_info, x).unwrap();
            assert_eq!(buffer, res);

            println!("ints bigendian");
            predictor_info.bits_per_sample = 8;
            assert_eq!(-1i32 as u8, 255);
            println!("testing i8");
            let mut buffer= input.iter().flat_map(|v| (*v as i8).to_be_bytes()).collect::<Vec<_>>();
            let res = expected.iter().flat_map(|v| (*v as i8).to_be_bytes()).collect::<Vec<_>>();
            predict_hdiff(&mut buffer, &predictor_info, x).unwrap();
            assert_eq!(buffer, res);
            assert_eq!(-1i32 as u16, u16::MAX);
            println!("testing i16");
            predictor_info.bits_per_sample = 16;
            let mut buffer= input.iter().flat_map(|v| (*v as i16).to_be_bytes()).collect::<Vec<_>>();
            let res = expected.iter().flat_map(|v| (*v as i16).to_ne_bytes()).collect::<Vec<_>>();
             predict_hdiff(&mut buffer, &predictor_info, x).unwrap();
            assert_eq!(buffer, res);
            assert_eq!(-1i32 as u32, u32::MAX);
            println!("testing i32");
            predictor_info.bits_per_sample = 32;
            let mut buffer= input.iter().flat_map(|v| v.to_be_bytes()).collect::<Vec<_>>();
            let res = expected.iter().flat_map(|v| (*v as i32).to_ne_bytes()).collect::<Vec<_>>();
             predict_hdiff(&mut buffer, &predictor_info, x).unwrap();
            assert_eq!(buffer, res);
            assert_eq!(-1i32 as u64, u64::MAX);
            println!("testing i64");
            predictor_info.bits_per_sample = 64;
            let mut buffer= input.iter().flat_map(|v| (*v as i64).to_be_bytes()).collect::<Vec<_>>();
            let res = expected.iter().flat_map(|v| (*v as i64).to_ne_bytes()).collect::<Vec<_>>();
             predict_hdiff(&mut buffer, &predictor_info, x).unwrap();
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
        predict_float(
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
        predict_float(
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
        predict_float(
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
        predict_float(&mut input, &mut output, &info, 0, 0).unwrap();
        assert_eq!(&output, &expect_be);
    }
}
