use exn::{bail, ResultExt};
use smallvec::smallvec;

use crate::saver::error::SaverError;
use crate::saver::SaverResult;
use crate::structs::{entry_size, num_entries_size, offset_size, Ifd, IfdEntry, Tag, TagData};
use crate::ByteOrder;

/// An Ifd saver
///
/// This is a struct that can be created from an [`Ifd`] to tell what values need to
/// be written at different places.
///
/// Once all those values have been changed to [`IfdEntry::Offset`], you can call write_to on this, consuming the [`IfdSaver`]
pub struct IfdSaver {
    offset: u64,
    bigtiff: bool,
    byte_order: ByteOrder,
    ifd: Ifd,
}

impl IfdSaver {
    pub fn count(&self) -> usize {
        self.ifd.count()
    }

    /// Returns an iterator of Entries that still need to be changed to Offset before this can be written
    fn to_do(&self) -> impl Iterator<Item = (&Tag, &TagData)> {
        self.ifd.iter().filter_map(|(k, v)| match v {
            IfdEntry::Value(tag_data) => {
                if u64::try_from(tag_data.as_ref().len()).unwrap() > offset_size(self.bigtiff) {
                    Some((k, tag_data))
                } else {
                    None
                }
            }
            IfdEntry::Offset(_) => None,
        })
    }

    /// Given an IFD and a buffer, write the IFD to the buffer
    ///
    /// TODO: how should it treat the IFD? also defer non-inlined values?
    pub fn from_ifd(offset: u64, ifd: Ifd, bigtiff: bool, byte_order: ByteOrder) -> Self {
        Self {
            offset,
            bigtiff,
            byte_order,
            ifd,
        }
    }

    fn required_len(&self) -> u64 {
        let count = u64::try_from(self.ifd.count()).unwrap();
        num_entries_size(self.bigtiff)
            + count * entry_size(self.bigtiff)
            + offset_size(self.bigtiff)
    }

    pub fn write(&self, buf: &mut [u8], next_ifd_offset: u64) -> SaverResult<()> {
        // check if all entries can be written
        let mut to_dos = self.to_do().peekable();
        if to_dos.peek().is_some() {
            bail!(SaverError::unfinished_ifd(
                to_dos.map(|(k, _)| *k),
                "cannot write ifd yet, some tags still need to be externalized".into()
            ))
        }
        // check if the buffer is large enough
        if u64::try_from(buf.len()).unwrap() < self.required_len() {
            bail!(SaverError::invalid_buffer(
                self.required_len(),
                format!(
                    "ifd buffer of length {:?} too small, need {:?}",
                    buf.len(),
                    self.required_len()
                )
            ))
        }
        // we can unwrap here, because the key type is u16, so it cannot overflow u64
        let count = u64::try_from(self.ifd.count()).unwrap();
        // all good: start writing
        let mut offset = if self.bigtiff {
            TagData::Long8(smallvec![count])
        } else {
            // we can unwrap here, because the key type is u16, so it cannot overflow u16
            TagData::Short(smallvec![u16::try_from(count).unwrap()])
        }
        .to_buffer(buf, self.byte_order);
        // Spec says we MUST write tags in-order, ifd.iter() is in-order
        for (tag, entry) in self.ifd.iter() {
            // tag and tag type
            offset += TagData::Short(smallvec![tag.to_u16(), entry.tag_type().to_u16()])
                .to_buffer(&mut buf[offset..], self.byte_order);

            // count
            offset += if self.bigtiff {
                TagData::Long8(smallvec![entry.count()])
            } else {
                // TODO: this should error
                TagData::Long(smallvec![u32::try_from(entry.count()).or_raise(|| {
                    SaverError::need_bigtiff(format!("entry count {} overflows u32", entry.count()))
                })?])
            }
            .to_buffer(&mut buf[offset..], self.byte_order);

            offset += match entry {
                IfdEntry::Value(v) => {
                    v.to_buffer(&mut buf[offset..], self.byte_order);
                    offset_size(self.bigtiff) as usize
                }
                IfdEntry::Offset(o) => {
                    if o.len() <= offset_size(self.bigtiff) {
                        bail!(SaverError::permanent(format!(
                            "{o:?} for tag {:?} fits as value, but is an offset",
                            tag
                        )))
                    }
                    if self.bigtiff {
                        TagData::Long8(smallvec![o.offset])
                    } else {
                        TagData::Long(smallvec![u32::try_from(o.offset).or_raise(|| {
                            SaverError::need_bigtiff(format!(
                                "entry offset {} overflows u32",
                                o.offset
                            ))
                        })?])
                    }
                    .to_buffer(&mut buf[offset..], self.byte_order)
                }
            };
        }
        offset += if self.bigtiff {
            TagData::Long8(smallvec![next_ifd_offset])
        } else {
            TagData::Long(smallvec![u32::try_from(next_ifd_offset).or_raise(
                || SaverError::need_bigtiff(format!(
                    "next ifd offset {next_ifd_offset} overflows u32"
                ))
            )?])
        }
        .to_buffer(&mut buf[offset..], self.byte_order);
        if u64::try_from(offset).unwrap() != self.required_len() {
            unreachable!("this is really bad, pleas open an issue")
        }
        Ok(())
    }
}

#[allow(unused_imports, clippy::useless_conversion)]
mod test_ifd {
    use std::collections::BTreeMap;

    use smallvec::smallvec;

    use super::*;
    use crate::{
        structs::{Offset, TagData, TagType},
        NATIVE_ENDIAN,
    };

    #[test]
    fn test_sanity() {
        assert_eq!((0..42).len(), 42);
    }

    /// test writing multiple tags, esp. whether we skip over the offset properly
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
                    (Tag::from_u16_exhaustive(0x0101), IfdEntry::Value(res1)),
                    (Tag::from_u16_exhaustive(0x0100), IfdEntry::Value(res2))
                ])
            };
            let mut res = vec![0;buf.len()];
            IfdSaver::from_ifd(0, ifd, false, ByteOrder::LittleEndian).write(&mut res, 0).unwrap();
            assert_eq!(&res, &buf);
        }
    }

    #[test]
    fn test_todo_small() {
        let ifd_saver = IfdSaver::from_ifd(
            0,
            Ifd {
                data: BTreeMap::from([
                    // this one fits
                    (
                        Tag::ImageWidth,
                        IfdEntry::Value(TagData::Byte(smallvec![42;4])),
                    ),
                    // this one doesn't
                    (
                        Tag::ImageLength,
                        IfdEntry::Value(TagData::Byte(smallvec![42;5])),
                    ),
                ]),
            },
            false,
            NATIVE_ENDIAN,
        );
        let to_dos = &ifd_saver.to_do().map(|(k, v)| (*k, v)).collect::<Vec<_>>();
        assert_eq!(to_dos.len(), 1);
        assert_eq!(to_dos[0].0, Tag::ImageLength);
        assert_eq!(to_dos[0].1, &TagData::Byte(smallvec![42;5]));
    }

    #[test]
    fn test_todo_big() {
        let ifd_saver = IfdSaver::from_ifd(
            0,
            Ifd {
                data: BTreeMap::from([
                    // this one fits
                    (
                        Tag::ImageWidth,
                        IfdEntry::Value(TagData::Byte(smallvec![42;8])),
                    ),
                    // this one doesn't
                    (
                        Tag::ImageLength,
                        IfdEntry::Value(TagData::Byte(smallvec![42;9])),
                    ),
                ]),
            },
            true,
            NATIVE_ENDIAN,
        );
        let to_dos = &ifd_saver.to_do().map(|(k, v)| (*k, v)).collect::<Vec<_>>();
        assert_eq!(to_dos.len(), 1);
        assert_eq!(to_dos[0].0, Tag::ImageLength);
        assert_eq!(to_dos[0].1, &TagData::Byte(smallvec![42;9]));
    }

    // ---------
    // errors
    // ---------

    #[test]
    fn test_write_todo_small() {
        let ifd_saver = IfdSaver::from_ifd(
            0,
            Ifd {
                data: BTreeMap::from([
                    // this one fits
                    (
                        Tag::ImageWidth,
                        IfdEntry::Value(TagData::Byte(smallvec![42;4])),
                    ),
                    // this one doesn't
                    (
                        Tag::ImageLength,
                        IfdEntry::Value(TagData::Byte(smallvec![42;5])),
                    ),
                ]),
            },
            false,
            NATIVE_ENDIAN,
        );
        let mut buf = vec![0; ifd_saver.required_len() as usize];
        assert_eq!(
            ifd_saver
                .write(&mut buf, 0)
                .unwrap_err()
                .frame()
                .error()
                .downcast_ref::<SaverError>()
                .unwrap(),
            &SaverError::unfinished_ifd(
                ifd_saver.to_do().map(|(k, _)| *k),
                "cannot write ifd yet, some tags still need to be externalized".into()
            )
        );
    }

    #[test]
    fn test_write_todo_big() {
        let ifd_saver = IfdSaver::from_ifd(
            0,
            Ifd {
                data: BTreeMap::from([
                    // this one fits
                    (
                        Tag::ImageWidth,
                        IfdEntry::Value(TagData::Byte(smallvec![42;8])),
                    ),
                    // this one doesn't
                    (
                        Tag::ImageLength,
                        IfdEntry::Value(TagData::Byte(smallvec![42;9])),
                    ),
                ]),
            },
            true,
            NATIVE_ENDIAN,
        );
        // let to_dos = ;
        let mut buf = vec![0; ifd_saver.required_len() as usize];
        assert_eq!(
            ifd_saver
                .write(&mut buf, 0)
                .unwrap_err()
                .frame()
                .error()
                .downcast_ref::<SaverError>()
                .unwrap(),
            &SaverError::unfinished_ifd(
                ifd_saver.to_do().map(|(k, _)| *k),
                "cannot write ifd yet, some tags still need to be externalized".into()
            )
        );
    }

    #[test]
    fn test_write_fitting_offset_small() {
        let ifd_saver = IfdSaver::from_ifd(
            0,
            Ifd {
                data: BTreeMap::from([
                    // this one doesn't fit
                    (
                        Tag::ImageWidth,
                        IfdEntry::Offset(Offset {
                            tag_type: TagType::BYTE,
                            count: 5,
                            offset: 42,
                        }),
                    ),
                    // this one does
                    (
                        Tag::ImageLength,
                        IfdEntry::Offset(Offset {
                            tag_type: TagType::BYTE,
                            count: 4,
                            offset: 42,
                        }),
                    ),
                ]),
            },
            false,
            NATIVE_ENDIAN,
        );
        let mut buf = vec![0; ifd_saver.required_len() as usize];
        assert_eq!(
            ifd_saver
                .write(&mut buf, 0)
                .unwrap_err()
                .frame()
                .error()
                .downcast_ref::<SaverError>()
                .unwrap(),
            &SaverError::permanent(
                "Offset { tag_type: BYTE, count: 4, offset: 42 } for tag ImageLength fits as value, but is an offset"
                    .into()
            )
        );
    }

    #[test]
    fn test_write_fitting_offset_big() {
        let ifd_saver = IfdSaver::from_ifd(
            0,
            Ifd {
                data: BTreeMap::from([
                    // this one doesn't fit
                    (
                        Tag::ImageWidth,
                        IfdEntry::Offset(Offset {
                            tag_type: TagType::BYTE,
                            count: 9,
                            offset: 42,
                        }),
                    ),
                    // this one does
                    (
                        Tag::ImageLength,
                        IfdEntry::Offset(Offset {
                            tag_type: TagType::BYTE,
                            count: 8,
                            offset: 42,
                        }),
                    ),
                ]),
            },
            true,
            NATIVE_ENDIAN,
        );
        let mut buf = vec![0; ifd_saver.required_len() as usize];
        assert_eq!(
            ifd_saver
                .write(&mut buf, 0)
                .unwrap_err()
                .frame()
                .error()
                .downcast_ref::<SaverError>()
                .unwrap(),
            &SaverError::permanent(
                "Offset { tag_type: BYTE, count: 8, offset: 42 } for tag ImageLength fits as value, but is an offset"
                    .into()
            )
        );
    }

    #[test]
    fn test_write_need_big_entry_count() {
        let ifd_saver = IfdSaver::from_ifd(
            0,
            Ifd {
                data: BTreeMap::from([(
                    Tag::ImageWidth,
                    IfdEntry::Offset(Offset {
                        tag_type: TagType::BYTE,
                        count: 1 << 32,
                        offset: 42,
                    }),
                )]),
            },
            false,
            NATIVE_ENDIAN,
        );
        let mut buf = vec![0; ifd_saver.required_len() as usize];
        assert_eq!(
            ifd_saver
                .write(&mut buf, 0)
                .unwrap_err()
                .frame()
                .error()
                .downcast_ref::<SaverError>()
                .unwrap(),
            &SaverError::need_bigtiff(format!("entry count {} overflows u32", 1u64 << 32))
        );
    }

    #[test]
    fn test_write_need_big_entry_offset() {
        let ifd_saver = IfdSaver::from_ifd(
            0,
            Ifd {
                data: BTreeMap::from([(
                    Tag::ImageWidth,
                    IfdEntry::Offset(Offset {
                        tag_type: TagType::BYTE,
                        count: 42,
                        offset: 1 << 32,
                    }),
                )]),
            },
            false,
            NATIVE_ENDIAN,
        );
        let mut buf = vec![0; ifd_saver.required_len() as usize];
        assert_eq!(
            ifd_saver
                .write(&mut buf, 0)
                .unwrap_err()
                .frame()
                .error()
                .downcast_ref::<SaverError>()
                .unwrap(),
            &SaverError::need_bigtiff(format!("entry offset {} overflows u32", 1u64 << 32))
        );
    }

    #[test]
    fn test_write_need_big_next_ifd_offset() {
        let ifd_saver = IfdSaver::from_ifd(
            0,
            Ifd {
                data: BTreeMap::from([]),
            },
            false,
            NATIVE_ENDIAN,
        );
        let mut buf = vec![0; ifd_saver.required_len() as usize];
        assert_eq!(
            ifd_saver
                .write(&mut buf, 1 << 32)
                .unwrap_err()
                .frame()
                .error()
                .downcast_ref::<SaverError>()
                .unwrap(),
            &SaverError::need_bigtiff(format!("next ifd offset {} overflows u32", 1u64 << 32))
        );
    }

    #[test]
    fn test_write_too_small_buf() {
        let ifd_saver = IfdSaver::from_ifd(
            0,
            Ifd {
                data: BTreeMap::from([]),
            },
            false,
            NATIVE_ENDIAN,
        );
        let mut buf = vec![0; ifd_saver.required_len() as usize - 1];
        assert_eq!(
            ifd_saver
                .write(&mut buf, 0)
                .unwrap_err()
                .frame()
                .error()
                .downcast_ref::<SaverError>()
                .unwrap(),
            &SaverError::invalid_buffer(
                ifd_saver.required_len(),
                format!("ifd buffer of length 5 too small, need 6")
            )
        );
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
        //       tag type  count      offset    next ifd
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
            let mut res = vec![0;buf.len()];
            IfdSaver::from_ifd(0, ifd, false, byte_order).write(&mut res, 0).unwrap();
            assert_eq!(&res, &buf);
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
        //                 tag   type       count            offset                next ifd
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
        // we special-case IFD
        ];
        for (buf, byte_order, data) in cases {
            println!("Trying {data:?} with {byte_order:?}, should become  {buf:?}");
            let ifd = Ifd {
                data: BTreeMap::from([(Tag::from_u16_exhaustive(0x01_01), IfdEntry::Value(data))])
            };
            let mut res = vec![0;buf.len()];
            IfdSaver::from_ifd(0, ifd, true, byte_order).write(&mut res, 0).unwrap();
            assert_eq!(&res, &buf);
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
        //  tag type  count    offset      next ifd
        //  // /  \  /     \   /     \     /     \
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
        // others don't fit, neither 8-types and we special-case IFD
        ];
        for (buf, byte_order, data) in cases {
            println!("Trying {data:?} with {byte_order:?}, should become  {buf:?}");
            let ifd = Ifd {
                data: BTreeMap::from([(Tag::from_u16_exhaustive(0x01_01), IfdEntry::Value(data))])
            };
            let mut res = vec![0;buf.len()];
            IfdSaver::from_ifd(0, ifd, false, byte_order).write(&mut res, 0).unwrap();
            assert_eq!(&res, &buf);
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
        //                 tag   type       count            offset
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
        // we special-case IFD
        ];
        for (buf, byte_order, data) in cases {
            println!("Trying {data:?} with {byte_order:?}, should become  {buf:?}");
            let ifd = Ifd {
                data: BTreeMap::from([(Tag::from_u16_exhaustive(0x01_01), IfdEntry::Value(data))])
            };
            let mut res = vec![0;buf.len()];
            IfdSaver::from_ifd(0, ifd, true, byte_order).write(&mut res, 0).unwrap();
            assert_eq!(&res, &buf);
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
        //       tag type  count    offset      next ifd
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
        // Double doesn't fit, neither 8-types and we special-case IFD
        ];
        for (buf, byte_order, count, tag_type) in cases {
            println!("Trying {tag_type:?} with {byte_order:?}, should become  {buf:?}");
            let ifd = Ifd {
                data: BTreeMap::from([(Tag::from_u16_exhaustive(0x01_01), IfdEntry::Offset(Offset { tag_type, count, offset: 42 }))])
            };
            let mut res = vec![0;buf.len()];
            IfdSaver::from_ifd(0, ifd, false, byte_order).write(&mut res, 0).unwrap();
            assert_eq!(&res, &buf);
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
        //1,0,0,0,0,0,0,0, tag   type       count            offset
        //0,0,0,0,0,0,0,1,  //  /   \ 1 2 3 4 5 6 7 8   1  2  3  4  5  6  7  8  1 2 3 4 5 6 7 8
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
        // we special-case IFD
        ];
        for (buf, byte_order, count, tag_type) in cases {
            println!("Trying {tag_type:?} with {byte_order:?}, should become  {buf:?}");
            let ifd = Ifd {
                data: BTreeMap::from([(Tag::from_u16_exhaustive(0x01_01), IfdEntry::Offset(Offset { tag_type, count, offset: 42 }))])
            };
            let mut res = vec![0;buf.len()];
            IfdSaver::from_ifd(0, ifd, true, byte_order).write(&mut res, 0).unwrap();
            assert_eq!(&res, &buf);
        }
    }
}
