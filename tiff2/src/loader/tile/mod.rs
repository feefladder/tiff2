use std::error::Error;
use std::ops::Range;

use bytes::Bytes;
use derive_more::Display;
use exn::{bail, ensure, OptionExt, ResultExt};
use rayon::iter::IndexedParallelIterator;

use crate::loader::tile::predictor::{unpredict_float, unpredict_hdiff};
use crate::structs::error::CodingError;
use crate::structs::metadata::tags::{
    CompressionMethod, PhotometricInterpretation, PlanarConfiguration, Predictor, SampleFormat,
};
use crate::structs::{Ifd, IfdEntry, Tag, Tiff, TileCoord, TileData, TileOpts};
use crate::util::fix_endianness;
use crate::{ByteOrder, NATIVE_ENDIAN};

mod predictor;
#[cfg(test)]
pub(crate) use predictor::{unpredict_f32, unpredict_f64};
mod registry;
pub use registry::DecoderRegistry;

pub type CodingResult<T> = exn::Result<T, CodingError>;
pub type TileLoadResult<T> = exn::Result<T, TileLoadError>;

#[derive(Debug, Clone, Display)]
#[display("{kind}: {message}")]
pub struct TileLoadError {
    pub message: String,
    pub kind: TileLoadErrorKind,
}
impl Error for TileLoadError {}

#[derive(Debug, Clone, Display)]
pub enum TileLoadErrorKind {
    #[display("fatal error")]
    Fatal,
    #[display("ifd invalid")]
    InvalidIfd,
    #[display("ifd has deferred tags")]
    DeferredIfd,
}

fn invalid_ifd(message: String) -> TileLoadError {
    TileLoadError {
        message,
        kind: TileLoadErrorKind::InvalidIfd,
    }
}

fn deferred_ifd(message: String) -> TileLoadError {
    TileLoadError {
        message,
        kind: TileLoadErrorKind::DeferredIfd,
    }
}

fn fatal(message: String) -> TileLoadError {
    TileLoadError {
        message,
        kind: TileLoadErrorKind::Fatal,
    }
}

#[derive(Debug)]
pub struct TileLoader {
    pub(crate) tile_opts: TileOpts,
    pub(crate) tile_offsets: Vec<u64>,
    pub(crate) tile_byte_counts: Vec<u32>,
}

impl TileLoader {
    pub fn tile_range(&self, coord: TileCoord) -> exn::Result<Range<u64>, TileLoadError> {
        let i = self
            .tile_opts
            .coord2i(coord)
            .or_raise(|| fatal(format!("could not get range for {coord:?}")))?;
        let start = self.tile_offsets[i];
        Ok(start..start + u64::from(self.tile_byte_counts[i]))
    }

    pub fn tiles_ranges<'b>(
        &'b self,
        coords: impl Iterator<Item = &'b TileCoord> + 'b,
    ) -> impl Iterator<Item = (TileCoord, exn::Result<Range<u64>, TileLoadError>)> + 'b {
        coords.map(|tc| (*tc, self.tile_range(*tc)))
    }

    /// Get decoded tiles from their data
    ///
    /// This will decode tiles in parallel
    pub fn get_tiles<'b>(
        &'b self,
        tile_datas: impl IndexedParallelIterator<Item = (TileCoord, Bytes)> + 'b,
        decoder_registry: &'b DecoderRegistry,
    ) -> impl IndexedParallelIterator<Item = (TileCoord, CodingResult<TileData>)> + 'b {
        tile_datas.map(|(coord, buf)| {
            (
                coord,
                Self::decode(coord.x, coord.y, &self.tile_opts, buf, decoder_registry),
            )
        })
    }

    pub fn decode(
        x: u32,
        y: u32,
        tile_opts: &TileOpts,
        compressed: Bytes,
        decoder_registry: &DecoderRegistry,
    ) -> exn::Result<TileData, CodingError> {
        // output size in number of samples
        // we don't support 16-bit pointer archs
        let output_size = usize::try_from(
            tile_opts
                .chunk_width_pixels(x)
                .or_raise(|| CodingError::invalid_tile_index(x, y))?,
        )
        .unwrap()
        .saturating_mul(
            usize::try_from(
                tile_opts
                    .chunk_height_pixels(y)
                    .or_raise(|| CodingError::invalid_tile_index(x, y))?,
            )
            .unwrap(),
        )
        .saturating_mul(tile_opts.samples_per_pixel.into());
        let mut out_buf = TileData::new(
            output_size,
            tile_opts.dtype().or_raise(|| {
                CodingError::unsupported_bit_depth(tile_opts.bits_per_sample, "could not get dtype")
            })?,
        );
        Self::decode_into(
            x,
            y,
            tile_opts,
            compressed,
            out_buf.as_mut(),
            decoder_registry,
        )?;
        Ok(out_buf)
    }

    pub fn decode_into(
        x: u32,
        y: u32,
        tile_opts: &TileOpts,
        compressed: Bytes,
        out_buf: &mut [u8],
        decoder_registry: &DecoderRegistry,
    ) -> exn::Result<(), CodingError> {
        let decoder = decoder_registry
            .as_ref()
            .get(&tile_opts.compression_method)
            .ok_or_raise(|| CodingError::unsupported_compression(tile_opts.compression_method))?;
        match tile_opts.predictor {
            Predictor::None => {
                decoder.decode_tile(&compressed, out_buf, tile_opts)?;
                fix_endianness(
                    out_buf,
                    tile_opts.byte_order,
                    NATIVE_ENDIAN,
                    tile_opts.bits_per_sample,
                );
            }
            Predictor::Horizontal => {
                decoder.decode_tile(&compressed, out_buf, tile_opts)?;
                unpredict_hdiff(out_buf, tile_opts, x);
            }
            Predictor::FloatingPoint => {
                let mut temp_buf = vec![
                    0u8;
                    tile_opts.input_row_stride(x).or_raise(|| {
                        CodingError::invalid_tile_index(x, y)
                    })? * tile_opts.tile_height as usize
                ];
                decoder.decode_tile(&compressed, &mut temp_buf, tile_opts)?;
                unpredict_float(&mut temp_buf, out_buf, tile_opts, x)?;
            }
        }
        Ok(())
    }
}

/// Tags that are required to create a TileLoader
const REQUIRED_TAGS: [Tag; 3] = [
    Tag::ImageWidth,                // fits in offset (1  SHORT or LONG)
    Tag::ImageLength,               // fits in offset (1 SHORT or LONG)
    Tag::PhotometricInterpretation, // fits in offset (1 SHORT)
];
/// Tags that are needed in a TileLoader, but have defaults
const OPTIONAL_TAGS: [Tag; 7] = [
    Tag::BitsPerSample,       // may not fit  (SamplesPerPixel SHORT)
    Tag::SamplesPerPixel,     // fits in offset (1 SHORT)
    Tag::SampleFormat,        // may not fit SamplesPerPixel SHORT
    Tag::Compression,         // fits (1 SHORT)
    Tag::Predictor,           // fits (1 SHORT)
    Tag::PlanarConfiguration, // fits (1 SHORT)
    Tag::JPEGTables,          // may not fit
];
const STRIP_TAGS: [Tag; 3] = [Tag::StripOffsets, Tag::StripByteCounts, Tag::RowsPerStrip];
const TILE_TAGS: [Tag; 4] = [
    Tag::TileOffsets,
    Tag::TileByteCounts,
    Tag::TileWidth,
    Tag::TileLength,
];

/// Check if provided tags are both present and loaded
fn ensure_present(tags: &[Tag], ifd: &Ifd) -> TileLoadResult<()> {
    let missing_tags: Vec<Tag> = tags
        .iter()
        .filter(|tag| ifd.require_val(tag).is_err())
        .copied()
        .collect();
    if !missing_tags.is_empty() {
        Err(invalid_ifd(format!("missing {} required tags", missing_tags.len())).into())
    } else {
        Ok(())
    }
}

/// Check if provided tags are not deferred
///
/// They may be not present, that is ok
fn ensure_not_deferred(tags: impl Iterator<Item = &'static Tag>, ifd: &Ifd) -> TileLoadResult<()> {
    let deferred_tags: Vec<_> = tags
        .filter_map(|t| ifd.data.get(t))
        .filter(|entry| matches!(entry, IfdEntry::Offset(_)))
        .collect();
    if !deferred_tags.is_empty() {
        Err(deferred_ifd(format!("{} optional tags deferred", deferred_tags.len())).into())
    } else {
        Ok(())
    }
}

impl TileLoader {
    pub fn check_ifd(ifd: &Ifd) -> TileLoadResult<()> {
        ensure_present(&REQUIRED_TAGS, ifd)?;
        ensure_not_deferred(
            OPTIONAL_TAGS.iter().chain(&STRIP_TAGS).chain(&TILE_TAGS),
            ifd,
        )?;
        let invalid_tag = |tag: Tag| {
            move || TileLoadError {
                message: format!("tag {tag:?} invalid"),
                kind: TileLoadErrorKind::Fatal,
            }
        };
        // required tags
        let image_width = u32::try_from(ifd.require_val(&Tag::ImageWidth).unwrap())
            .or_raise(invalid_tag(Tag::ImageWidth))?;
        let image_height: u32 = u32::try_from(ifd.require_val(&Tag::ImageLength).unwrap())
            .or_raise(invalid_tag(Tag::ImageLength))?;
        ensure!(
            image_width != 0 && image_height != 0,
            invalid_ifd(format!("invalid image size {image_width}x{image_height}"))
        );
        PhotometricInterpretation::from_u16(
            u16::try_from(ifd.require_val(&Tag::PhotometricInterpretation).unwrap())
                .or_raise(invalid_tag(Tag::PhotometricInterpretation))?,
        )
        .ok_or_raise(invalid_tag(Tag::PhotometricInterpretation))?;

        // optional tags
        if let Ok(v) = ifd.require_val(&Tag::SamplesPerPixel) {
            ensure!(
                u16::try_from(v).or_raise(invalid_tag(Tag::SamplesPerPixel))? != 0,
                invalid_ifd("SamplesPerPixel is zero".into())
            )
        };

        if let Ok(v) = ifd.require_val(&Tag::SampleFormat) {
            let sfs = <&[u16]>::try_from(v).or_raise(invalid_tag(Tag::SampleFormat))?;
            ensure!(
                sfs.windows(2).all(|s| s[0] == s[1]),
                invalid_ifd("mixed sample formats unsupported".into())
            );
        }

        if let Ok(v) = ifd.require_val(&Tag::BitsPerSample) {
            let bpss = <&[u16]>::try_from(v).or_raise(invalid_tag(Tag::BitsPerSample))?;
            ensure!(
                bpss.windows(2).all(|s| s[0] == s[1]),
                invalid_ifd("mixed bits per sample unsupported".into())
            );
        }

        if let Ok(v) = ifd.require_val(&Tag::Predictor) {
            Predictor::from_u16(u16::try_from(v).or_raise(invalid_tag(Tag::Predictor))?)
                .ok_or_raise(invalid_tag(Tag::Predictor))?;
        }

        if let Ok(v) = ifd.require_val(&Tag::PlanarConfiguration) {
            PlanarConfiguration::from_u16(
                u16::try_from(v).or_raise(invalid_tag(Tag::PlanarConfiguration))?,
            )
            .ok_or_raise(invalid_tag(Tag::PlanarConfiguration))?;
        }
        match (
            ifd.data.contains_key(&Tag::StripByteCounts),
            ifd.data.contains_key(&Tag::StripOffsets),
            ifd.data.contains_key(&Tag::TileByteCounts),
            ifd.data.contains_key(&Tag::TileOffsets),
        ) {
            (true, true, false, false) => {
                ensure_present(&STRIP_TAGS, ifd)?;
                if let Ok(v) = ifd.require_val(&Tag::RowsPerStrip) {
                    ensure!(
                        u32::try_from(v).or_raise(invalid_tag(Tag::RowsPerStrip))? != 0,
                        invalid_ifd(format!("rows per strip {v:?} invalid"))
                    )
                }
                ensure!(
                    ifd.require_val(&Tag::StripByteCounts).unwrap().len()
                        == ifd.require_val(&Tag::StripOffsets).unwrap().len(),
                    invalid_ifd("strip offsets doesn't match byte counts".to_string())
                )
            }
            (false, false, true, true) => {
                ensure_present(&TILE_TAGS, ifd)?;
                let tw = u16::try_from(ifd.require_val(&Tag::TileWidth).unwrap())
                    .or_raise(invalid_tag(Tag::TileWidth))?;
                let tl = u16::try_from(ifd.require_val(&Tag::TileLength).unwrap())
                    .or_raise(invalid_tag(Tag::TileLength))?;
                ensure!(
                    tw != 0 && tl != 0,
                    invalid_ifd(format!("invalid tile dimensions {tw}x{tl}"))
                );
                ensure!(
                    ifd.require_val(&Tag::TileOffsets).unwrap().len()
                        == ifd.require_val(&Tag::TileByteCounts).unwrap().len(),
                    invalid_ifd("strip offsets doesn't match byte counts".to_string())
                )
            }
            (so, sbc, to, tbc) => bail!(invalid_ifd(format!(
                "inconsistent strip/tile ({so},{sbc})/({to},{tbc}) tags"
            ))),
        }
        Ok(())
    }

    pub fn from_tiff(tiff: &mut Tiff, idx: usize) -> TileLoadResult<Self> {
        let ifd_offset = tiff.ifd_offsets[idx];
        let ifd = tiff
            .ifds
            .remove(&ifd_offset)
            .ok_or_raise(|| fatal(format!("ifd {idx} at {ifd_offset} not found")))?;
        Self::from_ifd(ifd, ifd_offset, tiff.byte_order())
    }
    /// Create a tile server from this tiff at the given overview level (if relevant)
    ///
    /// This removes the Ifd from the tiff.
    ///
    // TODO: make this a check and infallible structure
    pub fn from_ifd(mut ifd: Ifd, ifd_offset: u64, byte_order: ByteOrder) -> TileLoadResult<Self> {
        Self::check_ifd(&ifd).or_raise(|| invalid_ifd("invalid ifd".into()))?;
        let invalid_tag = |tag: Tag| {
            move || TileLoadError {
                message: format!("tag {tag:?} at {ifd_offset} invalid"),
                kind: TileLoadErrorKind::Fatal,
            }
        };

        let image_width = u32::try_from(ifd.require_val(&Tag::ImageWidth).unwrap()).unwrap();
        let image_height: u32 = u32::try_from(ifd.require_val(&Tag::ImageLength).unwrap()).unwrap();

        let photometric_interpretation = PhotometricInterpretation::from_u16(
            u16::try_from(ifd.require_val(&Tag::PhotometricInterpretation).unwrap()).unwrap(),
        )
        .unwrap();

        let compression_method = ifd
            .require_val(&Tag::Compression)
            .map(|v| CompressionMethod::from_u16_exhaustive(u16::try_from(v).unwrap()))
            .unwrap_or(CompressionMethod::None);

        let samples_per_pixel = ifd
            .require_val(&Tag::SamplesPerPixel)
            .map(|v| u16::try_from(v).unwrap())
            .unwrap_or(1);

        let predictor = ifd
            .require_val(&Tag::Predictor)
            .map(|v| Predictor::from_u16(u16::try_from(v).unwrap()).unwrap())
            .unwrap_or(Predictor::None);

        let planar_config = ifd
            .require_val(&Tag::PlanarConfiguration)
            .map(|v| PlanarConfiguration::from_u16(u16::try_from(v).unwrap()).unwrap())
            .unwrap_or(PlanarConfiguration::Chunky);

        let jpeg_tables = if compression_method == CompressionMethod::ModernJPEG
            && ifd.data.contains_key(&Tag::JPEGTables)
        {
            let IfdEntry::Value(jt_tb) = ifd.data.remove(&Tag::JPEGTables).unwrap() else {
                unreachable!()
            };
            Some(<Vec<u8>>::try_from(jt_tb).or_raise(invalid_tag(Tag::JPEGTables))?)
        } else {
            None
        };

        let sample_format = ifd
            .require_val(&Tag::SampleFormat)
            .map(|v| SampleFormat::from_u16_exhaustive(<&[u16]>::try_from(v).unwrap()[0]))
            .unwrap_or(SampleFormat::Uint);

        let bits_per_sample = ifd
            .require_val(&Tag::BitsPerSample)
            .map(|v| <&[u16]>::try_from(v).unwrap()[0])
            .unwrap_or(1);

        let planes: u32 = match planar_config {
            PlanarConfiguration::Chunky => 1,
            PlanarConfiguration::Planar => samples_per_pixel.into(),
        };

        let tile_offsets: Vec<u64>;
        let tile_byte_counts: Vec<u32>;
        let tile_width;
        let tile_height;
        match (
            ifd.data.contains_key(&Tag::StripByteCounts),
            ifd.data.contains_key(&Tag::StripOffsets),
            ifd.data.contains_key(&Tag::TileByteCounts),
            ifd.data.contains_key(&Tag::TileOffsets),
        ) {
            (true, true, false, false) => {
                // stripped tiff
                let IfdEntry::Value(so_td) = ifd.data.remove(&Tag::StripOffsets).unwrap() else {
                    unreachable!()
                };
                tile_offsets = <Vec<u64>>::try_from(so_td).unwrap();
                let IfdEntry::Value(sbc_td) = ifd.data.remove(&Tag::StripByteCounts).unwrap()
                else {
                    unreachable!()
                };
                // this can fail if there were a strip bigger than 4 GB
                tile_byte_counts =
                    <Vec<u32>>::try_from(sbc_td).or_raise(invalid_tag(Tag::StripByteCounts))?;
                tile_width = image_width;
                // TODO: is this 1 or image_height?
                tile_height = ifd
                    .require_val(&Tag::RowsPerStrip)
                    .map(|v| u32::try_from(v).unwrap())
                    .unwrap_or(image_height);

                ensure!(
                    tile_offsets.len() as u32 == image_height.div_ceil(tile_height) * planes,
                    invalid_ifd(format!(
                        "inconsistency in row data {}!={}",
                        tile_offsets.len(),
                        image_height.div_ceil(tile_height) * planes
                    ))
                );
            }
            (false, false, true, true) => {
                tile_width = u32::try_from(ifd.require_val(&Tag::TileWidth).unwrap()).unwrap();
                tile_height = u32::try_from(ifd.require_val(&Tag::TileLength).unwrap()).unwrap();
                let IfdEntry::Value(to_td) = ifd.data.remove(&Tag::TileOffsets).unwrap() else {
                    unreachable!()
                };
                tile_offsets = <Vec<u64>>::try_from(to_td).unwrap();
                let IfdEntry::Value(tbc_td) = ifd.data.remove(&Tag::TileByteCounts).unwrap() else {
                    unreachable!()
                };
                // this can fail if there were a tile larger than 4GB
                tile_byte_counts =
                    <Vec<u32>>::try_from(tbc_td).or_raise(invalid_tag(Tag::TileByteCounts))?;
                let n_tiles =
                    image_width.div_ceil(tile_width) * image_height.div_ceil(tile_height) * planes;
                ensure!(
                    tile_offsets.len() as u32 == n_tiles,
                    invalid_ifd(format!(
                        "inconsistency in tile data {}!={n_tiles}",
                        tile_offsets.len(),
                    ))
                );
            }
            (_so, _sbc, _to, _tbc) => unreachable!(),
        }

        Ok(TileLoader {
            tile_opts: TileOpts {
                byte_order,
                image_width,
                image_height,
                bits_per_sample,
                samples_per_pixel,
                sample_format,
                photometric_interpretation,
                compression_method,
                predictor,
                jpeg_tables,
                planar_config,
                tile_width,
                tile_height,
            },
            tile_offsets,
            tile_byte_counts,
        })
    }
}
