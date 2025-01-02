use crate::{
    error::{TiffError, TiffFormatError, TiffResult, TiffUnsupportedError, UsageError},
    structs::{
        tags::{
            CompressionMethod, PhotometricInterpretation, PlanarConfiguration, Predictor,
            SampleFormat, Tag,
        },
        Ifd, IfdEntry, Offset, TagData,
    },
    ByteOrder, ChunkType, ColorType,
};

use std::{collections::BTreeMap, fmt::Debug, ops::Range, sync::Arc};

#[derive(Debug, Clone, PartialEq)]
pub struct StripDecodeState {
    pub rows_per_strip: u32,
}

#[derive(Debug, Clone, PartialEq)]
/// Computed values useful for tile decoding
pub struct TileAttributes {
    pub image_width: usize,
    pub image_height: usize,

    pub tile_width: usize,
    pub tile_length: usize,
}

impl TileAttributes {
    pub fn tiles_across(&self) -> usize {
        self.image_width.div_ceil(self.tile_width)
    }
    pub fn tiles_down(&self) -> usize {
        self.image_height.div_ceil(self.tile_length)
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
/// then mutated. once we implement partial tags
#[derive(Debug, PartialEq, Clone)]
pub struct ChunkOpts {
    /// tiff byte order
    pub byte_order: ByteOrder,
    /// width of the image in pixels
    pub image_width: u32,
    /// height of the image in pixels
    pub image_height: u32,
    /// bits per sample
    pub bits_per_sample: u8,
    /// samples per pixel
    pub samples: u16,
    /// datatype of samples
    pub sample_format: SampleFormat,
    /// photometric interpretation
    pub photometric_interpretation: PhotometricInterpretation,
    /// compression method
    ///
    /// supported decoding:
    /// - `LZW`
    /// - `ModernJPEG`
    /// - `Deflate`
    /// - `PackBits``
    pub compression_method: CompressionMethod,
    /// horizontal predictor type
    ///
    /// Allows for more efficient compression
    pub predictor: Predictor,
    /// Jpeg tables
    ///
    /// In case of ModernJPEG compression, the compression infomation _can_ be
    /// in this tag, where it is prepended to chunks before decoding.
    /// still an Arc, because we want to do error sharing.
    pub jpeg_tables: Option<Arc<Vec<u8>>>,
    /// Planar configuration:
    ///
    /// example: RGB
    /// - Chunky: [RGBRGBRGB]
    /// - Planar: [RRR] [GGG] [BBB]
    pub planar_config: PlanarConfiguration,
    /// Chunk type
    ///
    /// Either Strip or Tile
    /// Strip => Some(StripDecoder) && None
    /// Tile => None && Some(TileAttributes)
    pub chunk_type: ChunkType,
    pub strip_decoder: Option<StripDecodeState>,
    pub tile_attributes: Option<TileAttributes>,
}

impl ChunkOpts {
    /// Samples per pixel within chunk.
    ///
    /// In planar config, samples are stored in separate strips/chunks, also called bands.
    ///
    /// Example with `bits_per_sample = [8, 8, 8]` and `PhotometricInterpretation::RGB`:
    /// * `PlanarConfiguration::Chunky` -> 3 (RGBRGBRGB...)
    /// * `PlanarConfiguration::Planar` -> 1 (RRR...) (GGG...) (BBB...)
    pub fn samples_per_pixel(&self) -> usize {
        match self.planar_config {
            PlanarConfiguration::Chunky => self.samples.into(),
            PlanarConfiguration::Planar => 1,
        }
    }
    /// The length of a chunk row in bytes, taking padding into account.
    ///
    pub fn output_row_stride(&self, chunk_index: u32) -> TiffResult<usize> {
        let output_width = self.chunk_data_dimensions(chunk_index)?.0;
        usize::try_from(
            (output_width as u64)
                .saturating_mul(self.samples_per_pixel() as u64)
                .saturating_mul(self.bits_per_sample as u64)
                / 8,
        )
        .map_err(TiffError::from)
    }
    /// dimensions of a chunk, not taking padding into account.
    ///
    /// Can be directly deduced from ChunkType and corresponding data
    pub fn chunk_dimensions(&self) -> TiffResult<(u32, u32)> {
        match self.chunk_type {
            ChunkType::Strip => {
                let strip_attrs = self.strip_decoder.as_ref().unwrap();
                Ok((self.image_width, strip_attrs.rows_per_strip))
            }
            ChunkType::Tile => {
                let tile_attrs = self.tile_attributes.as_ref().unwrap();
                Ok((
                    u32::try_from(tile_attrs.tile_width)?,
                    u32::try_from(tile_attrs.tile_length)?,
                ))
            }
        }
    }

    /// return the dimensions of an expanded chunk, taking into account padding
    /// at the bottom and right side of the file.
    ///
    pub fn chunk_data_dimensions(&self, chunk_index: u32) -> TiffResult<(u32, u32)> {
        let dims = self.chunk_dimensions()?;

        match self.chunk_type {
            ChunkType::Strip => {
                // image ordering in case of planar configuration:
                // > The components are stored in separate “component planes.” The
                // > values in StripOffsets and StripByteCounts are then arranged as a 2-dimensional
                // > array, with SamplesPerPixel rows and StripsPerImage columns. (All of the col-
                // > umns for row 0 are stored first, followed by the columns of row 1, and so on.)
                // > PhotometricInterpretation describes the type of data stored in each component
                // > plane. For example, RGB data is stored with the Red components in one compo-
                // > nent plane, the Green in another, and the Blue in another.
                // so:
                // spp
                // ^
                // |
                // +--> chunks
                //       ___col0___________col2_______________colN______
                // row1 | Chunk0[RED]  , Chunk1[RED]  , ... ChunkN[RED]
                // row2 | Chunk0[GREEN], Chunk1[GREEN], ... ChunkN[GREEN]
                // row3 | Chunk0[BLUE] , Chunk1[BLUE] , ... ChunkN[BLUE]
                // "in memory": [Chunk1[RED],Chunk2[RED],...ChunkN[RED],Chunk1[GREEN]...]
                // let's say we have a 42x42 RGBA image with 8 rows_per_chunk
                // that's ceil(42/8)=6 strips_per_band, where the last chunk has 2 rows
                let strip_attrs = self.strip_decoder.as_ref().unwrap();
                // follow through, where we want to get chunk 5
                let strips_per_band = // the N of ChunkN
                    self.image_height.div_ceil(strip_attrs.rows_per_strip);
                let strip_height_without_padding = (chunk_index % strips_per_band)// 5
                    .checked_mul(dims.1)// 5*8=40
                    .and_then(|x| self.image_height.checked_sub(x)) // 2
                    .ok_or(TiffError::UsageError(UsageError::InvalidChunkIndex(
                        chunk_index,
                    )))?;

                // Ignore potential vertical padding on the bottommost strip
                let strip_height = dims.1.min(strip_height_without_padding);

                Ok((dims.0, strip_height))
            }
            ChunkType::Tile => {
                let tile_attrs = self.tile_attributes.as_ref().unwrap();
                let (padding_right, padding_down) = tile_attrs.get_padding(chunk_index as usize);

                let tile_width = tile_attrs.tile_width - padding_right;
                let tile_length = tile_attrs.tile_length - padding_down;

                Ok((u32::try_from(tile_width)?, u32::try_from(tile_length)?))
            }
        }
    }

    /// Derive colortype from info
    ///
    /// ## TODO: fix:
    /// - RGB++
    /// - TransparencyMask
    /// - [CIELab](https://en.wikipedia.org/wiki/CIELAB_color_space)
    pub fn colortype(&self) -> TiffResult<ColorType> {
        match self.photometric_interpretation {
            PhotometricInterpretation::RGB => match self.samples {
                3 => Ok(ColorType::RGB(self.bits_per_sample)),
                4 => Ok(ColorType::RGBA(self.bits_per_sample)),
                // FIXME: We should _ignore_ other components. In particular:
                // > Beware of extra components. Some TIFF files may have more components per pixel
                // than you think. A Baseline TIFF reader must skip over them gracefully,using the
                // values of the SamplesPerPixel and BitsPerSample fields.
                // > -- TIFF 6.0 Specification, Section 7, Additional Baseline requirements.
                _ => Err(TiffError::UnsupportedError(
                    TiffUnsupportedError::InterpretationWithBits(
                        self.photometric_interpretation,
                        vec![self.bits_per_sample; self.samples as usize],
                    ),
                )),
            },
            PhotometricInterpretation::CMYK => match self.samples {
                4 => Ok(ColorType::CMYK(self.bits_per_sample)),
                _ => Err(TiffError::UnsupportedError(
                    TiffUnsupportedError::InterpretationWithBits(
                        self.photometric_interpretation,
                        vec![self.bits_per_sample; self.samples as usize],
                    ),
                )),
            },
            PhotometricInterpretation::YCbCr => match self.samples {
                3 => Ok(ColorType::YCbCr(self.bits_per_sample)),
                _ => Err(TiffError::UnsupportedError(
                    TiffUnsupportedError::InterpretationWithBits(
                        self.photometric_interpretation,
                        vec![self.bits_per_sample; self.samples as usize],
                    ),
                )),
            },
            PhotometricInterpretation::BlackIsZero | PhotometricInterpretation::WhiteIsZero => {
                match self.samples {
                    1 => Ok(ColorType::Gray(self.bits_per_sample)),
                    _ => Ok(ColorType::Multiband {
                        bit_depth: self.bits_per_sample,
                        num_samples: self.samples,
                    }),
                }
            }
            // TODO: this is bad we should not fail at this point
            PhotometricInterpretation::RGBPalette
            | PhotometricInterpretation::TransparencyMask
            | PhotometricInterpretation::CIELab => Err(TiffError::UnsupportedError(
                TiffUnsupportedError::InterpretationWithBits(
                    self.photometric_interpretation,
                    vec![self.bits_per_sample; self.samples as usize],
                ),
            )),
        }
    }
}

// pub enum MaybePartial {
//     Whole(BufferedEntry),
//     Partial {
//         // tag_type: TagType,
//         offset: u64,
//         chunk_size: usize,
//         data: Arc<RwLock<HashMap<u64, BufferedEntry>>>,
//         pending_chunks: Arc<Mutex<HashMap<u64, Condvar>>>,
//     },
// }

// pub enum MaybePartialIndex<T> {
//     Ok(T),
//     NeedRead {
//         offset: u64,
//         count: u64,
//         buf: Vec<u8>,
//     },
//     Pending(Condvar),
// }

// impl MaybePartial {
//     fn get_u64(&self, index: usize) -> TiffResult<MaybePartialIndex<u64>> {
//         match self {
//             MaybePartial::Whole(e) => Ok(MaybePartialIndex::Ok(e.get_u64(index)?)),
//             MaybePartial::Partial {
//                 offset,
//                 chunk_size,
//                 data,
//                 pending_chunks,
//             } => {
//                 let i_chunk: usize = index / chunk_size;
//                 let subindex: usize = index % chunk_size;
//                 if let Some(entry) = data.try_read()?.get(&i_chunk.try_into()?) {
//                     Ok(MaybePartialIndex::Ok(entry.get_u64(subindex)?))
//                 } else {
//                     if let Some(cv) = pending_chunks.try_lock()?.get(&i_chunk.try_into()?) {
//                         Ok(MaybePartialIndex::Pending(cv.clone()))
//                     } else {
//                         pending_chunks
//                             .try_lock()?
//                             .insert(i_chunk.try_into()?, Condvar::new());
//                         Ok(MaybePartialIndex::NeedRead {
//                             offset: *offset,
//                             count: u64::try_from(*chunk_size)?,
//                             buf: vec![0u8; *chunk_size],
//                         })
//                     }
//                 }
//             }
//         }
//     }
// }

/// Image struct that holds all relevant metadata for locating an image's data in the file and which decoding method to use
#[derive(PartialEq, Clone)]
pub struct Image {
    /// IFD holding all data
    pub ifd: Ifd,
    /// Data that doesn't change between chunks
    pub chunk_opts: Arc<ChunkOpts>,
    /// Chunk offsets (maybe partially loaded)
    pub chunk_offsets: Vec<u64>,
    // Number of bytes per chunk (maybe partially loaded)
    pub chunk_bytes: Vec<u64>,
}

impl Debug for Image {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Image")
            .field("ifd", &self.ifd)
            .field("chunk_opts", &self.chunk_opts)
            .field(
                "chunk_offsets",
                &&self.chunk_offsets[..if self.chunk_offsets.len() < 32 {
                    self.chunk_offsets.len()
                } else {
                    16
                }],
            )
            .field(
                "chunk_bytes",
                &&self.chunk_bytes[..if self.chunk_bytes.len() < 32 {
                    self.chunk_offsets.len()
                } else {
                    16
                }],
            )
            .finish()
    }
}

const REQUIRED_TAGS: [Tag; 3] = [
    Tag::ImageWidth,                // fits in offset (1  SHORT or LONG)
    Tag::ImageLength,               // fits in offset (1 SHORT or LONG)
    Tag::PhotometricInterpretation, // fits in offset (1 SHORT)
];
const OPTIONAL_TAGS: [Tag; 7] = [
    Tag::BitsPerSample,       // may not fit  (SamplesPerPixel SHORT)
    Tag::SamplesPerPixel,     // fits in offset (1 SHORT)
    Tag::SampleFormat,        // may not fit SamplesPerPixel SHORT
    Tag::Compression,         // fits (1 SHORT)
    Tag::Predictor,           // fits (1 SHORT)
    Tag::PlanarConfiguration, // fits (1 SHORT)
    Tag::JPEGTables,          // may not fit
];
// these we special-case:
// - Tag::StripByteCounts,
// - Tag::StripOffsets,
//   - Tag::RowsPerStrip (optional)
// - Tag::TileByteCounts,
// - Tag::TileOffsets,
//   - Tag::TileWidth
//   - Tag::TileLenght

impl Image {
    // pub fn chunk_offsets(&self) -> &BufferedEntry {
    //     match self.
    // }

    /// offset in file of chunk
    pub fn chunk_offset(&self, index: usize) -> TiffResult<&u64> {
        self.chunk_offsets
            .get(index)
            .ok_or(TiffError::LimitsExceeded)
    }
    /// number of (compressed) bytes of chunk
    pub fn chunk_bytes(&self, index: usize) -> TiffResult<&u64> {
        self.chunk_bytes.get(index).ok_or(TiffError::LimitsExceeded)
    }

    /// the range within the file where the compressed chunk bytes are
    pub fn chunk_file_range(&self, index: usize) -> TiffResult<Range<u64>> {
        Ok(*self.chunk_offset(index)?..*self.chunk_offset(index)? + self.chunk_bytes(index)?)
    }

    /// get [`ChunkOpts`] of image
    pub fn chunk_opts(&self) -> Arc<ChunkOpts> {
        self.chunk_opts.clone()
    }
    /// check if the given IFD can be made into an image Ifd
    ///
    /// returns a dictionary of tags that are present, but whose values need to
    /// be loaded from the given offsets.  
    /// Doesn't check for tag values, only for presence/absence conflicts in tags
    ///
    /// TODO: check which tags _always_ - by the spec - fit inside the offset field.
    pub fn check_ifd(ifd: &Ifd) -> TiffResult<BTreeMap<Tag, Offset>> {
        let mut res = BTreeMap::<Tag, Offset>::new();

        // required tags: these need to be present, otherwise we're not an Image
        // - ImageWidth
        // - ImageLength
        // - PhotometricInterpretation
        for tag in REQUIRED_TAGS {
            if let IfdEntry::Offset(o) = ifd.require_tag(&tag)? {
                res.insert(tag, *o);
            }
        }
        let image_height = u32::try_from(ifd.require_tag_value(&Tag::ImageLength)?)?;
        let image_width = u32::try_from(ifd.require_tag_value(&Tag::ImageWidth)?)?;
        if image_width == 0 || image_height == 0 {
            return Err(TiffFormatError::InvalidDimensions(image_width, image_height).into());
        }
        if PhotometricInterpretation::from_u16(
            ifd.require_tag_value(&Tag::PhotometricInterpretation)?
                .try_into()?,
        )
        .is_none()
        {
            return Err(TiffUnsupportedError::UnknownInterpretation.into());
        };

        // optional tags: These we can supply with a default value if not present
        // - Compression: None=no compression
        //   - JPEGTables: if CompressionMethod = ModernJPEG, still check if we
        //     didn't decode CompressionMethod
        // - SamplesPerPixel: None = 1
        // - Predictor: None = Predictor::None
        // - PlanarConfiguration: None = PlanarConfiguration::Chunky
        // - SampleFormat: None = SampleFormat::UInt
        // - BitsPerSample: None = vec![1]
        for tag in OPTIONAL_TAGS {
            if let Some(IfdEntry::Offset(o)) = ifd.get_tag(&tag) {
                res.insert(tag, *o);
            }
        }

        // Special-case chunk tags
        match (
            ifd.contains_key(&Tag::StripByteCounts),
            ifd.contains_key(&Tag::StripOffsets),
            ifd.contains_key(&Tag::TileByteCounts),
            ifd.contains_key(&Tag::TileOffsets),
        ) {
            (true, true, false, false) => {
                if let IfdEntry::Offset(o) = ifd.get_tag(&Tag::StripByteCounts).unwrap() {
                    res.insert(Tag::StripByteCounts, *o);
                }
                if let IfdEntry::Offset(o) = ifd.get_tag(&Tag::StripOffsets).unwrap() {
                    res.insert(Tag::StripOffsets, *o);
                }
            }
            (false, false, true, true) => {
                if let IfdEntry::Offset(o) = ifd.get_tag(&Tag::TileByteCounts).unwrap() {
                    res.insert(Tag::TileByteCounts, *o);
                }
                if let IfdEntry::Offset(o) = ifd.get_tag(&Tag::TileOffsets).unwrap() {
                    res.insert(Tag::TileOffsets, *o);
                }
                if let IfdEntry::Offset(o) = ifd.require_tag(&Tag::TileWidth)? {
                    res.insert(Tag::TileWidth, *o);
                }
                if let IfdEntry::Offset(o) = ifd.require_tag(&Tag::TileLength)? {
                    res.insert(Tag::TileLength, *o);
                }
            }
            _ => {
                return Err(TiffFormatError::StripTileTagConflict.into());
            }
        }
        Ok(res)
    }

    /// Create this image from the IFD.  
    ///
    /// will remove fast-access values from the Directory:
    /// - `ImageWidth`
    /// - `ImageLength`
    /// - `PhotometricInterpretation`
    /// - `Compression`: `None` = no compression
    ///   - `JPEGTables`
    /// - `SamplesPerPixel`: None = 1
    /// - `Predictor`: `None` = Predictor::None
    /// - `PlanarConfiguration`: `None` = PlanarConfiguration::Chunky
    /// - `SampleFormat`: `None` = SampleFormat::UInt
    /// - `BitsPerSample`: `None` = vec![1]
    pub fn from_ifd(mut ifd: Ifd, byte_order: ByteOrder) -> TiffResult<Image> {
        let image_width: u32 = ifd.remove_required_val(&Tag::ImageWidth)?.try_into()?;
        let image_height: u32 = ifd.remove_required_val(&Tag::ImageLength)?.try_into()?;
        if image_width == 0 || image_height == 0 {
            return Err(TiffError::FormatError(TiffFormatError::InvalidDimensions(
                image_width,
                image_height,
            )));
        }

        let photometric_interpretation = PhotometricInterpretation::from_u16(
            ifd.remove_required_val(&Tag::PhotometricInterpretation)?
                .try_into()?,
        )
        .ok_or(TiffUnsupportedError::UnknownInterpretation)?;

        // Try to parse both the compression method and the number, format, and bits of the included samples.
        // If they are not explicitly specified, those tags are reset to their default values and not carried from previous images.
        let compression_method = match ifd.remove_optional_val(&Tag::Compression)? {
            Some(val) => CompressionMethod::from_u16_exhaustive(u16::try_from(val)?),
            None => CompressionMethod::None,
        };

        let samples: u16 = ifd
            .remove_optional_val(&Tag::SamplesPerPixel)?
            .map(u16::try_from)
            .transpose()?
            .unwrap_or(1);
        if samples == 0 {
            return Err(TiffFormatError::SamplesPerPixelIsZero.into());
        }

        let predictor = ifd
            .remove_optional_val(&Tag::Predictor)?
            .map(u16::try_from)
            .transpose()?
            .map(|p| {
                Predictor::from_u16(p)
                    .ok_or(TiffError::FormatError(TiffFormatError::UnknownPredictor(p)))
            })
            .transpose()?
            .unwrap_or(Predictor::None);

        let planar_config = ifd
            .remove_optional_val(&Tag::PlanarConfiguration)?
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
            // we already checked for presence
            let vec: Vec<u8> = ifd.remove_required_val(&Tag::JPEGTables)?.try_into()?;
            if vec.len() < 2 {
                return Err(TiffError::FormatError(
                    TiffFormatError::InvalidTagValueType(Tag::JPEGTables.to_u16()),
                ));
            }

            Some(Arc::new(vec))
        } else {
            None
        };

        let sample_format = match ifd.remove_optional_val(&Tag::SampleFormat)? {
            Some(e) => {
                let sample_format: Vec<_> = <&[u16]>::try_from(&e)?
                    .iter()
                    .map(|v| SampleFormat::from_u16_exhaustive(*v))
                    .collect();

                // TODO: for now, only homogenous formats across samples are supported.
                if !sample_format.windows(2).all(|s| s[0] == s[1]) {
                    return Err(TiffUnsupportedError::UnsupportedSampleFormat(sample_format).into());
                }

                sample_format[0]
            }
            None => SampleFormat::Uint,
        };

        let bits_per_sample: Vec<u8> = ifd
            .remove_optional_val(&Tag::BitsPerSample)?
            .map(<Vec<u8>>::try_from)
            .transpose()?
            .unwrap_or_else(|| vec![1]);

        // Technically bits_per_sample.len() should be *equal* to samples, but libtiff also allows
        // it to be a single value that applies to all samples.
        if bits_per_sample.len() != usize::from(samples) && bits_per_sample.len() != 1 {
            return Err(TiffFormatError::InconsistentSizesEncountered(TagData::from(
                bits_per_sample,
            ))
            .into());
        }

        // This library (and libtiff) do not support mixed sample formats and zero bits per sample
        // doesn't make sense.
        if bits_per_sample.iter().any(|&b| b != bits_per_sample[0]) || bits_per_sample[0] == 0 {
            return Err(TiffUnsupportedError::InconsistentBitsPerSample(bits_per_sample).into());
        }

        let chunk_type;
        let chunk_offsets: Vec<u64>;
        let chunk_bytes: Vec<u64>;
        let strip_decoder;
        let tile_attributes;
        match (
            ifd.contains_key(&Tag::StripByteCounts),
            ifd.contains_key(&Tag::StripOffsets),
            ifd.contains_key(&Tag::TileByteCounts),
            ifd.contains_key(&Tag::TileOffsets),
        ) {
            (true, true, false, false) => {
                chunk_type = ChunkType::Strip;

                chunk_offsets = ifd.remove_required_val(&Tag::StripOffsets)?.try_into()?;
                chunk_bytes = ifd.remove_required_val(&Tag::StripByteCounts)?.try_into()?;
                let rows_per_strip = ifd
                    .remove_optional_val(&Tag::RowsPerStrip)?
                    .map(u32::try_from)
                    .transpose()?
                    .unwrap_or(image_height);
                strip_decoder = Some(StripDecodeState { rows_per_strip });
                tile_attributes = None;

                if chunk_offsets.len() != chunk_bytes.len()
                    || rows_per_strip == 0
                    || u32::try_from(chunk_offsets.len())?
                        != (image_height.saturating_sub(1) / rows_per_strip + 1) * planes as u32
                {
                    return Err(TiffFormatError::InconsistentSizesEncountered(TagData::from(
                        chunk_offsets,
                    ))
                    .into());
                }
            }
            (false, false, true, true) => {
                chunk_type = ChunkType::Tile;

                let tile_width =
                    usize::try_from(u64::try_from(ifd.remove_required_val(&Tag::TileWidth)?)?)?;
                let tile_length =
                    usize::try_from(u64::try_from(ifd.remove_required_val(&Tag::TileLength)?)?)?;

                if tile_width == 0 {
                    return Err(
                        TiffFormatError::InvalidTagValueType(Tag::TileWidth.to_u16()).into(),
                    );
                } else if tile_length == 0 {
                    return Err(
                        TiffFormatError::InvalidTagValueType(Tag::TileLength.to_u16()).into(),
                    );
                }

                strip_decoder = None;
                tile_attributes = Some(TileAttributes {
                    image_width: usize::try_from(image_width)?,
                    image_height: usize::try_from(image_height)?,
                    tile_width,
                    tile_length,
                });
                chunk_offsets = ifd.remove_required_val(&Tag::TileOffsets)?.try_into()?;
                chunk_bytes = ifd.remove_required_val(&Tag::TileByteCounts)?.try_into()?;

                let tile = tile_attributes.as_ref().unwrap();
                if chunk_offsets.len() != chunk_bytes.len()
                    || chunk_offsets.len()
                        != tile.tiles_down() * tile.tiles_across() * planes as usize
                {
                    return Err(TiffFormatError::InconsistentSizesEncountered(TagData::from(
                        chunk_bytes,
                    ))
                    .into());
                }
            }
            (_, _, _, _) => {
                return Err(TiffFormatError::StripTileTagConflict.into());
            }
        };
        let chunk_opts = Arc::new(ChunkOpts {
            byte_order,
            image_width,
            image_height,
            bits_per_sample: bits_per_sample[0],
            samples,
            sample_format,
            photometric_interpretation,
            compression_method,
            predictor,
            jpeg_tables,
            planar_config,
            chunk_type,
            strip_decoder,
            tile_attributes,
        });
        Ok(Image {
            ifd,
            chunk_opts,
            chunk_offsets,
            chunk_bytes,
        })
    }
}

#[cfg(test)]
mod test {
    use crate::structs::{ifd::Directory, tags::TagType};

    use super::*;
    fn build_dir() -> Directory {
        let mut dir = Directory::new();
        dir.insert(Tag::ImageWidth, IfdEntry::Value(TagData::from(42u32)));
        dir.insert(Tag::ImageLength, IfdEntry::Value(TagData::from(42u32)));
        dir.insert(
            Tag::PhotometricInterpretation,
            IfdEntry::Value(TagData::from(PhotometricInterpretation::RGB.to_u16())),
        );
        dir
    }
    /// build an Ifd that is an image
    fn build_strip_dir() -> Directory {
        let mut dir = build_dir();
        // conditional tags: single strip, no byte counts
        dir.insert(
            Tag::StripByteCounts,
            IfdEntry::Value(TagData::from(vec![42u32 * 42])),
        );
        dir.insert(
            Tag::StripOffsets,
            IfdEntry::Value(TagData::from(vec![42u32])),
        );
        dir
    }
    fn build_tile_dir() -> Directory {
        let mut dir = build_dir();
        dir.insert(
            Tag::TileByteCounts,
            IfdEntry::Value(TagData::from(vec![42u32 * 42])),
        );
        dir.insert(
            Tag::TileOffsets,
            IfdEntry::Value(TagData::from(vec![42u32])),
        );
        dir.insert(Tag::TileLength, IfdEntry::Value(TagData::from(vec![42u32])));
        dir.insert(Tag::TileWidth, IfdEntry::Value(TagData::from(vec![42u32])));
        dir
    }

    #[test]
    fn test_check_ifd_only_req() {
        let dir = build_dir();
        let TiffError::FormatError(e) = Image::check_ifd(&Ifd::from(dir)).unwrap_err() else {
            unreachable!()
        };
        assert_eq!(e, TiffFormatError::StripTileTagConflict);

        assert_eq!(
            BTreeMap::new(),
            Image::check_ifd(&Ifd::from(build_strip_dir())).unwrap()
        );
        assert_eq!(
            BTreeMap::new(),
            Image::check_ifd(&Ifd::from(build_tile_dir())).unwrap()
        );
    }

    #[test]
    fn test_check_ifd_no_req() {
        for req_tag in REQUIRED_TAGS {
            let mut d = build_strip_dir();
            d.remove(&req_tag);
            let TiffError::FormatError(e) = Image::check_ifd(&Ifd::from(d)).unwrap_err() else {
                unreachable!();
            };
            assert_eq!(e, TiffFormatError::RequiredTagNotFound(req_tag))
        }
    }

    // #[test]
    // /// This test should check that - in case we have required tags that are
    // /// actually an offset into the tiff, they get returned as a BTReeMap.
    // /// However, that will not happen,since required tags always fit in the
    // /// offset field.
    // fn test_check_ifd_req_not_loaded() {
    //     for req_tag in REQUIRED_TAGS {
    //         // since all required tags fit within the offset field, they cannot
    //         // be an offset in the tiff
    //         let entry = ProcessedEntry::Long(vec![42u32]);

    //         let mut d = build_strip_dir();
    //         d.insert(req_tag, IfdEntry::Value(entry)).unwrap();

    //         let mut target_d = BTreeMap::new();
    //         target_d.insert(req_tag, );

    //         assert_eq!(target_d, Image::check_ifd(&Ifd::from(d)).unwrap());
    //     }
    // }

    #[test]
    fn test_check_ifd_opt_not_loaded() {
        for opt_tag in OPTIONAL_TAGS {
            let offset = Offset {
                tag_type: TagType::LONG,
                count: 1,
                offset: 42,
            };

            let mut d = build_strip_dir();
            d.insert(opt_tag, IfdEntry::Offset(offset));

            let mut target_d = BTreeMap::new();
            target_d.insert(opt_tag, offset);

            assert_eq!(target_d, Image::check_ifd(&Ifd::from(d)).unwrap());
        }
    }

    #[test]
    fn test_check_ifd_opt_loaded() {
        for opt_tag in OPTIONAL_TAGS {
            // let offset = Offset {tag_type: TagType::LONG, count: 1, offset: 42};

            let mut d = build_strip_dir();
            d.insert(opt_tag, IfdEntry::Value(TagData::from(vec![42u32])));

            assert_eq!(BTreeMap::new(), Image::check_ifd(&Ifd::from(d)).unwrap());
        }
    }

    #[test]
    fn test_check_ifd_strip_not_loaded() {
        let tag_type = TagType::LONG;
        let of_offsets = Offset {
            tag_type,
            count: 1,
            offset: 42,
        };
        let of_bytes = Offset {
            tag_type,
            count: 1,
            offset: of_offsets.offset + tag_type.size() as u64,
        };

        let mut d = build_strip_dir();
        d.insert(Tag::StripOffsets, IfdEntry::Offset(of_offsets));
        d.insert(Tag::StripByteCounts, IfdEntry::Offset(of_bytes));

        let mut tg_dir = BTreeMap::new();
        tg_dir.insert(Tag::StripOffsets, of_offsets);
        tg_dir.insert(Tag::StripByteCounts, of_bytes);

        assert_eq!(tg_dir, Image::check_ifd(&Ifd::from(d)).unwrap());
    }

    /// If we have tiles, TileLength and TileWidth are required
    #[test]
    fn check_ifd_tile_no_req() {
        let reqs = [Tag::TileLength, Tag::TileWidth];
        for req in reqs {
            let mut d = build_tile_dir();
            d.remove(&req).unwrap();
            let TiffError::FormatError(e) = Image::check_ifd(&Ifd::from(d)).unwrap_err() else {
                unreachable!()
            };
            assert_eq!(e, TiffFormatError::RequiredTagNotFound(req));
        }
    }

    #[test]
    fn test_from_ifd_strip_success() {
        let d = build_strip_dir();
        let IfdEntry::Value(ofs) = d[&Tag::StripOffsets].clone() else {
            unreachable!()
        };
        let IfdEntry::Value(bytes) = d[&Tag::StripByteCounts].clone() else {
            unreachable!()
        };
        let byte_order = ByteOrder::LittleEndian;
        let img = Image {
            ifd: Ifd::from(BTreeMap::new()),
            chunk_opts: Arc::new(ChunkOpts {
                byte_order,
                image_width: 42,
                image_height: 42,
                bits_per_sample: 1,
                samples: 1,
                sample_format: SampleFormat::Uint,
                photometric_interpretation: PhotometricInterpretation::RGB,
                compression_method: CompressionMethod::None,
                predictor: Predictor::None,
                jpeg_tables: None,
                planar_config: PlanarConfiguration::Chunky,
                chunk_type: ChunkType::Strip,
                strip_decoder: Some(StripDecodeState { rows_per_strip: 42 }),
                tile_attributes: None,
            }),
            chunk_offsets: ofs.try_into().unwrap(),
            chunk_bytes: bytes.try_into().unwrap(),
        };
        // implicitly checks if entries are removed from ifd
        assert_eq!(img, Image::from_ifd(Ifd::from(d), byte_order).unwrap());
    }

    #[test]
    fn test_from_ifd_tile_success() {
        let d = build_tile_dir();
        let IfdEntry::Value(ofs) = d[&Tag::TileOffsets].clone() else {
            unreachable!()
        };
        let IfdEntry::Value(bytes) = d[&Tag::TileByteCounts].clone() else {
            unreachable!()
        };
        let IfdEntry::Value(tile_length) = d[&Tag::TileLength].clone() else {
            unreachable!()
        };
        let IfdEntry::Value(tile_width) = d[&Tag::TileWidth].clone() else {
            unreachable!()
        };
        let byte_order = ByteOrder::LittleEndian;
        let img = Image {
            ifd: Ifd::from(BTreeMap::new()),
            chunk_opts: Arc::new(ChunkOpts {
                byte_order,
                image_width: 42,
                image_height: 42,
                bits_per_sample: 1,
                samples: 1,
                sample_format: SampleFormat::Uint,
                photometric_interpretation: PhotometricInterpretation::RGB,
                compression_method: CompressionMethod::None,
                predictor: Predictor::None,
                jpeg_tables: None,
                planar_config: PlanarConfiguration::Chunky,
                chunk_type: ChunkType::Tile,
                strip_decoder: None,
                tile_attributes: Some(TileAttributes {
                    image_height: 42,
                    image_width: 42,
                    tile_length: u64::try_from(tile_length).unwrap().try_into().unwrap(),
                    tile_width: u64::try_from(tile_width).unwrap().try_into().unwrap(),
                }),
            }),
            chunk_offsets: ofs.try_into().unwrap(),
            chunk_bytes: bytes.try_into().unwrap(),
        };
        assert_eq!(img, Image::from_ifd(Ifd::from(d), byte_order).unwrap());
    }

    #[test]
    fn test_image_from_ifd_no_width() {
        let mut d = build_strip_dir();
        d.insert(Tag::ImageWidth, IfdEntry::Value(TagData::from(vec![0u32])));
        let TiffError::FormatError(e) =
            Image::from_ifd(Ifd::from(d), ByteOrder::LittleEndian).unwrap_err()
        else {
            unreachable!()
        };
        assert_eq!(e, TiffFormatError::InvalidDimensions(0, 42));
    }

    #[test]
    fn test_image_from_ifd_no_height() {
        let mut d = build_strip_dir();
        d.insert(Tag::ImageLength, IfdEntry::Value(TagData::from(vec![0u32])));
        let TiffError::FormatError(e) =
            Image::from_ifd(Ifd::from(d), ByteOrder::LittleEndian).unwrap_err()
        else {
            unreachable!()
        };
        assert_eq!(e, TiffFormatError::InvalidDimensions(42, 0));
    }

    #[test]
    fn test_image_unknown_photometric() {
        let cases = [build_strip_dir, build_tile_dir];
        for case in cases {
            let mut d = case();
            d.insert(
                Tag::PhotometricInterpretation,
                IfdEntry::Value(TagData::from(vec![42u16])),
            );
            let TiffError::UnsupportedError(e) =
                Image::from_ifd(Ifd::from(d), ByteOrder::LittleEndian).unwrap_err()
            else {
                unreachable!()
            };
            assert_eq!(e, TiffUnsupportedError::UnknownInterpretation);
        }
    }

    // actually we don't err on unknowncompression at this point yet
    // #[test]
    // fn test_image_unknown_compression() {
    //     let cases = [build_strip_dir, build_tile_dir];
    //     for case in cases {
    //         let mut d = case();
    //         d.insert(Tag::Compression, IfdEntry::Value(TagData::Short(vec![42])));
    //         let TiffError::UnsupportedError(e) = Image::from_ifd(Ifd::from(d), ByteOrder::LittleEndian).unwrap_err() else {unreachable!()};
    //         assert_eq!(e, TiffUnsupportedError::UnknownCompressionMethod);
    //     }
    // }

    #[test]
    fn test_image_zero_samples_per_pixel() {
        let cases = [build_strip_dir, build_tile_dir];
        for case in cases {
            let mut d = case();
            d.insert(
                Tag::SamplesPerPixel,
                IfdEntry::Value(TagData::from(vec![0u16])),
            );
            let TiffError::FormatError(e) =
                Image::from_ifd(Ifd::from(d), ByteOrder::LittleEndian).unwrap_err()
            else {
                unreachable!()
            };
            assert_eq!(e, TiffFormatError::SamplesPerPixelIsZero);
        }
    }

    #[test]
    fn test_image_unknown_predictor() {
        let cases = [build_strip_dir, build_tile_dir];
        for case in cases {
            let mut d = case();
            d.insert(Tag::Predictor, IfdEntry::Value(TagData::from(vec![42u16])));
            let TiffError::FormatError(e) =
                Image::from_ifd(Ifd::from(d), ByteOrder::LittleEndian).unwrap_err()
            else {
                unreachable!()
            };
            assert_eq!(e, TiffFormatError::UnknownPredictor(42));
        }
    }

    #[test]
    fn test_image_unknown_planar_config() {
        let cases = [build_strip_dir, build_tile_dir];
        for case in cases {
            let mut d = case();
            d.insert(
                Tag::PlanarConfiguration,
                IfdEntry::Value(TagData::from(vec![42u16])),
            );
            let TiffError::FormatError(e) =
                Image::from_ifd(Ifd::from(d), ByteOrder::LittleEndian).unwrap_err()
            else {
                unreachable!()
            };
            assert_eq!(e, TiffFormatError::UnknownPlanarConfiguration(42));
        }
    }

    #[test]
    fn test_image_short_jpeg_tables() {
        let cases = [build_strip_dir, build_tile_dir];
        for case in cases {
            let mut d = case();
            d.insert(
                Tag::Compression,
                IfdEntry::Value(TagData::from(CompressionMethod::ModernJPEG.to_u16())),
            );
            d.insert(Tag::JPEGTables, IfdEntry::Value(TagData::from(vec![42u16])));
            let TiffError::FormatError(e) =
                Image::from_ifd(Ifd::from(d), ByteOrder::LittleEndian).unwrap_err()
            else {
                unreachable!()
            };
            assert_eq!(
                e,
                TiffFormatError::InvalidTagValueType(Tag::JPEGTables.to_u16())
            );
        }
    }

    #[test]
    fn test_image_invalid_bits_per_sample_length() {
        let cases = [build_strip_dir, build_tile_dir];
        for case in cases {
            let mut d = case();
            let spp = TagData::from(vec![2u8, 4]);
            d.insert(Tag::BitsPerSample, IfdEntry::Value(spp.clone()));
            let TiffError::FormatError(e) =
                Image::from_ifd(Ifd::from(d), ByteOrder::LittleEndian).unwrap_err()
            else {
                unreachable!()
            };
            assert_eq!(e, TiffFormatError::InconsistentSizesEncountered(spp));
        }
    }

    #[test]
    fn test_image_incoherent_bits_per_sample_length() {
        let cases = [build_strip_dir, build_tile_dir];
        for case in cases {
            let mut d = case();
            let bits_per_sample = TagData::from(vec![8u8; 4]);
            let samples_per_pixel = TagData::from(vec![3u16]);
            d.insert(Tag::BitsPerSample, IfdEntry::Value(bits_per_sample.clone()));
            d.insert(Tag::SamplesPerPixel, IfdEntry::Value(samples_per_pixel));
            println!(
                "bps: {:?} spp: {:?}",
                d[&Tag::BitsPerSample],
                d[&Tag::SamplesPerPixel]
            );
            match Image::from_ifd(Ifd::from(d), ByteOrder::LittleEndian).unwrap_err() {
                TiffError::FormatError(e) => {
                    assert_eq!(
                        e,
                        TiffFormatError::InconsistentSizesEncountered(bits_per_sample)
                    );
                }
                e => {
                    panic!("unexpected error {:?}", e);
                }
            };
        }
    }

    #[test]
    fn test_image_inconsistent_bits_per_sample() {
        let cases = [build_strip_dir, build_tile_dir];
        for case in cases {
            let mut d = case();
            let bits_per_sample: Vec<u8> = vec![2, 4, 8];
            let samples_per_pixel = TagData::from(vec![3u16]);
            d.insert(
                Tag::BitsPerSample,
                IfdEntry::Value(TagData::from(bits_per_sample.clone())),
            );
            d.insert(Tag::SamplesPerPixel, IfdEntry::Value(samples_per_pixel));
            println!(
                "bps: {:?} spp: {:?}",
                d[&Tag::BitsPerSample],
                d[&Tag::SamplesPerPixel]
            );
            match Image::from_ifd(Ifd::from(d), ByteOrder::LittleEndian).unwrap_err() {
                TiffError::UnsupportedError(e) => {
                    assert_eq!(
                        e,
                        TiffUnsupportedError::InconsistentBitsPerSample(bits_per_sample)
                    );
                }
                e => {
                    panic!("unexpected error {:?}", e);
                }
            };
        }
    }

    #[test]
    fn test_image_inconsistent_bits_per_sample_zero() {
        let cases = [build_strip_dir, build_tile_dir];
        for case in cases {
            let mut d = case();
            let bits_per_sample: Vec<u8> = vec![0, 0, 0];
            let samples_per_pixel = TagData::from(vec![3u16]);
            d.insert(
                Tag::BitsPerSample,
                IfdEntry::Value(TagData::from(bits_per_sample.clone())),
            );
            d.insert(Tag::SamplesPerPixel, IfdEntry::Value(samples_per_pixel));
            println!(
                "bps: {:?} spp: {:?}",
                d[&Tag::BitsPerSample],
                d[&Tag::SamplesPerPixel]
            );
            match Image::from_ifd(Ifd::from(d), ByteOrder::LittleEndian).unwrap_err() {
                TiffError::UnsupportedError(e) => {
                    assert_eq!(
                        e,
                        TiffUnsupportedError::InconsistentBitsPerSample(bits_per_sample)
                    );
                }
                e => {
                    panic!("unexpected error {:?}", e);
                }
            };
        }
    }

    // from buffer using chatgpt: "write me an IFD (byte buffer) for the
    // following cases (little-endian): // above functions"
    fn build_dir_buffer() -> Vec<u8> {
        let mut buffer = Vec::new();

        // Number of entries (2 bytes)
        buffer.extend_from_slice(&(3u16).to_le_bytes());

        // Entry: ImageWidth (tag 0x0100, type LONG, count 1, value 42)
        buffer.extend_from_slice(&Tag::ImageWidth.to_u16().to_le_bytes()); // Tag
        buffer.extend_from_slice(&TagType::LONG.to_u16().to_le_bytes()); // Type (LONG)
        buffer.extend_from_slice(&1u32.to_le_bytes()); // Count
        buffer.extend_from_slice(&42u32.to_le_bytes()); // Value

        // Entry: ImageLength (tag 0x0101, type LONG, count 1, value 42)
        buffer.extend_from_slice(&Tag::ImageLength.to_u16().to_le_bytes()); // Tag
        buffer.extend_from_slice(&TagType::LONG.to_u16().to_le_bytes()); // Type (LONG)
        buffer.extend_from_slice(&1u32.to_le_bytes()); // Count
        buffer.extend_from_slice(&42u32.to_le_bytes()); // Value

        // Entry: PhotometricInterpretation (tag 0x0106, type SHORT, count 1, value 2)
        buffer.extend_from_slice(&Tag::PhotometricInterpretation.to_u16().to_le_bytes()); // Tag
        buffer.extend_from_slice(&TagType::SHORT.to_u16().to_le_bytes()); // Type (SHORT)
        buffer.extend_from_slice(&1u32.to_le_bytes()); // Count
        buffer.extend_from_slice(&PhotometricInterpretation::RGB.to_u16().to_le_bytes()); // Value (RGB)
        buffer.extend_from_slice(&0u16.to_le_bytes()); // Padding for 4-byte alignment

        // Next IFD offset (4 bytes, end of directory so 0)
        // buffer.extend_from_slice(&0u32.to_le_bytes());

        buffer
    }

    fn build_strip_dir_buffer() -> Vec<u8> {
        let mut buffer = build_dir_buffer();

        // Update the number of entries to 5 (previous entries + 2 new ones)
        buffer[0..2].copy_from_slice(&5u16.to_le_bytes());

        // Entry: StripByteCounts (tag 0x0117, type LONG, count 1, value 1764)
        buffer.extend_from_slice(&Tag::StripByteCounts.to_u16().to_le_bytes()); // Tag
        buffer.extend_from_slice(&TagType::LONG.to_u16().to_le_bytes()); // Type (LONG)
        buffer.extend_from_slice(&1u32.to_le_bytes()); // Count
        buffer.extend_from_slice(&(42u32 * 42 * 3).to_le_bytes()); // Value (1764)

        // Entry: StripOffsets (tag 0x0111, type LONG, count 1, value 42)
        buffer.extend_from_slice(&Tag::StripOffsets.to_u16().to_le_bytes()); // Tag
        buffer.extend_from_slice(&TagType::LONG.to_u16().to_le_bytes()); // Type (LONG)
        buffer.extend_from_slice(&1u32.to_le_bytes()); // Count
        buffer.extend_from_slice(&42u32.to_le_bytes()); // Value

        // Next IFD offset (4 bytes, end of directory so 0)
        buffer.extend_from_slice(&0u32.to_le_bytes());

        buffer
    }

    fn build_tile_dir_buffer() -> Vec<u8> {
        let mut buffer = build_dir_buffer();

        // Update the number of entries to 7 (previous entries + 4 new ones)
        buffer[0..2].copy_from_slice(&7u16.to_le_bytes());

        // Entry: TileByteCounts (tag 0x0145, type LONG, count 1, value 1764)
        buffer.extend_from_slice(&Tag::TileByteCounts.to_u16().to_le_bytes()); // Tag
        buffer.extend_from_slice(&TagType::LONG.to_u16().to_le_bytes()); // Type (LONG)
        buffer.extend_from_slice(&1u32.to_le_bytes()); // Count
        buffer.extend_from_slice(&(42u32 * 42 * 3).to_le_bytes()); // Value (1764)

        // Entry: TileOffsets (tag 0x0144, type LONG, count 1, value 42)
        buffer.extend_from_slice(&Tag::TileOffsets.to_u16().to_le_bytes()); // Tag
        buffer.extend_from_slice(&TagType::LONG.to_u16().to_le_bytes()); // Type (LONG)
        buffer.extend_from_slice(&1u32.to_le_bytes()); // Count
        buffer.extend_from_slice(&42u32.to_le_bytes()); // Value

        // Entry: TileLength (tag 0x0143, type LONG, count 1, value 42)
        buffer.extend_from_slice(&Tag::TileLength.to_u16().to_le_bytes()); // Tag
        buffer.extend_from_slice(&TagType::LONG.to_u16().to_le_bytes()); // Type (LONG)
        buffer.extend_from_slice(&1u32.to_le_bytes()); // Count
        buffer.extend_from_slice(&42u32.to_le_bytes()); // Value

        // Entry: TileWidth (tag 0x0142, type LONG, count 1, value 42)
        buffer.extend_from_slice(&Tag::TileWidth.to_u16().to_le_bytes()); // Tag
        buffer.extend_from_slice(&TagType::LONG.to_u16().to_le_bytes()); // Type (LONG)
        buffer.extend_from_slice(&1u32.to_le_bytes()); // Count
        buffer.extend_from_slice(&42u32.to_le_bytes()); // Value

        // Next IFD offset (4 bytes, end of directory so 0)
        buffer.extend_from_slice(&0u32.to_le_bytes());

        buffer
    }

    #[test]
    fn test_image_from_buffer_tile_notbig() {
        let buf = build_tile_dir_buffer();
        let byte_order = ByteOrder::LittleEndian;
        let (ifd, next) =
            Ifd::from_buffer(&buf, byte_order, false).expect("Could not build ifd from buffer");
        assert_eq!(
            Image::check_ifd(&ifd).expect("not a valid ifd"),
            BTreeMap::new()
        );
        let res_img = Image::from_ifd(ifd, byte_order).expect("Could not build image frim ifd");
        let tg_img = Image {
            ifd: Ifd::from(BTreeMap::new()),
            chunk_opts: Arc::new(ChunkOpts {
                byte_order,
                image_width: 42,
                image_height: 42,
                bits_per_sample: 1,
                samples: 1,
                sample_format: SampleFormat::Uint,
                photometric_interpretation: PhotometricInterpretation::RGB,
                compression_method: CompressionMethod::None,
                predictor: Predictor::None,
                jpeg_tables: None,
                planar_config: PlanarConfiguration::Chunky,
                chunk_type: ChunkType::Tile,
                strip_decoder: None,
                tile_attributes: Some(TileAttributes {
                    image_height: 42,
                    image_width: 42,
                    tile_length: 42,
                    tile_width: 42,
                }),
            }),
            chunk_offsets: vec![42],
            chunk_bytes: vec![42 * 42 * 3],
        };
        assert_eq!(res_img, tg_img);
        assert_eq!(next, 0);
    }

    #[test]
    fn test_image_from_buffer_strip_notbig() {
        let buf = build_strip_dir_buffer();
        let byte_order = ByteOrder::LittleEndian;
        let (ifd, next) =
            Ifd::from_buffer(&buf, byte_order, false).expect("Could not build ifd from buffer");
        assert_eq!(
            Image::check_ifd(&ifd).expect("not a valid ifd"),
            BTreeMap::new()
        );
        let res_img = Image::from_ifd(ifd, byte_order).expect("Could not build image frim ifd");
        let tg_img = Image {
            ifd: Ifd::from(BTreeMap::new()),
            chunk_opts: Arc::new(ChunkOpts {
                byte_order,
                image_width: 42,
                image_height: 42,
                bits_per_sample: 1,
                samples: 1,
                sample_format: SampleFormat::Uint,
                photometric_interpretation: PhotometricInterpretation::RGB,
                compression_method: CompressionMethod::None,
                predictor: Predictor::None,
                jpeg_tables: None,
                planar_config: PlanarConfiguration::Chunky,
                chunk_type: ChunkType::Strip,
                strip_decoder: Some(StripDecodeState { rows_per_strip: 42 }),
                tile_attributes: None,
            }),
            chunk_offsets: vec![42],
            chunk_bytes: vec![42 * 42 * 3],
        };
        assert_eq!(res_img, tg_img);
        assert_eq!(next, 0);
    }
}
