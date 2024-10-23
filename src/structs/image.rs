use crate::{
    entry::{BufferedEntry, IfdEntry}, error::{TiffError, TiffFormatError, TiffResult, TiffUnsupportedError}, ifd::Ifd, tags::{
        CompressionMethod, PhotometricInterpretation, PlanarConfiguration, Predictor, SampleFormat,
        Tag,
    }, ByteOrder, ChunkType
};

use std::{collections::HashMap, sync::{Arc, Condvar, Mutex, RwLock}};

use super::tags::TagType;

#[derive(Debug, Clone)]
pub struct StripDecodeState {
    pub rows_per_strip: u32,
}

#[derive(Debug, Clone)]
/// Computed values useful for tile decoding
pub struct TileAttributes {
    pub image_width: usize,
    pub image_height: usize,

    pub tile_width: usize,
    pub tile_length: usize,
}

impl TileAttributes {
    pub fn tiles_across(&self) -> usize {
        (self.image_width + self.tile_width - 1) / self.tile_width
    }
    pub fn tiles_down(&self) -> usize {
        (self.image_height + self.tile_length - 1) / self.tile_length
    }
    fn padding_right(&self) -> usize {
        (self.tile_width - self.image_width % self.tile_width) % self.tile_width
    }
    fn padding_down(&self) -> usize {
        (self.tile_length - self.image_height % self.tile_length) % self.tile_length
    }
    pub fn get_padding(&self, tile: usize) -> (usize, usize) {
        let row = tile / self.tiles_across();
        let column = tile % self.tiles_across();

        let padding_right = if column == self.tiles_across() - 1 {
            self.padding_right()
        } else {
            0
        };

        let padding_down = if row == self.tiles_down() - 1 {
            self.padding_down()
        } else {
            0
        };

        (padding_right, padding_down)
    }
}


/// Struct that holds all relevant metadata that is needed to decode a chunk
/// (strip or tile).
/// this does not include chunkoffsets or -bytes, since those may be partial and
/// then mutated.
pub struct ChunkMetaData {
    pub byte_order: ByteOrder,
    pub image_width: u32,
    pub image_height: u32,
    pub bits_per_sample: u8,
    pub samples: u16,
    pub sample_format: SampleFormat,
    pub photometric_interpretation: PhotometricInterpretation,
    pub compression_method: CompressionMethod,
    pub predictor: Predictor,
    pub jpeg_tables: Option<BufferedEntry>,
    pub planar_config: PlanarConfiguration,
    pub chunk_type: ChunkType,
    pub strip_decoder: Option<StripDecodeState>,
    pub tile_attributes: Option<TileAttributes>,
}

pub enum MaybePartial {
    Whole(BufferedEntry),
    Partial{
        // tag_type: TagType,
        offset: u64,
        chunk_size: usize,
        data: Arc<RwLock<HashMap<u64, BufferedEntry>>>,
        pending_chunks: Arc<Mutex<HashMap<u64, Condvar>>>,
    }
}

pub enum MaybePartialIndex<T> {
    Ok(T),
    NeedRead{
        offset: u64,
        count: u64,
        buf: Vec<u8>
    },
    Pending(Condvar),
}


impl MaybePartial {
    fn get_u64(&self, index: usize) -> TiffResult<MaybePartialIndex<u64>> {
        match self {
            MaybePartial::Whole(e) => Ok(MaybePartialIndex::Ok(e.get_u64(index)?)),
            MaybePartial::Partial { offset, chunk_size, data, pending_chunks } => {
                let i_chunk: usize = index / chunk_size;
                let subindex: usize = index % chunk_size;
                if let Some(entry) = data.try_read()?.get(&i_chunk.try_into()?) {
                    Ok(MaybePartialIndex::Ok(entry.get_u64(subindex)?))
                } else {
                    if let Some(cv) = pending_chunks.try_lock()?.get(&i_chunk.try_into()?) {
                        Ok(MaybePartialIndex::Pending(cv.clone()))
                    } else {
                        pending_chunks.try_lock()?.insert(i_chunk.try_into()?, Condvar::new());
                        Ok(MaybePartialIndex::NeedRead { offset: *offset , count: u64::try_from(*chunk_size)?, buf: vec![0u8; *chunk_size] })
                    }
                }
            }
        }
    }
}

/// Image struct that holds all relevant metadata for locating an image's data in the file and which decoding method to use
pub struct Image {
    /// IFD holding all data
    pub ifd: Ifd,
    /// Data that doesn't change between chunks
    pub chunk_metadata: Arc<ChunkMetaData>,
    /// Chunk offsets (maybe partially loaded)
    pub chunk_offsets: BufferedEntry,
    // Number of bytes per chunk (maybe partially loaded)
    pub chunk_bytes: BufferedEntry,
}


const IMAGE_TAGS: [Tag; 14] = [
    Tag::ImageWidth,
    Tag::ImageLength,

    Tag::BitsPerSample,
    Tag::SamplesPerPixel,
    Tag::SampleFormat,

    Tag::PhotometricInterpretation,
    Tag::Compression,
    Tag::Predictor,
    Tag::PlanarConfiguration,
    Tag::JPEGTables,
    
    Tag::StripByteCounts,
    Tag::StripOffsets,
    Tag::TileByteCounts,
    Tag::TileOffsets,
];

impl Image {
    // pub fn chunk_offsets(&self) -> &BufferedEntry {
    //     match self.
    // }
    
    pub fn from_ifd(
        // reader: &mut SmartReader<R>,
        ifd: Ifd,
        // limits: &Limits,
        bigtiff: bool,
    ) -> TiffResult<Image> {
        // ------------------------------
        // Tags that fit in offset fields
        // ------------------------------
        let width: u32 = ifd.require_tag_value(&Tag::ImageWidth)?.try_into()?;
        let height: u32 = ifd.require_tag_value(&Tag::ImageLength)?.try_into()?;
        if width == 0 || height == 0 {
            return Err(TiffError::FormatError(TiffFormatError::InvalidDimensions(
                width, height,
            )));
        }

        let photometric_interpretation = ifd
            .get_tag_value(&Tag::PhotometricInterpretation)?
            .map(u16::try_from)
            .transpose()?
            .and_then(PhotometricInterpretation::from_u16)
            .ok_or(TiffUnsupportedError::UnknownInterpretation)?;

        // Try to parse both the compression method and the number, format, and bits of the included samples.
        // If they are not explicitly specified, those tags are reset to their default values and not carried from previous images.
        let compression_method = match ifd.get_tag_value(&Tag::Compression)? {
            Some(val) => CompressionMethod::from_u16_exhaustive(u16::try_from(val)?),
            None => CompressionMethod::None,
        };

        let samples: u16 = ifd
            .get_tag_value(&Tag::SamplesPerPixel)?
            .map(u16::try_from)
            .transpose()?
            .unwrap_or(1);
        if samples == 0 {
            return Err(TiffFormatError::SamplesPerPixelIsZero.into());
        }

        let predictor = ifd
            .get_tag_value(&Tag::Predictor)?
            .map(u16::try_from)
            .transpose()?
            .map(|p| {
                Predictor::from_u16(p)
                    .ok_or(TiffError::FormatError(TiffFormatError::UnknownPredictor(p)))
            })
            .transpose()?
            .unwrap_or(Predictor::None);

        let planar_config = ifd
            .get_tag_value(&Tag::PlanarConfiguration)?
            .map(u16::try_from)
            .transpose()?
            .map(|p| {
                PlanarConfiguration::from_u16(p).ok_or(TiffError::FormatError(
                    TiffFormatError::UnknownPlanarConfiguration(p),
                ))
            })
            .transpose()?
            .unwrap_or(PlanarConfiguration::Chunky);

        let planes = match planar_config {
            PlanarConfiguration::Chunky => 1,
            PlanarConfiguration::Planar => samples,
        };

        let jpeg_tables = if compression_method == CompressionMethod::ModernJPEG
            && ifd.contains_key(&Tag::JPEGTables)
        {
            let vec = ifd
                .find_tag(Tag::JPEGTables)?
                .unwrap()
                .into_u8_vec()?;
            if vec.len() < 2 {
                return Err(TiffError::FormatError(
                    TiffFormatError::InvalidTagValueType(Tag::JPEGTables.to_u16()),
                ));
            }

            Some(Arc::new(vec))
        } else {
            None
        };

        // let sample_format = match tag_reader.find_tag_uint_vec(Tag::SampleFormat)? {
        //     Some(vals) => {
        //         let sample_format: Vec<_> = vals
        //             .into_iter()
        //             .map(SampleFormat::from_u16_exhaustive)
        //             .collect();

        //         // TODO: for now, only homogenous formats across samples are supported.
        //         if !sample_format.windows(2).all(|s| s[0] == s[1]) {
        //             return Err(TiffUnsupportedError::UnsupportedSampleFormat(sample_format).into());
        //         }

        //         sample_format[0]
        //     }
        //     None => SampleFormat::Uint,
        // };

        // let bits_per_sample: Vec<u8> = tag_reader
        //     .find_tag_uint_vec(Tag::BitsPerSample)?
        //     .unwrap_or_else(|| vec![1]);

        // // Technically bits_per_sample.len() should be *equal* to samples, but libtiff also allows
        // // it to be a single value that applies to all samples.
        // if bits_per_sample.len() != usize::from(samples) && bits_per_sample.len() != 1 {
        //     return Err(TiffError::FormatError(
        //         TiffFormatError::InconsistentSizesEncountered,
        //     ));
        // }

        // // This library (and libtiff) do not support mixed sample formats and zero bits per sample
        // // doesn't make sense.
        // if bits_per_sample.iter().any(|&b| b != bits_per_sample[0]) || bits_per_sample[0] == 0 {
        //     return Err(TiffUnsupportedError::InconsistentBitsPerSample(bits_per_sample).into());
        // }

        // let chunk_type;
        // let chunk_offsets;
        // let chunk_bytes;
        // let strip_decoder;
        // let tile_attributes;
        // match (
        //     ifd.contains_key(&Tag::StripByteCounts),
        //     ifd.contains_key(&Tag::StripOffsets),
        //     ifd.contains_key(&Tag::TileByteCounts),
        //     ifd.contains_key(&Tag::TileOffsets),
        // ) {
        //     (true, true, false, false) => {
        //         chunk_type = ChunkType::Strip;

        //         chunk_offsets = tag_reader
        //             .find_tag(Tag::StripOffsets)?
        //             .unwrap()
        //             .into_u64_vec()?;
        //         chunk_bytes = tag_reader
        //             .find_tag(Tag::StripByteCounts)?
        //             .unwrap()
        //             .into_u64_vec()?;
        //         let rows_per_strip = tag_reader
        //             .find_tag(Tag::RowsPerStrip)?
        //             .map(Value::into_u32)
        //             .transpose()?
        //             .unwrap_or(height);
        //         strip_decoder = Some(StripDecodeState { rows_per_strip });
        //         tile_attributes = None;

        //         if chunk_offsets.len() != chunk_bytes.len()
        //             || rows_per_strip == 0
        //             || u32::try_from(chunk_offsets.len())?
        //                 != (height.saturating_sub(1) / rows_per_strip + 1) * planes as u32
        //         {
        //             return Err(TiffError::FormatError(
        //                 TiffFormatError::InconsistentSizesEncountered,
        //             ));
        //         }
        //     }
        //     (false, false, true, true) => {
        //         chunk_type = ChunkType::Tile;

        //         let tile_width =
        //             usize::try_from(tag_reader.require_tag(Tag::TileWidth)?.into_u32()?)?;
        //         let tile_length =
        //             usize::try_from(tag_reader.require_tag(Tag::TileLength)?.into_u32()?)?;

        //         if tile_width == 0 {
        //             return Err(TiffFormatError::InvalidTagValueType(Tag::TileWidth).into());
        //         } else if tile_length == 0 {
        //             return Err(TiffFormatError::InvalidTagValueType(Tag::TileLength).into());
        //         }

        //         strip_decoder = None;
        //         tile_attributes = Some(TileAttributes {
        //             image_width: usize::try_from(width)?,
        //             image_height: usize::try_from(height)?,
        //             tile_width,
        //             tile_length,
        //         });
        //         chunk_offsets = tag_reader
        //             .find_tag(Tag::TileOffsets)?
        //             .unwrap()
        //             .into_u64_vec()?;
        //         chunk_bytes = tag_reader
        //             .find_tag(Tag::TileByteCounts)?
        //             .unwrap()
        //             .into_u64_vec()?;

        //         let tile = tile_attributes.as_ref().unwrap();
        //         if chunk_offsets.len() != chunk_bytes.len()
        //             || chunk_offsets.len()
        //                 != tile.tiles_down() * tile.tiles_across() * planes as usize
        //         {
        //             return Err(TiffError::FormatError(
        //                 TiffFormatError::InconsistentSizesEncountered,
        //             ));
        //         }
        //     }
        //     (_, _, _, _) => {
        //         return Err(TiffError::FormatError(
        //             TiffFormatError::StripTileTagConflict,
        //         ))
        //     }
        // };
        todo!()
    }
}

mod test {
    use crate::tags::TagType;

    use super::*;

    #[test]
    fn test_arcyness() {
        let asdf = Arc::new(BufferedEntry {
            tag_type: TagType::BYTE,
            count: 5,
            data: vec![42, 43, 44, 45, 46],
        });
        assert_eq!(asdf.get_u64(2).unwrap(), 43);
    }
}
