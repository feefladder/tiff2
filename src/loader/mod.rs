use std::borrow::Cow;

use crate::structs::tags::{
    CompressionMethod, PhotometricInterpretation, PlanarConfiguration, Predictor,
};
use crate::structs::{tags::SampleFormat, Tiff};
use crate::structs::{ChunkOpts, Ifd, IfdEntry, Tag};
use crate::ColorType;

pub(crate) mod metadata;
use bytes::Bytes;
use exn::{bail, ensure, OptionExt, ResultExt};
pub use metadata::{CacheMiss, IfdLoader, MetaError, MetaErrorKind, MetaResult, TiffLoader};
pub(crate) mod tile;
pub use tile::TileServer;

/// Tags that are required to create a TileServer
const REQUIRED_TAGS: [Tag; 3] = [
    Tag::ImageWidth,                // fits in offset (1  SHORT or LONG)
    Tag::ImageLength,               // fits in offset (1 SHORT or LONG)
    Tag::PhotometricInterpretation, // fits in offset (1 SHORT)
];
/// Tags that are needed in a TileServer, but have defaults
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

impl Tiff {
    /// Create a tile server from this tiff at the given overview level (if relevant)
    pub fn tile_server(&self, idx: usize) -> MetaResult<TileServer<'_>> {
        let ifd_offset = self.ifd_offsets[idx];
        let ifd = &self.ifds[&ifd_offset];

        /// Check if provided tags are both present and loaded
        fn ensure_present(tags: &[Tag], ifd: &Ifd, ifd_offset: u64) -> MetaResult<()> {
            let missing_tags: Vec<Tag> = tags
                .iter()
                .filter(|tag| ifd.require_val(tag).is_err())
                .map(|t| *t)
                .collect();
            if !missing_tags.is_empty() {
                Err(MetaError::invalid_ifd(
                    ifd_offset,
                    missing_tags,
                    "could not create TileServer".into(),
                )
                .into())
            } else {
                Ok(())
            }
        }
        ensure_present(&REQUIRED_TAGS, ifd, ifd_offset)?;
        /// Check if provided tags are not deferred
        ///
        /// They may be not present, that is ok
        fn ensure_not_deferred(
            tags: impl Iterator<Item = &'static Tag>,
            ifd: &Ifd,
            ifd_offset: u64,
        ) -> MetaResult<()> {
            let mut ranges = Vec::new();
            let mut missing_tags = Vec::new();
            for (tag, range) in tags.filter_map(|t| ifd.data.get_key_value(t)).filter_map(
                |(t, entry)| match entry {
                    IfdEntry::Offset(o) => Some((t, o.range())),
                    _ => None,
                },
            ) {
                ranges.push(range);
                missing_tags.push(*tag);
            }
            if !missing_tags.is_empty() {
                Err(MetaError::deferred_ifd(
                    ranges,
                    ifd_offset,
                    missing_tags,
                    "optional tags deferred".into(),
                )
                .into())
            } else {
                Ok(())
            }
        }
        // check for all possible tags, so there is a single error
        ensure_not_deferred(
            OPTIONAL_TAGS.iter().chain(&STRIP_TAGS).chain(&TILE_TAGS),
            ifd,
            ifd_offset,
        )?;

        let image_width = u32::try_from(ifd.require_val(&Tag::ImageWidth).unwrap())
            .or_raise(|| MetaError::invalid_tag(Tag::ImageWidth))?;
        let image_height: u32 = u32::try_from(ifd.require_val(&Tag::ImageLength).unwrap())
            .or_raise(|| MetaError::invalid_tag(Tag::ImageLength))?;
        ensure!(
            image_width != 0 && image_height != 0,
            MetaError::permanent(format!("invalid image size {image_width}x{image_height}"))
        );
        let photometric_interpretation = PhotometricInterpretation::from_u16(
            u16::try_from(ifd.require_val(&Tag::PhotometricInterpretation).unwrap())
                .or_raise(|| MetaError::invalid_tag(Tag::PhotometricInterpretation))?,
        )
        .ok_or_raise(|| MetaError::invalid_tag(Tag::PhotometricInterpretation))?;

        let compression_method = match ifd.require_val(&Tag::Compression) {
            Ok(v) => CompressionMethod::from_u16_exhaustive(
                u16::try_from(v).or_raise(|| MetaError::invalid_tag(Tag::Compression))?,
            ),
            Err(_) => CompressionMethod::None,
        };

        let samples_per_pixel = match ifd.require_val(&Tag::SamplesPerPixel) {
            Ok(v) => u16::try_from(v).or_raise(|| MetaError::invalid_tag(Tag::SamplesPerPixel))?,
            Err(_) => 1,
        };
        ensure!(
            samples_per_pixel != 0,
            MetaError::permanent("SamplesPerPixel is zero".into())
        );

        let predictor = match ifd.require_val(&Tag::Predictor) {
            Ok(v) => Predictor::from_u16(
                u16::try_from(v).or_raise(|| MetaError::invalid_tag(Tag::Predictor))?,
            )
            .ok_or_raise(|| MetaError::invalid_tag(Tag::Predictor))?,
            Err(_) => Predictor::None,
        };

        let planar_config = match ifd.require_val(&Tag::PlanarConfiguration) {
            Ok(v) => PlanarConfiguration::from_u16(
                u16::try_from(v).or_raise(|| MetaError::invalid_tag(Tag::PlanarConfiguration))?,
            )
            .ok_or_raise(|| MetaError::invalid_tag(Tag::PlanarConfiguration))?,
            Err(_) => PlanarConfiguration::Chunky,
        };

        let jpeg_tables = if compression_method == CompressionMethod::ModernJPEG
            && ifd.data.contains_key(&Tag::JPEGTables)
        {
            Some(Cow::from(
                // TODO: not copy this data
                ifd.require_val(&Tag::JPEGTables).unwrap().as_ref(),
            ))
        } else {
            None
        };

        let sample_format = match ifd.require_val(&Tag::SampleFormat) {
            Ok(v) => {
                let sfs =
                    <&[u16]>::try_from(v).or_raise(|| MetaError::invalid_tag(Tag::SampleFormat))?;
                ensure!(
                    sfs.windows(2).all(|s| s[0] == s[1]),
                    MetaError::permanent("mixed sample formats unsupported".into())
                );
                SampleFormat::from_u16_exhaustive(sfs[0])
            }
            Err(_) => SampleFormat::Uint,
        };

        let bits_per_sample = match ifd.require_val(&Tag::BitsPerSample) {
            Ok(v) => {
                let bpss =
                    <&[u8]>::try_from(v).or_raise(|| MetaError::invalid_tag(Tag::BitsPerSample))?;
                ensure!(
                    bpss.windows(2).all(|s| s[0] == s[1]),
                    MetaError::permanent("mixed bits per sample unsupported".into())
                );
                bpss[0]
            }
            Err(_) => 1,
        };

        let planes: u32 = match planar_config {
            PlanarConfiguration::Chunky => 1,
            PlanarConfiguration::Planar => samples_per_pixel.into(),
        };

        let tile_offsets: Cow<[u64]>;
        let tile_byte_counts: Cow<[u32]>;
        let tile_width;
        let tile_height;
        match (
            ifd.data.contains_key(&Tag::StripByteCounts),
            ifd.data.contains_key(&Tag::StripOffsets),
            ifd.data.contains_key(&Tag::TileByteCounts),
            ifd.data.contains_key(&Tag::TileOffsets),
        ) {
            (true, true, false, false) => {
                ensure_present(&STRIP_TAGS, ifd, ifd_offset)?;
                // stripped tiff
                tile_offsets =
                    <Cow<[u64]>>::try_from(ifd.require_val(&Tag::StripByteCounts).unwrap())
                        .or_raise(|| MetaError::invalid_tag(Tag::StripByteCounts))?;
                tile_byte_counts =
                    <Cow<[u32]>>::try_from(ifd.require_val(&Tag::StripOffsets).unwrap())
                        .or_raise(|| MetaError::invalid_tag(Tag::StripOffsets))?;
                tile_width = image_width;
                tile_height = if ifd.data.contains_key(&Tag::RowsPerStrip) {
                    u32::try_from(ifd.require_val(&Tag::RowsPerStrip).unwrap())
                        .or_raise(|| MetaError::invalid_tag(Tag::RowsPerStrip))?
                } else {
                    image_height
                };

                ensure!(
                    tile_offsets.len() == tile_byte_counts.len()
                        && tile_height != 0
                        && tile_offsets.len() as u32
                            == image_height.div_ceil(tile_height) * planes as u32,
                    MetaError::permanent(format!(
                        "inconsistency in row data {}!={} or != {}",
                        tile_offsets.len(),
                        tile_byte_counts.len(),
                        image_height.div_ceil(tile_height) * planes as u32
                    ))
                );
            }
            (false, false, true, true) => {
                ensure_present(&TILE_TAGS, ifd, ifd_offset)?;
                let missing_tags: Vec<Tag> = [
                    Tag::TileWidth,
                    Tag::TileLength,
                    Tag::TileOffsets,
                    Tag::TileByteCounts,
                ]
                .iter()
                .filter(|tag| ifd.require_val(tag).is_err())
                .map(|t| *t)
                .collect();
                ensure!(
                    missing_tags.is_empty(),
                    MetaError::invalid_ifd(
                        ifd_offset,
                        missing_tags,
                        "tiled tags are not present".into()
                    )
                );

                tile_width = u32::try_from(ifd.require_val(&Tag::TileWidth).unwrap())
                    .or_raise(|| MetaError::invalid_tag(Tag::TileWidth))?;
                tile_height = u32::try_from(ifd.require_val(&Tag::TileLength).unwrap())
                    .or_raise(|| MetaError::invalid_tag(Tag::TileLength))?;
                tile_offsets = <Cow<[u64]>>::try_from(ifd.require_val(&Tag::TileOffsets).unwrap())
                    .or_raise(|| MetaError::invalid_tag(Tag::TileOffsets))?;
                tile_byte_counts =
                    <Cow<[u32]>>::try_from(ifd.require_val(&Tag::TileByteCounts).unwrap())
                        .or_raise(|| MetaError::invalid_tag(Tag::TileByteCounts))?;

                ensure!(
                    tile_width != 0 && tile_height != 0,
                    MetaError::permanent(format!(
                        "invalid tile dimensions {tile_width}x{tile_height}"
                    ))
                );
                let n_tiles =
                    image_width.div_ceil(tile_width) * image_height.div_ceil(tile_height) * planes;
                ensure!(
                    tile_offsets.len() == tile_byte_counts.len()
                        && tile_offsets.len() as u32 == n_tiles,
                    MetaError::permanent(format!(
                        "inconsistency in tile data {}!={} or != {n_tiles}",
                        tile_offsets.len(),
                        tile_byte_counts.len()
                    ))
                );
            }
            (so, sbc, to, tbc) => bail!(MetaError::permanent(format!(
                "inconsistent strip ({so},{sbc})/tile({to},{tbc}) tags"
            ))),
        }

        Ok(TileServer {
            chunk_opts: ChunkOpts {
                byte_order: self.byte_order,
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
            tile_offsets: tile_offsets,
            tile_byte_counts: tile_byte_counts,
        })
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
#[non_exhaustive]
pub struct Limits {
    /// The maximum size of any `TileData` in bytes, the default is
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
        }
    }
}

impl Default for Limits {
    /// Default limits for reading an image
    /// - 256 MiB for decoding buffer
    /// - 128 MiB for intermediate buffer
    /// - 64 MiB for ifd values
    fn default() -> Limits {
        Limits {
            decoding_buffer_size: 256 * 1024 * 1024,
            intermediate_buffer_size: 128 * 1024 * 1024,
            ifd_value_size: 64 * 1024 * 1024,
        }
    }
}
