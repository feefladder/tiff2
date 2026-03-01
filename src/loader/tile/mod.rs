use std::borrow::Cow;
use std::ops::Range;

use bytes::Bytes;
use exn::{OptionExt, ResultExt};
use rayon::iter::ParallelIterator;

use crate::loader::tile::predictor::{unpredict_float, unpredict_hdiff};
use crate::structs::error::CodingError;
use crate::structs::tags::Predictor;
use crate::structs::ChunkOpts;
use crate::structs::TileData;
use crate::util::fix_endianness;
use crate::NATIVE_ENDIAN;

pub mod error;
mod predictor;
mod registry;
pub use registry::DecoderRegistry;

type CodingResult<T> = exn::Result<T, CodingError>;

pub struct TileServer<'a> {
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
        coords: impl ParallelIterator<Item = (u32, u32)> + 'b,
    ) -> impl ParallelIterator<Item = (u32, u32, Range<u64>)> + 'b {
        coords.map(|(x, y)| (x, y, self.tile_range(x, y)))
    }

    pub fn get_tiles<'b>(
        &'b self,
        tile_datas: impl ParallelIterator<Item = (u32, u32, Bytes)> + 'b,
        decoder_registry: &'b DecoderRegistry,
    ) -> impl ParallelIterator<Item = (u32, u32, CodingResult<TileData>)> + 'b {
        tile_datas.map(|(x, y, buf)| {
            (
                x,
                y,
                Self::decode(x, y, self.chunk_opts.clone(), buf, decoder_registry),
            )
        })
    }

    pub fn decode(
        x: u32,
        y: u32,
        chunk_opts: ChunkOpts,
        compressed: Bytes,
        decoder_registry: &DecoderRegistry,
    ) -> exn::Result<TileData, CodingError> {
        // output size in number of samples
        // we don't support 16-bit pointer archs
        let output_size = usize::try_from(
            chunk_opts
                .chunk_width_pixels(x)
                .or_raise(|| CodingError::invalid_tile_index(x, y))?,
        )
        .unwrap()
        .saturating_mul(
            usize::try_from(
                chunk_opts
                    .chunk_height_pixels(y)
                    .or_raise(|| CodingError::invalid_tile_index(x, y))?,
            )
            .unwrap(),
        )
        .saturating_mul(chunk_opts.samples_per_pixel.into());
        let mut res = TileData::new(
            output_size,
            chunk_opts.dtype().or_raise(|| {
                CodingError::unsupported_bit_depth(
                    chunk_opts.bits_per_sample,
                    "could not get dtype",
                )
            })?,
        );
        Self::decode_into(x, y, chunk_opts, compressed, res.as_mut(), decoder_registry)?;
        Ok(res)
    }

    pub fn decode_into(
        x: u32,
        y: u32,
        chunk_opts: ChunkOpts,
        compressed: Bytes,
        out_buf: &mut [u8],
        decoder_registry: &DecoderRegistry,
    ) -> exn::Result<(), CodingError> {
        let decoder = decoder_registry
            .as_ref()
            .get(&chunk_opts.compression_method)
            .ok_or_raise(|| CodingError::unsupported_compression(chunk_opts.compression_method))?;
        match chunk_opts.predictor {
            Predictor::None => {
                decoder.decode_chunk(&compressed, out_buf, &chunk_opts)?;
                fix_endianness(
                    out_buf,
                    chunk_opts.byte_order,
                    NATIVE_ENDIAN,
                    chunk_opts.bits_per_sample,
                );
            }
            Predictor::Horizontal => {
                decoder.decode_chunk(&compressed, out_buf, &chunk_opts)?;
                unpredict_hdiff(out_buf, &chunk_opts, x);
            }
            Predictor::FloatingPoint => {
                let mut temp_buf = vec![
                    0u8;
                    chunk_opts.input_row_stride(x).or_raise(|| {
                        CodingError::invalid_tile_index(x, y)
                    })? * chunk_opts.chunk_height as usize
                ];
                decoder.decode_chunk(&compressed, &mut temp_buf, &chunk_opts)?;
                unpredict_float(&mut temp_buf, out_buf, &chunk_opts, x, y)?;
            }
        }
        Ok(())
    }
}
