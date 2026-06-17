use crate::{
    loader::tile::REQUIRED_TAGS,
    structs::{
        metadata::tags::{PhotometricInterpretation, PlanarConfiguration, Predictor},
        Ifd, Tag, TileOpts,
    },
};

mod predictor;
mod registry;
use derive_more::{Display, Error};
use exn::{ensure, OptionExt, ResultExt};
pub use registry::EncoderRegistry;

pub struct TileSaver {
    pub(crate) tile_opts: TileOpts,
    pub(crate) tile_offsets: Vec<u64>,
    pub(crate) tile_byte_counts: Vec<u32>,
}

#[derive(Debug, Display, Error, Clone, PartialEq)]
pub struct TileSaveError {
    message: String,
}
pub type TileSaveResult<T> = exn::Result<T, TileSaveError>;

fn invalid_ifd(message: String) -> TileSaveError {
    TileSaveError { message }
}

fn ensure_present(tags: &[Tag], ifd: &Ifd) -> TileSaveResult<()> {
    let mut missing_tags = tags
        .iter()
        .filter(|tag| ifd.require_val(tag).is_err())
        .peekable();
    if missing_tags.peek().is_some() {
        let n_missing = missing_tags.collect::<Vec<_>>();
        Err(invalid_ifd(format!("missing {n_missing:?} required tags")).into())
    } else {
        Ok(())
    }
}

impl TileSaver {
    /// Check if an ifd can be made into a TileSaver
    ///
    /// This requires:
    /// - non-zero `ImageWidth` and `ImageHeight` that fit in `u32`
    /// - Sensible [`PhotometricInterpretation`]
    /// - non-zero or no SamplesPerPixel
    /// - consistent or no SampleFormat
    /// - BitsPerSample within u16
    /// - CompressionMethod within u16
    /// - sensible [`PlanarConfiguration`]
    /// - either stripped or tiled, not both
    ///   - Array length in the IFD must match calculated lenghts
    pub fn check_ifd(ifd: &Ifd) -> TileSaveResult<()> {
        // So at this point, th
        ensure_present(&REQUIRED_TAGS, ifd)?;
        let invalid_tag = |tag: Tag| {
            move || TileSaveError {
                message: format!("tag {tag:?} invalid"),
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

        let planar_config = ifd.require_val(&Tag::PlanarConfiguration).map(|v|
            PlanarConfiguration::from_u16(
                u16::try_from(v).or_raise(invalid_tag(Tag::PlanarConfiguration))?,
            ))
            .ok_or_raise(invalid_tag(Tag::PlanarConfiguration))?
        };
        let planes: u32 = match planar_config {
            PlanarConfiguration::Chunky => 1,
            PlanarConfiguration::Planar => samples_per_pixel.into(),
        };

        match (
            ifd.contains_tag(&Tag::StripByteCounts),
            ifd.contains_tag(&Tag::StripOffsets),
            ifd.contains_tag(&Tag::TileByteCounts),
            ifd.contains_tag(&Tag::TileOffsets),
        ) {
            (true, true, false, false) => {
                let n_so = ifd.tag_n_values(&Tag::StripOffsets).unwrap();
                let n_sbc = ifd.tag_n_values(&Tag::StripByteCounts).unwrap();
                // TODO: is this 1 or image_height?
                let rps = ifd
                    .require_val(&Tag::RowsPerStrip)
                    .map(|v| u32::try_from(v).unwrap())
                    .unwrap_or(image_height);
                ensure!(
                    n_so == n_sbc,
                    invalid_ifd("strip offsets doesn't match byte counts".to_string())
                );
                let n_expected = image_height.div_ceil(rps) * planes;
                ensure!(
                    n_so as u32 == n_expected,
                    invalid_ifd(format!("inconsistency in row data {n_so}!={n_expected}"))
                );
            }
            (false, false, true, true) => {}
            (sbc, so, tbc, to) => {
                let n_to = ifd.tag_n_values(&Tag::TileOffsets).unwrap();
                let n_tbc = ifd.tag_n_values(&Tag::TileByteCounts).unwrap();
            }
        }
        todo!()
    }
}
