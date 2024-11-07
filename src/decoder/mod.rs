use std::usize;

use crate::{
    structs::tags::{Predictor, SampleFormat},
    util::fix_endianness,
    ByteOrder, ColorType,
};

mod reader;
pub use reader::{CogReader, EndianReader};
mod chunk_decoder;
pub use chunk_decoder::ChunkDecoder;
mod decoder;
pub use decoder::Decoder;

/// reverse horizontal prediction
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

/// Reverse floating point prediction
///
/// floating point prediction first shuffles the bytes and then uses horizontal differencing
/// also performs byte-order conversion if needed.
///
/// ```
/// let samples = 1;
/// let input = [42.0f32, 43.0];
/// let in_be_bytes: Vec<u8> = input.iter().flat_map(|f| f.to_be_bytes()).collect();
/// assert_eq!(vec![0x42,0x28,0,0,0x42,0x2c,0,0],in_be_bytes);
/// //               0   1   2 3   4  5   6 7-- OG
/// //               0   2         1  3
/// //                       0 2          1 3
/// //             --0   4   1 5   2  6   3 7-- shuffled
/// //               0       1     2      3
/// //                   0     1      2     3
/// //               0   1   2 3   4  5   6 7-- deshuffled
/// let mut in_shuffled = vec![0x42,0x42,0x28,0x2c,0,0,0,0];
/// let n_floats = in_be_bytes.len() / 4;
/// for (i, chunk) in in_shuffled.chunks_mut(n_floats).enumerate() {
///     for (j, c) in chunk.iter_mut().enumerate() {
///         *c = in_be_bytes[j * 4 + i]
///     }
/// }
/// println!("shuffled: {in_shuffled:?}");
/// assert_eq!(vec![0x42,0x42,0x28,0x2c,0,0,0,0], in_shuffled);
/// let mut in_diffed = vec![0x42u8,0,230,4,212,0,0,0];
/// // let's see if we can do horizontal differencing without an extra copy...
/// let prev = in_diffed[..samples].clone();
/// // [1 2 3 4 5 6 7 8] samples = 2
/// // [1 2]| |
/// //   \--2 |          i=0; i=-2%2=0; in[i].wrapping_sub(prev[i%samples])
/// //   [3 2]|          prev[i%samples]=in[i]
/// //      \-2          i=1; i-samples%samples=-1%2=1;
/// // this seems horribly inefficient...
/// // maybe with a double-sized buffer...
/// // [0 1 2 3 4 5 6 7] samples = 2; let d_samp = samples*2=4;
/// // [0 1 2 3]         prev = in[..dsamp].clone();
/// //   \-[2]3 4 5 6 7  i=0; in[i]=2; in[i] = in[i].wrapping_sub(prev[i%dsamp])
/// // [4 1 2 3]         prev[(i+samples) % dsamp] = in[i+samples]
/// //     \2[2]4 5 6 7  i=1; in[i] = 3-1 = 2
/// // [4(5)2 3]         prev[1%4]=prev[1]=in[1+2]=in[3]=3
/// //      2\2[2]5 6 7  i=2; in[i]=  4-2 = 2
/// // [4 5 6 3]         prev[2%4]=prev[2]=in[2+2]=in[4]=4
/// //      2 2\2[2]6 7  i=3; in[i]=  5-3 = 2
/// //         [4 5 6 7] prev[3%4]=prev[3]=in[3+2]=in[5]=5
/// //      -------------remainder
/// //      2 2 2 2[2]7
/// //      2 2 2 2 2[2]
/// for i in samples..in_diffed.len() {
///     let p = in_diffed[i];
///     in_diffed[i] = in_shuffled[i].wrapping_sub(prev[i%samples]);
///     prev[i%samples] = p
/// }
/// println!("diffed: {in_diffed:?}");
/// assert_eq!(vec![0x42u8,0,230,4,212,0,0,0], in_diffed);
/// //assert_eq!(vec![42,42,0x28,0x2c,0,0,0,0], in_shuffled);
/// //              0  1  2    3   4 5 6 7 len=8 8/4=2
/// //              0     1        2   3   : 0 2 4 6
/// //                 0       1     2   3 : 1 3 5 7
/// let mut output = vec![0;8];
/// for (i, chunk) in output.chunks_mut(4).enumerate() {
///     chunk.copy_from_slice(&u32::to_be_bytes( // don't convert to native-endian
///         // preserve original byte-order
///         u32::from_be_bytes([
///             in_shuffled[i],
///             in_shuffled[in_shuffled.len() / 4 + i],
///             in_shuffled[in_shuffled.len() / 4 * 2 + i],
///             in_shuffled[in_shuffled.len() / 4 * 3 + i],
///         ])));
/// }
/// assert_eq!(output, in_be_bytes);
/// let mut in_diffed = vec![0x42u8,0,230,4,212,0,0,0];
/// for i in samples..in_diffed.len() {
///   in_diffed[i] = in_diffed[i].wrapping_add(in_diffed[i-samples]);
/// }
/// assert_eq!(in_diffed, in_shuffled);
/// ```
pub fn predict_f32(input: &mut [u8], output: &mut [u8], samples: usize) {
    // reverse horizontal differencing
    for i in samples..input.len() {
        input[i] = input[i].wrapping_add(input[i - samples]);
    }
    // reverse byte shuffle and fix endianness
    for (i, chunk) in output.chunks_mut(4).enumerate() {
        chunk.copy_from_slice(&u32::to_ne_bytes(
            // convert to native-endian
            // preserve original byte-order
            u32::from_be_bytes([
                input[i],
                input[input.len() / 4 + i],
                input[input.len() / 4 * 2 + i],
                input[input.len() / 4 * 3 + i],
            ]),
        ));
    }
}

fn predict_f64(input: &mut [u8], output: &mut [u8], samples: usize) {
    for i in samples..input.len() {
        input[i] = input[i].wrapping_add(input[i - samples]);
    }

    for (i, chunk) in output.chunks_mut(8).enumerate() {
        chunk.copy_from_slice(&u64::to_ne_bytes(u64::from_be_bytes([
            input[i],
            input[input.len() / 8 + i],
            input[input.len() / 8 * 2 + i],
            input[input.len() / 8 * 3 + i],
            input[input.len() / 8 * 4 + i],
            input[input.len() / 8 * 5 + i],
            input[input.len() / 8 * 6 + i],
            input[input.len() / 8 * 7 + i],
        ])));
    }
}

fn fix_endianness_and_predict(
    buf: &mut [u8],
    bit_depth: u8,
    samples: usize,
    byte_order: ByteOrder,
    predictor: Predictor,
) {
    match predictor {
        Predictor::None => {
            fix_endianness(buf, byte_order, bit_depth);
        }
        Predictor::Horizontal => {
            fix_endianness(buf, byte_order, bit_depth);
            rev_hpredict_nsamp(buf, bit_depth, samples);
        }
        Predictor::FloatingPoint => {
            let mut buffer_copy = buf.to_vec();
            match bit_depth {
                32 => predict_f32(&mut buffer_copy, buf, samples),
                64 => predict_f64(&mut buffer_copy, buf, samples),
                _ => unreachable!("Caller should have validated arguments. Please file a bug."),
            }
        }
    }
}

fn invert_colors(buf: &mut [u8], color_type: ColorType, sample_format: SampleFormat) {
    match (color_type, sample_format) {
        (ColorType::Gray(8), SampleFormat::Uint) => {
            for x in buf {
                *x = 0xff - *x;
            }
        }
        (ColorType::Gray(16), SampleFormat::Uint) => {
            for x in buf.chunks_mut(2) {
                let v = u16::from_ne_bytes(x.try_into().unwrap());
                x.copy_from_slice(&(0xffff - v).to_ne_bytes());
            }
        }
        (ColorType::Gray(32), SampleFormat::Uint) => {
            for x in buf.chunks_mut(4) {
                let v = u32::from_ne_bytes(x.try_into().unwrap());
                x.copy_from_slice(&(0xffff_ffff - v).to_ne_bytes());
            }
        }
        (ColorType::Gray(64), SampleFormat::Uint) => {
            for x in buf.chunks_mut(8) {
                let v = u64::from_ne_bytes(x.try_into().unwrap());
                x.copy_from_slice(&(0xffff_ffff_ffff_ffff - v).to_ne_bytes());
            }
        }
        (ColorType::Gray(32), SampleFormat::IEEEFP) => {
            for x in buf.chunks_mut(4) {
                let v = f32::from_ne_bytes(x.try_into().unwrap());
                x.copy_from_slice(&(1.0 - v).to_ne_bytes());
            }
        }
        (ColorType::Gray(64), SampleFormat::IEEEFP) => {
            for x in buf.chunks_mut(8) {
                let v = f64::from_ne_bytes(x.try_into().unwrap());
                x.copy_from_slice(&(1.0 - v).to_ne_bytes());
            }
        }
        _ => {}
    }
}

/// Decoding limits
#[derive(Clone, Debug, PartialEq)]
pub struct Limits {
    /// The maximum size of any `DecodingResult` in bytes, the default is
    /// 256MiB. If the entire image is decoded at once, then this will
    /// be the maximum size of the image. If it is decoded one strip at a
    /// time, this will be the maximum size of a strip.
    pub decoding_buffer_size: usize,
    /// The maximum size of any ifd value in bytes, the default is
    /// 1MiB.
    pub ifd_value_size: usize,
    /// Maximum size for intermediate buffer which may be used to limit the amount of data read per
    /// segment even if the entire image is decoded at once.
    pub intermediate_buffer_size: usize,
    /// The purpose of this is to prevent all the fields of the struct from
    /// being public, as this would make adding new fields a major version
    /// bump.
    _non_exhaustive: (),
}

impl Limits {
    /// A configuration that does not impose any limits.
    ///
    /// This is a good start if the caller only wants to impose selective limits, contrary to the
    /// default limits which allows selectively disabling limits.
    ///
    /// Note that this configuration is likely to crash on excessively large images since,
    /// naturally, the machine running the program does not have infinite memory.
    pub fn unlimited() -> Limits {
        Limits {
            decoding_buffer_size: usize::MAX,
            ifd_value_size: usize::MAX,
            intermediate_buffer_size: usize::MAX,
            _non_exhaustive: (),
        }
    }
}

impl Default for Limits {
    fn default() -> Limits {
        Limits {
            decoding_buffer_size: 256 * 1024 * 1024,
            intermediate_buffer_size: 128 * 1024 * 1024,
            ifd_value_size: 1024 * 1024,
            _non_exhaustive: (),
        }
    }
}
