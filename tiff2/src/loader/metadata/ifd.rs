use std::collections::BTreeMap;
use std::ops::Range;

use exn::{bail, OptionExt, ResultExt};

use crate::loader::metadata::error::TiffLoadError;
use crate::loader::TiffLoadResult;
use crate::structs::{
    entry_size, num_entries_size, offset_size, offset_tag_type, Ifd, IfdEntry, Offset, Tag,
    TagData, TagType,
};
use crate::ByteOrder;

#[derive(Debug, Clone, PartialEq)]
pub struct IfdLoader {
    pub bigtiff: bool,
    pub byte_order: ByteOrder,
    pub ifd: Ifd,
}

impl IfdLoader {
    pub fn count(&self) -> usize {
        self.ifd.count()
    }

    /// given a buffer holding the count value, get the number of entries
    fn ifd_entry_count(
        buf: &[u8],
        offset: u64,
        bigtiff: bool,
        byte_order: ByteOrder,
    ) -> TiffLoadResult<u64> {
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
        .or_raise(|| TiffLoadError::invalid_buffer(offset..offset + num_entries_size(bigtiff)))?
        .try_into()
        .unwrap();
        Ok(count)
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
    ///
    /// ```
    pub fn from_buffer(
        ifd_buf: &[u8],
        offset: u64,
        bigtiff: bool,
        byte_order: ByteOrder,
    ) -> TiffLoadResult<(Self, u64)> {
        let entry_count = Self::ifd_entry_count(ifd_buf, offset, bigtiff, byte_order)?;

        // check if the entire ifd is in memory
        if u64::try_from(ifd_buf.len()).unwrap()
            < entry_count * entry_size(bigtiff) + offset_size(bigtiff)
        {
            bail!(TiffLoadError::invalid_buffer(
                offset
                    ..offset
                        + entry_count * entry_size(bigtiff)
                        + offset_size(bigtiff)
                        + num_entries_size(bigtiff),
            ))
        }
        let mut pos = num_entries_size(bigtiff) as usize;
        let mut ifd_data = BTreeMap::new();
        // start reading entries
        for _ in 0..entry_count {
            // TODO: This should really become a
            //
            // let (tag, entry) = smart_function(&ifd_buf[pos..], bigtiff, byte_order)
            // pos += ifd_entry_size(bigtiff)
            //
            // after this refactor

            // tag and tag type in a single array
            let tag_ttype = TagData::from_buffer(&ifd_buf[pos..], TagType::SHORT, 2, byte_order)
                .expect("TODO: error handling");
            pos += tag_ttype.as_ref().len(); // 4
                                             // extract tag and tag type from array
            let tag = Tag::from_u16_exhaustive(<&[u16]>::try_from(&tag_ttype).unwrap()[0]);
            let tag_type = TagType::from_u16(<&[u16]>::try_from(&tag_ttype).unwrap()[1])
                .ok_or_raise(|| {
                    TiffLoadError::permanent(format!(
                        "invalid tag type {}",
                        <&[u16]>::try_from(&tag_ttype).unwrap()[1]
                    ))
                })?;
            // count
            let value_count: u64 =
                TagData::from_buffer(&ifd_buf[pos..], offset_tag_type(bigtiff), 1, byte_order)
                    // we can unwrap, because we checked buffer size
                    .unwrap()
                    .try_into()
                    .unwrap();
            pos += offset_size(bigtiff) as usize; // 8 or 4, coincidentally also offset_size(bigtiff)
            let entry =
                if u64::try_from(tag_type.size()).unwrap() * value_count > offset_size(bigtiff) {
                    IfdEntry::Offset(Offset {
                        tag_type,
                        count: value_count,
                        offset: TagData::from_buffer(
                            &ifd_buf[pos..],
                            offset_tag_type(bigtiff),
                            1,
                            byte_order,
                        )
                        .unwrap()
                        .try_into()
                        .unwrap(),
                    })
                } else {
                    IfdEntry::Value(
                        TagData::from_buffer(
                            &ifd_buf[pos..],
                            tag_type,
                            value_count as usize,
                            byte_order,
                        )
                        .unwrap(),
                    )
                };
            pos += offset_size(bigtiff) as usize;
            ifd_data.insert(tag, entry);
        }
        let next_ifd_offset: u64 =
            TagData::from_buffer(&ifd_buf[pos..], offset_tag_type(bigtiff), 1, byte_order)
                .unwrap()
                .try_into()
                .unwrap();
        Ok((
            Self {
                bigtiff,
                byte_order,
                ifd: Ifd::from_tags(ifd_data),
            },
            next_ifd_offset,
        ))
    }

    /// Get deferred values
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
    /// let mut ifd_loader = IfdLoader::from_buffer(buf,0,false,ByteOrder::LittleEndian).unwrap().0;
    ///
    /// assert_eq!(ifd_loader.deferred_values_mut().collect::<Vec<_>>().len(), 1);
    ///
    /// for (tag, val) in ifd_loader.deferred_values_mut() {
    ///     # assert_eq!(tag, &Tag::TileOffsets);
    ///     let loaded;
    ///     {
    ///         let IfdEntry::Offset(o) = val else {unreachable!();};
    ///         // fetch the entry when needed
    ///         loaded = TagData::Long(smallvec![42,43]);
    ///     }
    ///     *val = IfdEntry::Value(loaded);
    /// }
    /// ```
    pub fn deferred_values_mut(&mut self) -> impl Iterator<Item = (&Tag, &mut IfdEntry)> {
        self.ifd
            .iter_mut()
            .filter(|(_, v)| matches!(v, IfdEntry::Offset(_)))
    }

    pub fn deferred_ranges<'a>(&'a self) -> impl Iterator<Item = Range<u64>> + use<'a> {
        self.ifd.iter().filter_map(|(_, v)| match v {
            IfdEntry::Offset(o) => Some(o.range()),
            _ => None,
        })
    }

    pub fn finish(self) -> Ifd {
        self.ifd
    }
}

#[cfg(test)]
mod test {
    use smallvec::smallvec;

    use super::*;

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
            let ifd = Ifd {
                data: BTreeMap::from([
                    (Tag::ImageLength, IfdEntry::Value(res1)),
                    (Tag::ImageWidth, IfdEntry::Value(res2))
                ])
            };
            let (res, next) = IfdLoader:: from_buffer(&buf, 0, false, ByteOrder::LittleEndian).unwrap();
            assert_eq!(next, 0);
            assert_eq!(&res, &IfdLoader{bigtiff: false, byte_order: ByteOrder::LittleEndian, ifd});
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
            let ifd = Ifd {
                data: BTreeMap::from([(Tag::from_u16_exhaustive(0x01_01), IfdEntry::Value(data))])
            };
            let (res, next) = IfdLoader:: from_buffer(&buf, 0, false, byte_order).unwrap();
            assert_eq!(next, 0);
            assert_eq!(res, IfdLoader{bigtiff: false, byte_order, ifd});
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
            let ifd = Ifd {
                data: BTreeMap::from([(Tag::from_u16_exhaustive(0x01_01), IfdEntry::Value(data))])
            };
            let (res, next) = IfdLoader:: from_buffer(&buf, 0, true, byte_order).unwrap();
            assert_eq!(next, 0);
            assert_eq!(res, IfdLoader{bigtiff: true, byte_order, ifd});
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
            let ifd = Ifd {
                data: BTreeMap::from([(Tag::from_u16_exhaustive(0x01_01), IfdEntry::Value(data))])
            };
            let (res, next) = IfdLoader:: from_buffer(&buf, 0, false, byte_order).unwrap();
            assert_eq!(next, 0);
            assert_eq!(res, IfdLoader{bigtiff: false, byte_order, ifd});
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
            let ifd = Ifd {
                data: BTreeMap::from([(Tag::from_u16_exhaustive(0x01_01), IfdEntry::Value(data))])
            };
            let (res, next) = IfdLoader:: from_buffer(&buf, 0, true, byte_order).unwrap();
            assert_eq!(next, 0);
            assert_eq!(res, IfdLoader{bigtiff: true, byte_order, ifd});
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
            let ifd = Ifd {
                data: BTreeMap::from([(Tag::from_u16_exhaustive(0x01_01), IfdEntry::Offset(Offset { tag_type, count, offset: 42 }))])
            };
            let (res, next) = IfdLoader:: from_buffer(&buf, 0, false, byte_order).unwrap();
            assert_eq!(next, 0);
            assert_eq!(res, IfdLoader{bigtiff: false, byte_order, ifd});
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
            let ifd = Ifd {
                data: BTreeMap::from([(Tag::from_u16_exhaustive(0x01_01), IfdEntry::Offset(Offset { tag_type, count, offset: 42 }))])
            };
            let (res, next) = IfdLoader:: from_buffer(&buf, 0, true, byte_order).unwrap();
            assert_eq!(next, 0);
            assert_eq!(res, IfdLoader{bigtiff: true, byte_order, ifd});
        }
    }
}
