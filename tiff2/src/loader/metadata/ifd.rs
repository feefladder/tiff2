use std::collections::{BTreeMap, HashMap};
use std::fmt::format;
use std::ops::Range;
use std::sync::Arc;

use derive_more::{Display, Error};
use exn::{bail, ensure, OptionExt, Result, ResultExt};

use crate::loader::metadata::error::TiffLoadError;
use crate::loader::metadata::IfdLoadResponse;
use crate::loader::{TiffExtLoader, TiffExtLoaderRegistry, TiffLoadResult};
use crate::structs::error::{BUF_CHECK, VMATCH};
use crate::structs::{
    entry_size, num_entries_size, offset_size, offset_tag_type, Ifd, IfdEntry, Offset, Tag,
    TagData, TagType,
};
use crate::ByteOrder;

#[derive(Debug, Default)]
#[non_exhaustive]
pub struct IfdLoader {
    pub bigtiff: bool,
    pub byte_order: ByteOrder,
    pub tags: BTreeMap<Tag, TagData>,
    pub tag_offsets: BTreeMap<Tag, Offset>,
    pub extension_loaders: Vec<Box<dyn TiffExtLoader>>,
    pub extension_tags: BTreeMap<u16, usize>,
    pub next_ifd_offset: Option<u64>,
}

#[derive(Debug, Display, Error, Clone, PartialEq)]
pub enum IfdLoadError {
    #[display("Invalid buffer, need {required:?}")]
    InvalidBuffer {
        required: Range<u64>,
    },
    Permanent {
        message: String,
    },
}

fn invalid_buffer(required: Range<u64>) -> IfdLoadError {
    IfdLoadError::InvalidBuffer { required }
}

fn permanent(message: String) -> IfdLoadError {
    IfdLoadError::Permanent { message }
}

impl IfdLoader {
    pub fn count(&self) -> usize {
        self.tags.len() + self.tag_offsets.len() + self.extension_tags.len()
    }

    /// given a buffer holding the count value, get the number of entries
    fn ifd_entry_count(
        buf: &[u8],
        offset: u64,
        bigtiff: bool,
        byte_order: ByteOrder,
    ) -> Result<u64, IfdLoadError> {
        let count: u64 = TagData::from_buffer(
            buf,
            if bigtiff {
                TagType::LONG8
            } else {
                TagType::SHORT
            },
            1,
            byte_order,
        )
        .or_raise(|| invalid_buffer(offset..offset + num_entries_size(bigtiff)))?
        .try_into()
        .unwrap();
        Ok(count)
    }

    pub fn wrap(
        ifd: Ifd,
        bigtiff: bool,
        byte_order: ByteOrder,
        next_ifd_offset: Option<u64>,
        extension_registry: Arc<TiffExtLoaderRegistry>,
    ) -> Self {
        let (extension_loaders, extension_tags) = extension_registry.build();
        Self {
            bigtiff,
            byte_order,
            tags: ifd.tags,
            tag_offsets: ifd.tag_offsets,
            // Here it's sad that extension_loaders are re-building extension tags by construction
            extension_loaders,
            extension_tags,
            next_ifd_offset,
        }
    }

    /// Given a buffer holding the IFD, get the underlying IFD
    ///
    /// This also reads the count of the ifd (first value). The exact required
    /// size of this buffer cannot be known beforehand, but a (very) safe assumption is
    /// ~1KiB. If it fails, it will give the exact required range.
    ///
    /// ```
    /// let buf = [
    ///     2,0
    ///
    ///     1,0,
    ///     3,0,
    ///     1,0,0,0,
    ///     42,0,0,0,
    ///
    ///     0x44,1,
    ///     4,0,
    ///     2,0,0,0,
    ///     42,0,0,0,
    ///
    ///     0,0,0,0,
    /// ];
    /// let (loader, next) = IfdLoader::from_buffer(&buf, 0, false, ByteOrder::LittleEndian);
    /// ```
    pub fn from_buffer(
        ifd_buf: &[u8],
        offset: u64,
        bigtiff: bool,
        byte_order: ByteOrder,
        extension_registry: Arc<TiffExtLoaderRegistry>,
    ) -> Result<Self, IfdLoadError> {
        let entry_count = Self::ifd_entry_count(ifd_buf, offset, bigtiff, byte_order)?;

        // check if the entire ifd is in memory
        ensure!(
            u64::try_from(ifd_buf.len()).unwrap()
                >= entry_count * entry_size(bigtiff) + offset_size(bigtiff),
            invalid_buffer(
                offset
                    ..offset
                        + entry_count * entry_size(bigtiff)
                        + offset_size(bigtiff)
                        + num_entries_size(bigtiff),
            )
        );
        let mut pos = num_entries_size(bigtiff) as usize;
        let mut ifd_tags = BTreeMap::new();
        let mut tag_offsets = BTreeMap::new();
        // TODO: this should also load in-range tags, so we should already filter tags based on extensions...
        // That should ideally have some semi-ergonomic function
        let (mut extension_loaders, extension_tags) = extension_registry.build();
        // start reading entries
        for _ in 0..entry_count {
            // TODO: This should really become a
            //
            // let (tag, entry) = smart_function(&ifd_buf[pos..], bigtiff, byte_order)
            // pos += ifd_entry_size(bigtiff)
            //
            // after this refactor

            // tag and tag type in a single array
            let tag_ttype_td = TagData::from_buffer(&ifd_buf[pos..], TagType::SHORT, 2, byte_order)
                .expect(BUF_CHECK);
            let tag_ttype = <&[u16]>::try_from(&tag_ttype_td).expect(VMATCH);
            // 2 SHORTs of 2 bytes each
            pos += 2 * 2;
            let tag = Tag::from_u16_exhaustive(tag_ttype[0]);
            let tag_type = TagType::from_u16(tag_ttype[1])
                .ok_or_raise(|| permanent(format!("invalid tag type {}", tag_ttype[1])))?;
            // count
            let value_count: u64 =
                TagData::from_buffer(&ifd_buf[pos..], offset_tag_type(bigtiff), 1, byte_order)
                    // we can unwrap, because we checked buffer size
                    .expect(BUF_CHECK)
                    .try_into()
                    .expect(VMATCH);
            pos += offset_size(bigtiff) as usize; // 8 or 4, coincidentally also offset_size(bigtiff)
            if u64::try_from(tag_type.size()).unwrap() * value_count > offset_size(bigtiff) {
                let o = Offset {
                    tag_type,
                    count: value_count,
                    offset: TagData::from_buffer(
                        &ifd_buf[pos..],
                        offset_tag_type(bigtiff),
                        1,
                        byte_order,
                    )
                    .expect(BUF_CHECK)
                    .try_into()
                    .expect(VMATCH),
                };
                tag_offsets.insert(tag, o);
            } else {
                let td = TagData::from_buffer(
                    &ifd_buf[pos..],
                    tag_type,
                    value_count as usize,
                    byte_order,
                )
                .expect(BUF_CHECK);
                if extension_tags.contains_key(&tag.to_u16()) {
                    // TODO: should this be only on resolved tags, e.g. TagData?
                    // Otherwise, it'd be very very sad with regards to "all places where deferredness lives"
                    // But then again, TiffExtLoaders are there especially for this case...
                    // even though they are thrown out if incomplete when finalizing the ifd...
                    // So that'd kind of mean they'd only have TagData
                    // But that requires some changes with regards to deferred_values_mut...
                    // I think I'd really not like a trait object there
                    // maybe just add a deferred_tags_mut to the TiffExtLoader trait?
                    // Or just tell the ifd: load_tags: BAM
                    extension_loaders[extension_tags[&tag.to_u16()]].insert_tag(tag.to_u16(), td);
                } else {
                    ifd_tags.insert(tag, td);
                }
            }
            pos += offset_size(bigtiff) as usize;
        }
        let next_ifd_offset: u64 =
            TagData::from_buffer(&ifd_buf[pos..], offset_tag_type(bigtiff), 1, byte_order)
                .expect(BUF_CHECK)
                .try_into()
                .expect(VMATCH);

        Ok(Self {
            bigtiff,
            byte_order,
            tags: ifd_tags,
            tag_offsets,
            extension_loaders,
            extension_tags,
            next_ifd_offset: Some(next_ifd_offset),
        })
    }

    /// Get tags that still need to be loaded
    ///
    /// This will always be an offset
    ///
    /// You'd normally use this to fetch/read deferred values
    /// ```
    /// # use std::collections::BTreeMap;
    /// # use smallvec::smallvec;
    /// # use tiff2::ByteOrder;
    /// # use tiff2::structs::{Tag,TagType,IfdEntry,TagData,Offset,Ifd};
    /// # use tiff2::loader::IfdLoader;
    /// # let buf = &[
    /// #     2,0,
    /// #
    /// #     0,1,
    /// #     3,0,
    /// #     1,0,0,0,
    /// #     42,0,0,0,
    /// #
    /// #     0x44,1,
    /// #     4,0,
    /// #     2,0,0,0,
    /// #     42,0,0,0,
    /// #
    /// #     0,0,0,0
    /// # ];
    /// # assert_eq!(buf.len(),30);
    /// # fn fetch(r: Range<u64>) {vec![42;r.len()]}
    /// let mut ifd_loader = IfdLoader::from_buffer(buf,0,false,ByteOrder::LittleEndian).unwrap().0;
    ///
    /// assert_eq!(ifd_loader.deferred_values_mut().collect::<Vec<_>>().len(), 1);
    ///
    /// for (tag, offset) in ifd_loader.to_load().collect::<Vec<_>> {
    ///     # assert_eq!(tag, &Tag::TileOffsets);
    ///     let loaded;
    ///     {
    ///         // fetch the entry when needed
    ///         let data = fetch(offset.range())
    ///         // insert into IfdLoader
    ///         if_loader.load_tag_data(&data, tag);
    ///     }
    ///     *val = IfdEntry::Value(loaded);
    /// }
    /// ```
    pub fn to_load<'a>(&'a self) -> impl Iterator<Item = (Tag, Range<u64>)> + use<'a> {
        self.tag_offsets.iter().map(|(t, o)| (*t, o.range()))
    }

    /// Load data for a single tag into this ifd
    ///
    /// Errors if the tag is not present or the entry count overflows usize (possible on wasm builds)
    pub fn load_tag_data(&mut self, buf: &[u8], tag: Tag) -> Result<u64, IfdLoadError> {
        let offset = self
            .tag_offsets
            .remove(&tag)
            .ok_or_raise(|| permanent(format!("tag {tag:?} not in todo list")))?;
        let data = TagData::from_buffer(
            buf,
            offset.tag_type,
            usize::try_from(offset.count).or_raise(|| {
                permanent(format!(
                    "tag entry count {} for tag {tag:?} overflowed usize",
                    offset.count,
                ))
            })?,
            self.byte_order,
        )
        .or_raise(|| invalid_buffer(offset.range()))?;
        if let Some(ext_idx) = self.extension_tags.get(&tag.to_u16()) {
            self.extension_loaders[*ext_idx].insert_tag(tag.to_u16(), data);
        } else {
            self.tags.insert(tag, data);
        }
        Ok(offset.offset)
    }

    pub fn deferred_ranges<'a>(&'a self) -> impl Iterator<Item = Range<u64>> + use<'a> {
        self.tag_offsets.iter().map(|(_, o)| o.range())
    }

    pub fn deferred_tags<'a>(&'a self) -> impl Iterator<Item = Tag> + use<'a> {
        self.tag_offsets.iter().map(|(t, _)| *t)
    }

    pub fn finish(self) -> Ifd {
        let extensions = self
            .extension_loaders
            .into_iter()
            .filter_map(|l| l.finish().transpose())
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        let ext_typeid_idx = extensions
            .iter()
            .enumerate()
            .map(|(idx, ext)| (ext.type_id(), idx))
            .collect();
        Ifd {
            tags: self.tags,
            tag_offsets: self.tag_offsets,
            extensions,
            ext_tag_idx: self.extension_tags,
            ext_typeid_idx,
        }
    }

    pub(crate) fn to_response(self) -> IfdLoadResponse {
        if self.deferred_ranges().peekable().peek().is_none() {
            IfdLoadResponse::Complete {
                next_ifd_offset: self.next_ifd_offset.unwrap(),
                ifd: self.finish(),
            }
        } else {
            IfdLoadResponse::Partial {
                needed_data: self.deferred_ranges().collect(),
                ifd_loader: self,
            }
        }
    }
}

#[cfg(test)]
mod test {
    use std::default::Default;

    use smallvec::smallvec;

    use super::*;

    impl PartialEq for IfdLoader {
        fn eq(&self, other: &Self) -> bool {
            self.bigtiff == other.bigtiff
                && self.byte_order == other.byte_order
                && self.tags == other.tags
                && self.tag_offsets == other.tag_offsets
                && self.extension_loaders.is_empty()
                && other.extension_loaders.is_empty()
                && self.extension_tags == other.extension_tags
                && self.next_ifd_offset == other.next_ifd_offset
        }
    }

    /// test loading multiple tags, esp. whether we skip over the offset properly
    #[test]
    #[rustfmt::skip]
    fn test_multitag() {
        let cases = [
            //      tag type  count    offset      next ifd
            //      // /  \  /     \   /     \    /     \
            ([2,0, 1,1, 1,0, 1,0,0,0, 42, 0, 0, 0,
                   0,1, 1,0, 1,0,0,0, 43, 0, 0, 0, 0,0,0,0], TagData::Byte(smallvec![42]), TagData::Byte(smallvec![43])),
            ([2,0, 1,1, 4,0, 1,0,0,0, 42, 0, 0, 0,
                   0,1, 4,0, 1,0,0,0, 43, 0, 0, 0, 0,0,0,0], TagData::Long(smallvec![42]), TagData::Long(smallvec![43])),
            ([2,0, 1,1, 9,0, 1,0,0,0, 42, 0, 0, 0,
                   0,1, 9,0, 1,0,0,0, 43, 0, 0, 0, 0,0,0,0], TagData::SLong(smallvec![42]), TagData::SLong(smallvec![43])),
            ([2,0, 1,1, 1,0, 4,0,0,0, 42,42,42,42,
                   0,1, 1,0, 4,0,0,0, 43,43,43,43, 0,0,0,0], TagData::Byte(smallvec![42;4]), TagData::Byte(smallvec![43;4])),
            ([2,0, 1,1, 1,0, 3,0,0,0, 42,42,42, 0,
                   0,1, 9,0, 1,0,0,0, 42, 0, 0, 0, 0,0,0,0], TagData::Byte(smallvec![42;3]), TagData::SLong(smallvec![42])),
        ];
        for (buf, res1, res2) in cases {
            let res = IfdLoader::from_buffer(&buf, 0, false, ByteOrder::LittleEndian, Arc::new(Vec::new().into())).unwrap();
            assert_eq!(&res, &IfdLoader{
                bigtiff: false,
                byte_order: ByteOrder::LittleEndian,
                tags: BTreeMap::from([
                    (Tag::ImageLength, res1),
                    (Tag::ImageWidth, res2)
                ]),
                next_ifd_offset: Some(0),
                ..Default::default()});
        }
    }

    // -----------------------------------------------------------------
    // tests below are copy-pasted from Entry. Make sure to update there
    // accordingly
    // -----------------------------------------------------------------
    #[test]
    #[rustfmt::skip]
    fn test_fits_single_notbig() {
        // personal sanity checks
        assert_eq!(u16::from_le_bytes([42,0]),42);
        assert_eq!(u16::from_be_bytes([0,42]),42);

        assert_eq!(f32::from_le_bytes([0x42,0,0,0]),f32::from_bits(0x00_00_00_42));
        assert_eq!(f32::from_be_bytes([0,0,0,0x42]),f32::from_bits(0x00_00_00_42));
        let cases = [
        // n     tag type  count      offset    next ifd
        //       // /  \  /     \   /        \  /     \
        ([1,0, 1,1, 1, 0, 1,0,0,0, 42, 0, 0, 0, 0,0,0,0], ByteOrder::LittleEndian, TagData::Byte      (smallvec![42])                ),
        ([0,1, 1,1, 0, 1, 0,0,0,1, 42, 0, 0, 0, 0,0,0,0], ByteOrder::BigEndian,    TagData::Byte      (smallvec![42])                ),
        ([1,0, 1,1, 6, 0, 1,0,0,0, 42, 0, 0, 0, 0,0,0,0], ByteOrder::LittleEndian, TagData::SByte     (smallvec![42])                ),
        ([0,1, 1,1, 0, 6, 0,0,0,1, 42, 0, 0, 0, 0,0,0,0], ByteOrder::BigEndian,    TagData::SByte     (smallvec![42])                ),
        ([1,0, 1,1, 7, 0, 1,0,0,0, 42, 0, 0, 0, 0,0,0,0], ByteOrder::LittleEndian, TagData::Undefined (smallvec![42])                ),
        ([0,1, 1,1, 0, 7, 0,0,0,1, 42, 0, 0, 0, 0,0,0,0], ByteOrder::BigEndian,    TagData::Undefined (smallvec![42])                ),
        ([1,0, 1,1, 2, 0, 1,0,0,0,  0, 0, 0, 0, 0,0,0,0], ByteOrder::LittleEndian, TagData::Ascii     (smallvec![0 ])                ),
        ([0,1, 1,1, 0, 2, 0,0,0,1,  0, 0, 0, 0, 0,0,0,0], ByteOrder::BigEndian,    TagData::Ascii     (smallvec![0 ])                ),
        ([1,0, 1,1, 3, 0, 1,0,0,0, 42, 0, 0, 0, 0,0,0,0], ByteOrder::LittleEndian, TagData::Short     (smallvec![42])                ),
        ([0,1, 1,1, 0, 3, 0,0,0,1,  0,42, 0, 0, 0,0,0,0], ByteOrder::BigEndian,    TagData::Short     (smallvec![42])                ),
        ([1,0, 1,1, 8, 0, 1,0,0,0, 42, 0, 0, 0, 0,0,0,0], ByteOrder::LittleEndian, TagData::SShort    (smallvec![42])                ),
        ([0,1, 1,1, 0, 8, 0,0,0,1,  0,42, 0, 0, 0,0,0,0], ByteOrder::BigEndian,    TagData::SShort    (smallvec![42])                ),
        ([1,0, 1,1, 4, 0, 1,0,0,0, 42, 0, 0, 0, 0,0,0,0], ByteOrder::LittleEndian, TagData::Long      (smallvec![42])                ),
        ([0,1, 1,1, 0, 4, 0,0,0,1,  0, 0, 0,42, 0,0,0,0], ByteOrder::BigEndian,    TagData::Long      (smallvec![42])                ),
        ([1,0, 1,1, 9, 0, 1,0,0,0, 42, 0, 0, 0, 0,0,0,0], ByteOrder::LittleEndian, TagData::SLong     (smallvec![42])                ),
        ([0,1, 1,1, 0, 9, 0,0,0,1,  0, 0, 0,42, 0,0,0,0], ByteOrder::BigEndian,    TagData::SLong     (smallvec![42])                ),
        ([1,0, 1,1,13, 0, 1,0,0,0, 42, 0, 0, 0, 0,0,0,0], ByteOrder::LittleEndian, TagData::Ifd       (smallvec![42])                ),
        ([0,1, 1,1, 0,13, 0,0,0,1,  0, 0, 0,42, 0,0,0,0], ByteOrder::BigEndian,    TagData::Ifd       (smallvec![42])                ),
        ([1,0, 1,1,11, 0, 1,0,0,0, 42, 0, 0, 0, 0,0,0,0], ByteOrder::LittleEndian, TagData::Float     (smallvec![f32::from_bits(42)])),
        ([0,1, 1,1, 0,11, 0,0,0,1,  0, 0, 0,42, 0,0,0,0], ByteOrder::BigEndian,    TagData::Float     (smallvec![f32::from_bits(42)])),
        // Double doesn't fit, neither 8-types
        ];
        for (buf, byte_order, data) in cases {
            println!("Trying {data:?} with {byte_order:?}, should become  {buf:?}");
            let res = IfdLoader:: from_buffer(&buf, 0, false, byte_order, Arc::new(Vec::new().into())).unwrap();
            assert_eq!(res, IfdLoader{
                bigtiff: false,
                byte_order,
                tags: BTreeMap::from([(Tag::from_u16_exhaustive(0x01_01), data)]),
                next_ifd_offset: Some(0),
                ..Default::default()
            });
        }
    }

    #[test]
    #[rustfmt::skip]
    fn test_fits_single_big() {
        // personal sanity checks
        assert_eq!(u16::from_le_bytes([42,0]),42);
        assert_eq!(u16::from_be_bytes([0,42]),42);

        assert_eq!(f32::from_le_bytes([0x42,0,0,0]),f32::from_bits(0x00_00_00_42));
        assert_eq!(f32::from_be_bytes([0,0,0,0x42]),f32::from_bits(0x00_00_00_42));
        let cases = [
        //  entry_count    tag   type       count            offset             next_ifd_offset
        //                  //  /   \ 1 2 3 4 5 6 7 8   1  2  3  4  5  6  7  8  1 2 3 4 5 6 7 8
        ([1,0,0,0,0,0,0,0, 1,1, 1, 0, 1,0,0,0,0,0,0,0, 42, 0, 0, 0, 0, 0, 0, 0, 0,0,0,0,0,0,0,0], ByteOrder::LittleEndian, TagData::Byte      (smallvec![42])                ),
        ([0,0,0,0,0,0,0,1, 1,1, 0, 1, 0,0,0,0,0,0,0,1, 42, 0, 0, 0, 0, 0, 0, 0, 0,0,0,0,0,0,0,0], ByteOrder::BigEndian,    TagData::Byte      (smallvec![42])                ),
        ([1,0,0,0,0,0,0,0, 1,1, 6, 0, 1,0,0,0,0,0,0,0, 42, 0, 0, 0, 0, 0, 0, 0, 0,0,0,0,0,0,0,0], ByteOrder::LittleEndian, TagData::SByte     (smallvec![42])                ),
        ([0,0,0,0,0,0,0,1, 1,1, 0, 6, 0,0,0,0,0,0,0,1, 42, 0, 0, 0, 0, 0, 0, 0, 0,0,0,0,0,0,0,0], ByteOrder::BigEndian,    TagData::SByte     (smallvec![42])                ),
        ([1,0,0,0,0,0,0,0, 1,1, 7, 0, 1,0,0,0,0,0,0,0, 42, 0, 0, 0, 0, 0, 0, 0, 0,0,0,0,0,0,0,0], ByteOrder::LittleEndian, TagData::Undefined (smallvec![42])                ),
        ([0,0,0,0,0,0,0,1, 1,1, 0, 7, 0,0,0,0,0,0,0,1, 42, 0, 0, 0, 0, 0, 0, 0, 0,0,0,0,0,0,0,0], ByteOrder::BigEndian,    TagData::Undefined (smallvec![42])                ),
        ([1,0,0,0,0,0,0,0, 1,1, 2, 0, 1,0,0,0,0,0,0,0,  0, 0, 0, 0, 0, 0, 0, 0, 0,0,0,0,0,0,0,0], ByteOrder::LittleEndian, TagData::Ascii     (smallvec![0 ])               ),
        ([0,0,0,0,0,0,0,1, 1,1, 0, 2, 0,0,0,0,0,0,0,1,  0, 0, 0, 0, 0, 0, 0, 0, 0,0,0,0,0,0,0,0], ByteOrder::BigEndian,    TagData::Ascii     (smallvec![0 ])               ),
        ([1,0,0,0,0,0,0,0, 1,1, 3, 0, 1,0,0,0,0,0,0,0, 42, 0, 0, 0, 0, 0, 0, 0, 0,0,0,0,0,0,0,0], ByteOrder::LittleEndian, TagData::Short     (smallvec![42])                ),
        ([0,0,0,0,0,0,0,1, 1,1, 0, 3, 0,0,0,0,0,0,0,1,  0,42, 0, 0, 0, 0, 0, 0, 0,0,0,0,0,0,0,0], ByteOrder::BigEndian,    TagData::Short     (smallvec![42])                ),
        ([1,0,0,0,0,0,0,0, 1,1, 8, 0, 1,0,0,0,0,0,0,0, 42, 0, 0, 0, 0, 0, 0, 0, 0,0,0,0,0,0,0,0], ByteOrder::LittleEndian, TagData::SShort    (smallvec![42])                ),
        ([0,0,0,0,0,0,0,1, 1,1, 0, 8, 0,0,0,0,0,0,0,1,  0,42, 0, 0, 0, 0, 0, 0, 0,0,0,0,0,0,0,0], ByteOrder::BigEndian,    TagData::SShort    (smallvec![42])                ),
        ([1,0,0,0,0,0,0,0, 1,1, 4, 0, 1,0,0,0,0,0,0,0, 42, 0, 0, 0, 0, 0, 0, 0, 0,0,0,0,0,0,0,0], ByteOrder::LittleEndian, TagData::Long      (smallvec![42])                ),
        ([0,0,0,0,0,0,0,1, 1,1, 0, 4, 0,0,0,0,0,0,0,1,  0, 0, 0,42, 0, 0, 0, 0, 0,0,0,0,0,0,0,0], ByteOrder::BigEndian,    TagData::Long      (smallvec![42])                ),
        ([1,0,0,0,0,0,0,0, 1,1, 9, 0, 1,0,0,0,0,0,0,0, 42, 0, 0, 0, 0, 0, 0, 0, 0,0,0,0,0,0,0,0], ByteOrder::LittleEndian, TagData::SLong     (smallvec![42])                ),
        ([0,0,0,0,0,0,0,1, 1,1, 0, 9, 0,0,0,0,0,0,0,1,  0, 0, 0,42, 0, 0, 0, 0, 0,0,0,0,0,0,0,0], ByteOrder::BigEndian,    TagData::SLong     (smallvec![42])                ),
        ([1,0,0,0,0,0,0,0, 1,1,13, 0, 1,0,0,0,0,0,0,0, 42, 0, 0, 0, 0, 0, 0, 0, 0,0,0,0,0,0,0,0], ByteOrder::LittleEndian, TagData::Ifd       (smallvec![42])                ),
        ([0,0,0,0,0,0,0,1, 1,1, 0,13, 0,0,0,0,0,0,0,1,  0, 0, 0,42, 0, 0, 0, 0, 0,0,0,0,0,0,0,0], ByteOrder::BigEndian,    TagData::Ifd       (smallvec![42])                ),
        ([1,0,0,0,0,0,0,0, 1,1,16, 0, 1,0,0,0,0,0,0,0, 42, 0, 0, 0, 0, 0, 0, 0, 0,0,0,0,0,0,0,0], ByteOrder::LittleEndian, TagData::Long8     (smallvec![42])                ),
        ([0,0,0,0,0,0,0,1, 1,1, 0,16, 0,0,0,0,0,0,0,1,  0, 0, 0, 0, 0, 0, 0,42, 0,0,0,0,0,0,0,0], ByteOrder::BigEndian,    TagData::Long8     (smallvec![42])                ),
        ([1,0,0,0,0,0,0,0, 1,1,17, 0, 1,0,0,0,0,0,0,0, 42, 0, 0, 0, 0, 0, 0, 0, 0,0,0,0,0,0,0,0], ByteOrder::LittleEndian, TagData::SLong8    (smallvec![42])                ),
        ([0,0,0,0,0,0,0,1, 1,1, 0,17, 0,0,0,0,0,0,0,1,  0, 0, 0, 0, 0, 0, 0,42, 0,0,0,0,0,0,0,0], ByteOrder::BigEndian,    TagData::SLong8    (smallvec![42])                ),
        ([1,0,0,0,0,0,0,0, 1,1,18, 0, 1,0,0,0,0,0,0,0, 42, 0, 0, 0, 0, 0, 0, 0, 0,0,0,0,0,0,0,0], ByteOrder::LittleEndian, TagData::Ifd8      (smallvec![42])                ),
        ([0,0,0,0,0,0,0,1, 1,1, 0,18, 0,0,0,0,0,0,0,1,  0, 0, 0, 0, 0, 0, 0,42, 0,0,0,0,0,0,0,0], ByteOrder::BigEndian,    TagData::Ifd8      (smallvec![42])                ),
        ([1,0,0,0,0,0,0,0, 1,1,11, 0, 1,0,0,0,0,0,0,0, 42, 0, 0, 0, 0, 0, 0, 0, 0,0,0,0,0,0,0,0], ByteOrder::LittleEndian, TagData::Float     (smallvec![f32::from_bits(42)])),
        ([0,0,0,0,0,0,0,1, 1,1, 0,11, 0,0,0,0,0,0,0,1,  0, 0, 0,42, 0, 0, 0, 0, 0,0,0,0,0,0,0,0], ByteOrder::BigEndian,    TagData::Float     (smallvec![f32::from_bits(42)])),
        ([1,0,0,0,0,0,0,0, 1,1,12, 0, 1,0,0,0,0,0,0,0, 42, 0, 0, 0, 0, 0, 0, 0, 0,0,0,0,0,0,0,0], ByteOrder::LittleEndian, TagData::Double    (smallvec![f64::from_bits(42)])),
        ([0,0,0,0,0,0,0,1, 1,1, 0,12, 0,0,0,0,0,0,0,1,  0, 0, 0, 0, 0, 0, 0,42, 0,0,0,0,0,0,0,0], ByteOrder::BigEndian,    TagData::Double    (smallvec![f64::from_bits(42)])),
        ([1,0,0,0,0,0,0,0, 1,1, 5, 0, 1,0,0,0,0,0,0,0,  42,0, 0, 0,43, 0, 0, 0, 0,0,0,0,0,0,0,0], ByteOrder::LittleEndian, TagData::Rational  (smallvec![[42, 43]])          ),
        ([0,0,0,0,0,0,0,1, 1,1, 0, 5, 0,0,0,0,0,0,0,1,  0, 0, 0,42, 0, 0, 0,43, 0,0,0,0,0,0,0,0], ByteOrder::BigEndian,    TagData::Rational  (smallvec![[42, 43]])          ),
        ([1,0,0,0,0,0,0,0, 1,1, 10,0, 1,0,0,0,0,0,0,0, 42, 0, 0, 0,43, 0, 0, 0, 0,0,0,0,0,0,0,0], ByteOrder::LittleEndian, TagData::SRational (smallvec![[42, 43]])          ),
        ([0,0,0,0,0,0,0,1, 1,1, 0,10, 0,0,0,0,0,0,0,1,  0, 0, 0,42, 0, 0, 0,43, 0,0,0,0,0,0,0,0], ByteOrder::BigEndian,    TagData::SRational (smallvec![[42, 43]])          ),
        ];
        for (buf, byte_order, data) in cases {
            println!("Trying {data:?} with {byte_order:?}, should become  {buf:?}");
            let res = IfdLoader:: from_buffer(&buf, 0, true, byte_order, Arc::new(Vec::new().into())).unwrap();
            assert_eq!(res, IfdLoader{
                bigtiff: true,
                byte_order,
                tags: BTreeMap::from([(Tag::from_u16_exhaustive(0x01_01), data)]),
                next_ifd_offset: Some(0),
                ..Default::default()
            });
        }
    }

    #[test]
    #[rustfmt::skip]
    fn test_fits_multi_notbig() {
        // personal sanity checks
        assert_eq!(u16::from_le_bytes([42,0]),42);
        assert_eq!(u16::from_be_bytes([0,42]),42);

        assert_eq!(f32::from_le_bytes([0x42,0,0,0]),f32::from_bits(0x00_00_00_42));
        assert_eq!(f32::from_be_bytes([0,0,0,0x42]),f32::from_bits(0x00_00_00_42));
        let cases = [
        // n    tag  type  count    offset      next ifd
        //      //  /  \  /     \   /     \     /     \
        ([1,0, 1,1, 1, 0, 4,0,0,0, 42,42,42,42, 0,0,0,0], ByteOrder::LittleEndian, TagData::Byte      (smallvec![42; 4]) ),
        ([0,1, 1,1, 0, 1, 0,0,0,4, 42,42,42,42, 0,0,0,0], ByteOrder::BigEndian,    TagData::Byte      (smallvec![42; 4]) ),
        ([1,0, 1,1, 6, 0, 4,0,0,0, 42,42,42,42, 0,0,0,0], ByteOrder::LittleEndian, TagData::SByte     (smallvec![42; 4]) ),
        ([0,1, 1,1, 0, 6, 0,0,0,4, 42,42,42,42, 0,0,0,0], ByteOrder::BigEndian,    TagData::SByte     (smallvec![42; 4]) ),
        ([1,0, 1,1, 7, 0, 4,0,0,0, 42,42,42,42, 0,0,0,0], ByteOrder::LittleEndian, TagData::Undefined (smallvec![42; 4]) ),
        ([0,1, 1,1, 0, 7, 0,0,0,4, 42,42,42,42, 0,0,0,0], ByteOrder::BigEndian,    TagData::Undefined (smallvec![42; 4]) ),
        ([1,0, 1,1, 2, 0, 4,0,0,0, 42,42,42, 0, 0,0,0,0], ByteOrder::LittleEndian, TagData::Ascii     ("***\0".as_bytes().into())),
        ([0,1, 1,1, 0, 2, 0,0,0,4, 42,42,42, 0, 0,0,0,0], ByteOrder::BigEndian,    TagData::Ascii     ("***\0".as_bytes().into())),
        ([1,0, 1,1, 3, 0, 2,0,0,0, 42, 0,42, 0, 0,0,0,0], ByteOrder::LittleEndian, TagData::Short     (smallvec![42; 2]) ),
        ([0,1, 1,1, 0, 3, 0,0,0,2,  0,42, 0,42, 0,0,0,0], ByteOrder::BigEndian,    TagData::Short     (smallvec![42; 2]) ),
        ([1,0, 1,1, 8, 0, 2,0,0,0, 42, 0,42, 0, 0,0,0,0], ByteOrder::LittleEndian, TagData::SShort    (smallvec![42; 2]) ),
        ([0,1, 1,1, 0, 8, 0,0,0,2,  0,42, 0,42, 0,0,0,0], ByteOrder::BigEndian,    TagData::SShort    (smallvec![42; 2]) ),
        ([0,1, 1,1, 0, 2, 0,0,0,4, b'A',b'B',b'C',0, 0,0,0,0], ByteOrder::BigEndian, TagData::Ascii("ABC\0".as_bytes().into())),
        // others don't fit, neither 8-types
        ];
        for (buf, byte_order, data) in cases {
            println!("Trying {data:?} with {byte_order:?}, should become  {buf:?}");
            let res = IfdLoader:: from_buffer(&buf, 0, false, byte_order, Arc::new(Vec::new().into())).unwrap();
            assert_eq!(res, IfdLoader{
                bigtiff: false,
                byte_order,
                tags: BTreeMap::from([(Tag::from_u16_exhaustive(0x01_01), data)]),
                next_ifd_offset: Some(0),
                ..Default::default()
            });
        }
    }

    #[test]
    #[rustfmt::skip]
    fn test_fits_multi_big() {
        // personal sanity checks
        assert_eq!(u16::from_le_bytes([42,0]),42);
        assert_eq!(u16::from_be_bytes([0,42]),42);

        assert_eq!(f32::from_le_bytes([0x42,0,0,0]),f32::from_bits(0x00_00_00_42));
        assert_eq!(f32::from_be_bytes([0,0,0,0x42]),f32::from_bits(0x00_00_00_42));
        let cases = [
        //   n_entries      tag   type       count            offset            next_ifd_offset
        //                  //  /   \ 1 2 3 4 5 6 7 8   1  2  3  4  5  6  7  8  1 2 3 4 5 6 7 8
        ([1,0,0,0,0,0,0,0, 1,1, 1, 0, 8,0,0,0,0,0,0,0, 42,42,42,42,42,42,42,42, 0,0,0,0,0,0,0,0], ByteOrder::LittleEndian, TagData::Byte      (smallvec![42                ; 8])),
        ([0,0,0,0,0,0,0,1, 1,1, 0, 1, 0,0,0,0,0,0,0,8, 42,42,42,42,42,42,42,42, 0,0,0,0,0,0,0,0], ByteOrder::BigEndian,    TagData::Byte      (smallvec![42                ; 8])),
        ([1,0,0,0,0,0,0,0, 1,1, 6, 0, 8,0,0,0,0,0,0,0, 42,42,42,42,42,42,42,42, 0,0,0,0,0,0,0,0], ByteOrder::LittleEndian, TagData::SByte     (smallvec![42                ; 8])),
        ([0,0,0,0,0,0,0,1, 1,1, 0, 6, 0,0,0,0,0,0,0,8, 42,42,42,42,42,42,42,42, 0,0,0,0,0,0,0,0], ByteOrder::BigEndian,    TagData::SByte     (smallvec![42                ; 8])),
        ([1,0,0,0,0,0,0,0, 1,1, 7, 0, 8,0,0,0,0,0,0,0, 42,42,42,42,42,42,42,42, 0,0,0,0,0,0,0,0], ByteOrder::LittleEndian, TagData::Undefined (smallvec![42                ; 8])),
        ([0,0,0,0,0,0,0,1, 1,1, 0, 7, 0,0,0,0,0,0,0,8, 42,42,42,42,42,42,42,42, 0,0,0,0,0,0,0,0], ByteOrder::BigEndian,    TagData::Undefined (smallvec![42                ; 8])),
        ([1,0,0,0,0,0,0,0, 1,1, 2, 0, 8,0,0,0,0,0,0,0, 42,42,42,42,42,42,42, 0, 0,0,0,0,0,0,0,0], ByteOrder::LittleEndian, TagData::Ascii     ("*******\0".as_bytes().into()   )),
        ([0,0,0,0,0,0,0,1, 1,1, 0, 2, 0,0,0,0,0,0,0,8, 42,42,42,42,42,42,42, 0, 0,0,0,0,0,0,0,0], ByteOrder::BigEndian,    TagData::Ascii     ("*******\0".as_bytes().into()   )),
        ([1,0,0,0,0,0,0,0, 1,1, 3, 0, 4,0,0,0,0,0,0,0, 42, 0,42, 0,42, 0,42, 0, 0,0,0,0,0,0,0,0], ByteOrder::LittleEndian, TagData::Short     (smallvec![42                ; 4])),
        ([0,0,0,0,0,0,0,1, 1,1, 0, 3, 0,0,0,0,0,0,0,4,  0,42, 0,42, 0,42, 0,42, 0,0,0,0,0,0,0,0], ByteOrder::BigEndian,    TagData::Short     (smallvec![42                ; 4])),
        ([1,0,0,0,0,0,0,0, 1,1, 8, 0, 4,0,0,0,0,0,0,0, 42, 0,42, 0,42, 0,42, 0, 0,0,0,0,0,0,0,0], ByteOrder::LittleEndian, TagData::SShort    (smallvec![42                ; 4])),
        ([0,0,0,0,0,0,0,1, 1,1, 0, 8, 0,0,0,0,0,0,0,4,  0,42, 0,42, 0,42, 0,42, 0,0,0,0,0,0,0,0], ByteOrder::BigEndian,    TagData::SShort    (smallvec![42                ; 4])),
        ([1,0,0,0,0,0,0,0, 1,1, 4, 0, 2,0,0,0,0,0,0,0, 42, 0, 0, 0,42, 0, 0, 0, 0,0,0,0,0,0,0,0], ByteOrder::LittleEndian, TagData::Long      (smallvec![42                ; 2])),
        ([0,0,0,0,0,0,0,1, 1,1, 0, 4, 0,0,0,0,0,0,0,2,  0, 0, 0,42, 0, 0, 0,42, 0,0,0,0,0,0,0,0], ByteOrder::BigEndian,    TagData::Long      (smallvec![42                ; 2])),
        ([1,0,0,0,0,0,0,0, 1,1, 9, 0, 2,0,0,0,0,0,0,0, 42, 0, 0, 0,42, 0, 0, 0, 0,0,0,0,0,0,0,0], ByteOrder::LittleEndian, TagData::SLong     (smallvec![42                ; 2])),
        ([0,0,0,0,0,0,0,1, 1,1, 0, 9, 0,0,0,0,0,0,0,2,  0, 0, 0,42, 0, 0, 0,42, 0,0,0,0,0,0,0,0], ByteOrder::BigEndian,    TagData::SLong     (smallvec![42                ; 2])),
        ([1,0,0,0,0,0,0,0, 1,1,13, 0, 2,0,0,0,0,0,0,0, 42, 0, 0, 0,42, 0, 0, 0, 0,0,0,0,0,0,0,0], ByteOrder::LittleEndian, TagData::Ifd       (smallvec![42                ; 2])),
        ([0,0,0,0,0,0,0,1, 1,1, 0,13, 0,0,0,0,0,0,0,2,  0, 0, 0,42, 0, 0, 0,42, 0,0,0,0,0,0,0,0], ByteOrder::BigEndian,    TagData::Ifd       (smallvec![42                ; 2])),
        ([1,0,0,0,0,0,0,0, 1,1,11, 0, 2,0,0,0,0,0,0,0, 42, 0, 0, 0,42, 0, 0, 0, 0,0,0,0,0,0,0,0], ByteOrder::LittleEndian, TagData::Float     (smallvec![f32::from_bits(42); 2])),
        ([0,0,0,0,0,0,0,1, 1,1, 0,11, 0,0,0,0,0,0,0,2,  0, 0, 0,42, 0, 0, 0,42, 0,0,0,0,0,0,0,0], ByteOrder::BigEndian,    TagData::Float     (smallvec![f32::from_bits(42); 2])),
        ];
        for (buf, byte_order, data) in cases {
            println!("Trying {data:?} with {byte_order:?}, should become  {buf:?}");
            let res = IfdLoader:: from_buffer(&buf, 0, true, byte_order, Arc::new(Vec::new().into())).unwrap();
            assert_eq!(res, IfdLoader{
                bigtiff: true,
                byte_order,
                tags: BTreeMap::from([(Tag::from_u16_exhaustive(0x01_01), data)]),
                next_ifd_offset: Some(0),
                ..Default::default()
            });
        }
    }

    #[test]
    #[rustfmt::skip]
    fn test_notfits_notbig() {
        // personal sanity checks
        assert_eq!(u16::from_le_bytes([42,0]),42);
        assert_eq!(u16::from_be_bytes([0,42]),42);

        assert_eq!(f32::from_le_bytes([0x42,0,0,0]),f32::from_bits(0x00_00_00_42));
        assert_eq!(f32::from_be_bytes([0,0,0,0x42]),f32::from_bits(0x00_00_00_42));
        let cases = [
        // n    tag type  count    offset      next ifd
        //       // /  \  /     \   /     \     /     \
        ([1,0, 1,1, 1, 0, 5,0,0,0, 42, 0, 0, 0, 0,0,0,0], ByteOrder::LittleEndian, 5, TagType::BYTE      ),
        ([0,1, 1,1, 0, 1, 0,0,0,5,  0, 0, 0,42, 0,0,0,0], ByteOrder::BigEndian   , 5, TagType::BYTE      ),
        ([1,0, 1,1, 6, 0, 5,0,0,0, 42, 0, 0, 0, 0,0,0,0], ByteOrder::LittleEndian, 5, TagType::SBYTE     ),
        ([0,1, 1,1, 0, 6, 0,0,0,5,  0, 0, 0,42, 0,0,0,0], ByteOrder::BigEndian   , 5, TagType::SBYTE     ),
        ([1,0, 1,1, 7, 0, 5,0,0,0, 42, 0, 0, 0, 0,0,0,0], ByteOrder::LittleEndian, 5, TagType::UNDEFINED ),
        ([0,1, 1,1, 0, 7, 0,0,0,5,  0, 0, 0,42, 0,0,0,0], ByteOrder::BigEndian   , 5, TagType::UNDEFINED ),
        ([1,0, 1,1, 2, 0, 5,0,0,0, 42, 0, 0, 0, 0,0,0,0], ByteOrder::LittleEndian, 5, TagType::ASCII     ),
        ([0,1, 1,1, 0, 2, 0,0,0,5,  0, 0, 0,42, 0,0,0,0], ByteOrder::BigEndian   , 5, TagType::ASCII     ),
        ([1,0, 1,1, 3, 0, 3,0,0,0, 42, 0, 0, 0, 0,0,0,0], ByteOrder::LittleEndian, 3, TagType::SHORT     ),
        ([0,1, 1,1, 0, 3, 0,0,0,3,  0, 0, 0,42, 0,0,0,0], ByteOrder::BigEndian   , 3, TagType::SHORT     ),
        ([1,0, 1,1, 8, 0, 3,0,0,0, 42, 0, 0, 0, 0,0,0,0], ByteOrder::LittleEndian, 3, TagType::SSHORT    ),
        ([0,1, 1,1, 0, 8, 0,0,0,3,  0, 0, 0,42, 0,0,0,0], ByteOrder::BigEndian   , 3, TagType::SSHORT    ),
        ([1,0, 1,1, 4, 0, 2,0,0,0, 42, 0, 0, 0, 0,0,0,0], ByteOrder::LittleEndian, 2, TagType::LONG      ),
        ([0,1, 1,1, 0, 4, 0,0,0,2,  0, 0, 0,42, 0,0,0,0], ByteOrder::BigEndian   , 2, TagType::LONG      ),
        ([1,0, 1,1,13, 0, 2,0,0,0, 42, 0, 0, 0, 0,0,0,0], ByteOrder::LittleEndian, 2, TagType::IFD       ),
        ([0,1, 1,1, 0,13, 0,0,0,2,  0, 0, 0,42, 0,0,0,0], ByteOrder::BigEndian   , 2, TagType::IFD       ),
        ([1,0, 1,1, 9, 0, 2,0,0,0, 42, 0, 0, 0, 0,0,0,0], ByteOrder::LittleEndian, 2, TagType::SLONG     ),
        ([0,1, 1,1, 0, 9, 0,0,0,2,  0, 0, 0,42, 0,0,0,0], ByteOrder::BigEndian   , 2, TagType::SLONG     ),
        ([1,0, 1,1, 11,0, 2,0,0,0, 42, 0, 0, 0, 0,0,0,0], ByteOrder::LittleEndian, 2, TagType::FLOAT     ),
        ([0,1, 1,1, 0,11, 0,0,0,2,  0, 0, 0,42, 0,0,0,0], ByteOrder::BigEndian   , 2, TagType::FLOAT     ),
        ([1,0, 1,1, 12,0, 1,0,0,0, 42, 0, 0, 0, 0,0,0,0], ByteOrder::LittleEndian, 1, TagType::DOUBLE    ),
        ([0,1, 1,1, 0,12, 0,0,0,1,  0, 0, 0,42, 0,0,0,0], ByteOrder::BigEndian   , 1, TagType::DOUBLE    ),
        ([1,0, 1,1, 5, 0, 1,0,0,0, 42, 0, 0, 0, 0,0,0,0], ByteOrder::LittleEndian, 1, TagType::RATIONAL  ),
        ([0,1, 1,1, 0, 5, 0,0,0,1,  0, 0, 0,42, 0,0,0,0], ByteOrder::BigEndian   , 1, TagType::RATIONAL  ),
        ([1,0, 1,1, 10,0, 1,0,0,0, 42, 0, 0, 0, 0,0,0,0], ByteOrder::LittleEndian, 1, TagType::SRATIONAL ),
        ([0,1, 1,1, 0,10, 0,0,0,1,  0, 0, 0,42, 0,0,0,0], ByteOrder::BigEndian   , 1, TagType::SRATIONAL ),
        // Double doesn't fit, neither 8-types
        ];
        for (buf, byte_order, count, tag_type) in cases {
            println!("Trying {tag_type:?} with {byte_order:?}, should become  {buf:?}");
            let res = IfdLoader:: from_buffer(&buf, 0, false, byte_order, Arc::new(Vec::new().into())).unwrap();
            assert_eq!(res, IfdLoader{
                bigtiff: false,
                byte_order,
                tag_offsets: BTreeMap::from([(Tag::from_u16_exhaustive(0x01_01), Offset{tag_type, count, offset: 42})]),
                next_ifd_offset: Some(0),
                ..Default::default()
            });
        }
    }

    #[test]
    #[rustfmt::skip]
    fn test_notfits_big() {
        // personal sanity checks
        assert_eq!(u16::from_le_bytes([42,0]),42);
        assert_eq!(u16::from_be_bytes([0,42]),42);

        assert_eq!(f32::from_le_bytes([0x42,0,0,0]),f32::from_bits(0x00_00_00_42));
        assert_eq!(f32::from_be_bytes([0,0,0,0x42]),f32::from_bits(0x00_00_00_42));
        let cases = [
        //  entry_count    tag   type       count            offset             next_ifd_offset
        //                  //  /   \ 1 2 3 4 5 6 7 8   1  2  3  4  5  6  7  8  1 2 3 4 5 6 7 8
        ([1,0,0,0,0,0,0,0, 1,1, 1, 0, 9,0,0,0,0,0,0,0, 42, 0, 0, 0, 0, 0, 0, 0, 0,0,0,0,0,0,0,0], ByteOrder::LittleEndian, 9, TagType::BYTE      ),
        ([0,0,0,0,0,0,0,1, 1,1, 0, 1, 0,0,0,0,0,0,0,9,  0, 0, 0, 0, 0, 0, 0,42, 0,0,0,0,0,0,0,0], ByteOrder::BigEndian   , 9, TagType::BYTE      ),
        ([1,0,0,0,0,0,0,0, 1,1, 6, 0, 9,0,0,0,0,0,0,0, 42, 0, 0, 0, 0, 0, 0, 0, 0,0,0,0,0,0,0,0], ByteOrder::LittleEndian, 9, TagType::SBYTE     ),
        ([0,0,0,0,0,0,0,1, 1,1, 0, 6, 0,0,0,0,0,0,0,9,  0, 0, 0, 0, 0, 0, 0,42, 0,0,0,0,0,0,0,0], ByteOrder::BigEndian   , 9, TagType::SBYTE     ),
        ([1,0,0,0,0,0,0,0, 1,1, 7, 0, 9,0,0,0,0,0,0,0, 42, 0, 0, 0, 0, 0, 0, 0, 0,0,0,0,0,0,0,0], ByteOrder::LittleEndian, 9, TagType::UNDEFINED ),
        ([0,0,0,0,0,0,0,1, 1,1, 0, 7, 0,0,0,0,0,0,0,9,  0, 0, 0, 0, 0, 0, 0,42, 0,0,0,0,0,0,0,0], ByteOrder::BigEndian   , 9, TagType::UNDEFINED ),
        ([1,0,0,0,0,0,0,0, 1,1, 2, 0, 9,0,0,0,0,0,0,0, 42, 0, 0, 0, 0, 0, 0, 0, 0,0,0,0,0,0,0,0], ByteOrder::LittleEndian, 9, TagType::ASCII     ),
        ([0,0,0,0,0,0,0,1, 1,1, 0, 2, 0,0,0,0,0,0,0,9,  0, 0, 0, 0, 0, 0, 0,42, 0,0,0,0,0,0,0,0], ByteOrder::BigEndian   , 9, TagType::ASCII     ),
        ([1,0,0,0,0,0,0,0, 1,1, 3, 0, 5,0,0,0,0,0,0,0, 42, 0, 0, 0, 0, 0, 0, 0, 0,0,0,0,0,0,0,0], ByteOrder::LittleEndian, 5, TagType::SHORT     ),
        ([0,0,0,0,0,0,0,1, 1,1, 0, 3, 0,0,0,0,0,0,0,5,  0, 0, 0, 0, 0, 0, 0,42, 0,0,0,0,0,0,0,0], ByteOrder::BigEndian   , 5, TagType::SHORT     ),
        ([1,0,0,0,0,0,0,0, 1,1, 8, 0, 5,0,0,0,0,0,0,0, 42, 0, 0, 0, 0, 0, 0, 0, 0,0,0,0,0,0,0,0], ByteOrder::LittleEndian, 5, TagType::SSHORT    ),
        ([0,0,0,0,0,0,0,1, 1,1, 0, 8, 0,0,0,0,0,0,0,5,  0, 0, 0, 0, 0, 0, 0,42, 0,0,0,0,0,0,0,0], ByteOrder::BigEndian   , 5, TagType::SSHORT    ),
        ([1,0,0,0,0,0,0,0, 1,1, 4, 0, 3,0,0,0,0,0,0,0, 42, 0, 0, 0, 0, 0, 0, 0, 0,0,0,0,0,0,0,0], ByteOrder::LittleEndian, 3, TagType::LONG      ),
        ([0,0,0,0,0,0,0,1, 1,1, 0, 4, 0,0,0,0,0,0,0,3,  0, 0, 0, 0, 0, 0, 0,42, 0,0,0,0,0,0,0,0], ByteOrder::BigEndian   , 3, TagType::LONG      ),
        ([1,0,0,0,0,0,0,0, 1,1, 9, 0, 3,0,0,0,0,0,0,0, 42, 0, 0, 0, 0, 0, 0, 0, 0,0,0,0,0,0,0,0], ByteOrder::LittleEndian, 3, TagType::SLONG     ),
        ([0,0,0,0,0,0,0,1, 1,1, 0, 9, 0,0,0,0,0,0,0,3,  0, 0, 0, 0, 0, 0, 0,42, 0,0,0,0,0,0,0,0], ByteOrder::BigEndian   , 3, TagType::SLONG     ),
        ([1,0,0,0,0,0,0,0, 1,1,13, 0, 3,0,0,0,0,0,0,0, 42, 0, 0, 0, 0, 0, 0, 0, 0,0,0,0,0,0,0,0], ByteOrder::LittleEndian, 3, TagType::IFD       ),
        ([0,0,0,0,0,0,0,1, 1,1, 0,13, 0,0,0,0,0,0,0,3,  0, 0, 0, 0, 0, 0, 0,42, 0,0,0,0,0,0,0,0], ByteOrder::BigEndian   , 3, TagType::IFD       ),
        ([1,0,0,0,0,0,0,0, 1,1,16, 0, 3,0,0,0,0,0,0,0, 42, 0, 0, 0, 0, 0, 0, 0, 0,0,0,0,0,0,0,0], ByteOrder::LittleEndian, 3, TagType::LONG8     ),
        ([0,0,0,0,0,0,0,1, 1,1, 0,16, 0,0,0,0,0,0,0,3,  0, 0, 0, 0, 0, 0, 0,42, 0,0,0,0,0,0,0,0], ByteOrder::BigEndian   , 3, TagType::LONG8     ),
        ([1,0,0,0,0,0,0,0, 1,1,17, 0, 3,0,0,0,0,0,0,0, 42, 0, 0, 0, 0, 0, 0, 0, 0,0,0,0,0,0,0,0], ByteOrder::LittleEndian, 3, TagType::SLONG8    ),
        ([0,0,0,0,0,0,0,1, 1,1, 0,17, 0,0,0,0,0,0,0,3,  0, 0, 0, 0, 0, 0, 0,42, 0,0,0,0,0,0,0,0], ByteOrder::BigEndian   , 3, TagType::SLONG8    ),
        ([1,0,0,0,0,0,0,0, 1,1,18, 0, 3,0,0,0,0,0,0,0, 42, 0, 0, 0, 0, 0, 0, 0, 0,0,0,0,0,0,0,0], ByteOrder::LittleEndian, 3, TagType::IFD8      ),
        ([0,0,0,0,0,0,0,1, 1,1, 0,18, 0,0,0,0,0,0,0,3,  0, 0, 0, 0, 0, 0, 0,42, 0,0,0,0,0,0,0,0], ByteOrder::BigEndian   , 3, TagType::IFD8      ),
        ([1,0,0,0,0,0,0,0, 1,1,11, 0, 3,0,0,0,0,0,0,0, 42, 0, 0, 0, 0, 0, 0, 0, 0,0,0,0,0,0,0,0], ByteOrder::LittleEndian, 3, TagType::FLOAT     ),
        ([0,0,0,0,0,0,0,1, 1,1, 0,11, 0,0,0,0,0,0,0,3,  0, 0, 0, 0, 0, 0, 0,42, 0,0,0,0,0,0,0,0], ByteOrder::BigEndian   , 3, TagType::FLOAT     ),
        ([1,0,0,0,0,0,0,0, 1,1,12, 0, 2,0,0,0,0,0,0,0, 42, 0, 0, 0, 0, 0, 0, 0, 0,0,0,0,0,0,0,0], ByteOrder::LittleEndian, 2, TagType::DOUBLE    ),
        ([0,0,0,0,0,0,0,1, 1,1, 0,12, 0,0,0,0,0,0,0,2,  0, 0, 0, 0, 0, 0, 0,42, 0,0,0,0,0,0,0,0], ByteOrder::BigEndian   , 2, TagType::DOUBLE    ),
        ([1,0,0,0,0,0,0,0, 1,1, 5, 0, 2,0,0,0,0,0,0,0, 42, 0, 0, 0, 0, 0, 0, 0, 0,0,0,0,0,0,0,0], ByteOrder::LittleEndian, 2, TagType::RATIONAL  ),
        ([0,0,0,0,0,0,0,1, 1,1, 0, 5, 0,0,0,0,0,0,0,2,  0, 0, 0, 0, 0, 0, 0,42, 0,0,0,0,0,0,0,0], ByteOrder::BigEndian   , 2, TagType::RATIONAL  ),
        ([1,0,0,0,0,0,0,0, 1,1,10, 0, 2,0,0,0,0,0,0,0, 42, 0, 0, 0, 0, 0, 0, 0, 0,0,0,0,0,0,0,0], ByteOrder::LittleEndian, 2, TagType::SRATIONAL ),
        ([0,0,0,0,0,0,0,1, 1,1, 0,10, 0,0,0,0,0,0,0,2,  0, 0, 0, 0, 0, 0, 0,42, 0,0,0,0,0,0,0,0], ByteOrder::BigEndian   , 2, TagType::SRATIONAL ),
        ];
        for (buf, byte_order, count, tag_type) in cases {
            println!("Trying {tag_type:?} with {byte_order:?}, should become  {buf:?}");
            let res = IfdLoader:: from_buffer(&buf, 0, true, byte_order, Arc::new(Vec::new().into())).unwrap();
            assert_eq!(res, IfdLoader{
                bigtiff: true,
                byte_order,
                tag_offsets: BTreeMap::from([(Tag::from_u16_exhaustive(0x01_01), Offset{tag_type, count, offset: 42})]),
                next_ifd_offset: Some(0),
                ..Default::default()
            });
        }
    }
}
