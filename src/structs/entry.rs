use crate::{
    decoder::EndianReader,
    error::{TiffError, TiffFormatError, TiffResult, UsageError},
    tags::{
        Tag,
        TagType::{
            self,
            // self, ASCII, BYTE, DOUBLE, FLOAT, IFD, IFD8, LONG, RATIONAL, SBYTE, SHORT, SLONG,
            // SRATIONAL, SSHORT, UNDEFINED, LONG8,
        },
    },
    util::fix_endianness,
    value::Value,
    ByteOrder,
};

use std::{
    collections::BTreeMap,
    io::{self, Read},
};
pub type Directory = BTreeMap<Tag, IfdEntry>;

///
#[derive(Debug, PartialEq)]
pub enum IfdEntry {
    Offset {
        tag_type: TagType,
        count: u64,
        offset: u64,
    },
    Value(Value),
}

impl IfdEntry {
    /// Create this entry from an EndianReader
    ///
    /// The reader should have its cursor at the start of tag_type, not at tag
    ///
    /// If the value fits in the offset field, it will be converted
    /// ```
    /// # use tiff2::ByteOrder;
    /// # use tiff2::{tags::TagType, value::Value, entry::IfdEntry, decoder::EndianReader};
    /// let entry_buf = [
    ///     0x03, 0x00,                         // Type (SHORT)
    ///     0x01, 0x00, 0x00, 0x00,             // Count (1)
    ///     0x2C, 0x01, 0x00, 0x00,             // Offset = Value (300)
    /// ];
    /// let mut r = EndianReader::wrap(std::io::Cursor::new(entry_buf), ByteOrder::LittleEndian);
    /// assert_eq!(IfdEntry::from_reader(&mut r, false).unwrap(), IfdEntry::Value(Value::Short(300)));
    /// ```
    /// Otherwise an offset is saved
    /// ```
    /// # use tiff2::ByteOrder;
    /// # use tiff2::{tags::TagType, value::Value, entry::IfdEntry, decoder::EndianReader};
    /// let entry_buf = [
    ///     0x03, 0x00,                         // Type (SHORT)
    ///     0x03, 0x00, 0x00, 0x00,             // Count (3)
    ///     0x2C, 0x01, 0x00, 0x00,             // Offset = Value (300)
    /// ];
    /// let mut r = EndianReader::wrap(std::io::Cursor::new(entry_buf), ByteOrder::LittleEndian);
    /// assert_eq!(IfdEntry::from_reader(&mut r, false).unwrap(), IfdEntry::Offset{
    ///     tag_type: TagType::SHORT,
    ///     count: 3,
    ///     offset: 300,
    /// });
    /// ```
    pub fn from_reader<R: Read>(r: &mut EndianReader<R>, bigtiff: bool) -> TiffResult<Self> {
        let t_u16 = r.read_u16()?;
        let tag_type =
            TagType::from_u16(t_u16).ok_or(TiffFormatError::InvalidTagValueType(t_u16))?;
        let count: u64 = if bigtiff {
            r.read_u64()?
        } else {
            r.read_u32()?.into()
        };
        let Some(value_bytes) = count.checked_mul(tag_type.size().try_into()?) else {
            return Err(TiffError::LimitsExceeded);
        };
        if !bigtiff && value_bytes > 4
            || value_bytes > 8
            || tag_type == TagType::IFD
            || tag_type == TagType::IFD8
        {
            // we are too big, just insert the offset for now
            Ok(IfdEntry::Offset {
                tag_type,
                count,
                offset: if bigtiff {
                    r.read_u64()?
                } else {
                    r.read_u32()?.into()
                },
            })
        } else {
            let mut offset = vec![0u8; usize::try_from(count)? * tag_type.size()];
            r.read_exact(&mut offset)?;
            fix_endianness(&mut offset, r.byte_order, 8 * tag_type.primitive_size());
            Ok(IfdEntry::Value(
                BufferedEntry {
                    tag_type,
                    count,
                    data: offset,
                }
                .try_into()?,
            ))
        }
    }
}

/// Entry with buffered data.
///
/// Should not be used for tags where the data fits in the offset field
/// byte-order of the data should be native-endian in this buffer
#[derive(Debug, PartialEq)]
pub struct BufferedEntry {
    pub tag_type: TagType,
    pub count: u64,
    pub data: Vec<u8>,
}

// macro_rules! entry_try_into_unsigned {
//     ($type:ty) => {
//         #[rustfmt::skip]
//         impl TryInto<$type> for BufferedEntry {
//             type Error = TiffError;

//             fn try_into(self) -> Result<$type, Self::Error> {
//                 match self.tag_type {
//                     BYTE =>         <$type>::try_from(bytemuck::try_cast::<_, u8 >(self.data)?).into(),
//                     SHORT =>        <$type>::try_from(bytemuck::try_cast::<_, u16>(self.data)?).into(),
//                     IFD | LONG =>   <$type>::try_from(bytemuck::try_cast::<_, u32>(self.data)?).into(),
//                     IFD8 | LONG8 => <$type>::try_from(bytemuck::try_cast::<_, u64>(self.data)?).into(),
//                     _ => Err(TiffFormatError::UnsignedIntegerExpected(self)),
//                 }
//             }
//         }
//     };
// }

// entry_try_into_unsigned!(u8);
// entry_try_into_unsigned!(u16);
// entry_try_into_unsigned!(u32);
// entry_try_into_unsigned!(u64);


macro_rules! entry_try_into_signed {
    ($type:ty) => {
        #[rustfmt::skip]
        impl TryInto<$type> for BufferedEntry {
            type Error = TiffError;

            fn try_into(self) -> Result<$type, Self::Error> {
                if self.data.len() != usize::try_from(self.count)? * self.tag_type.size() {
                    return Err(TiffFormatError::InconsistentSizesEncountered.into());
                }
                match self.tag_type {
                    TagType::SBYTE  => Ok(<$type>::try_from(bytemuck::try_cast::<_, i8 >(<[u8; 8 ]>::try_from(self.data.as_slice()).unwrap()).unwrap()).unwrap()),
                    TagType::SSHORT => Ok(<$type>::try_from(bytemuck::try_cast::<_, i16>(<[u8; 16]>::try_from(self.data.as_slice()).unwrap()).unwrap()).unwrap()),
                    TagType::SLONG  => Ok(<$type>::try_from(bytemuck::try_cast::<_, i32>(<[u8; 32]>::try_from(self.data.as_slice()).unwrap()).unwrap()).unwrap()),
                    TagType::SLONG8 => Ok(<$type>::try_from(bytemuck::try_cast::<_, i64>(<[u8; 64]>::try_from(self.data.as_slice()).unwrap()).unwrap()).unwrap()),
                    _ => Err(TiffFormatError::InconsistentSizesEncountered.into())//UnsignedIntegerExpected(self).into()),
                }
            }
        }
    };
}

entry_try_into_signed!(i8);
entry_try_into_signed!(i16);
entry_try_into_signed!(i32);
entry_try_into_signed!(i64);

fn from_single(tag_type: TagType, data: &[u8]) -> TiffResult<Value> {
    Ok(match tag_type {
        TagType::BYTE => Value::Byte(data[0]),
        TagType::SBYTE => Value::SignedByte(data[0] as i8),
        TagType::UNDEFINED => Value::Undefined(data[0]),

        TagType::SHORT => Value::Short(u16::from_ne_bytes(data[..2].try_into().unwrap())),
        TagType::SSHORT => Value::SShort(i16::from_ne_bytes(data[..2].try_into().unwrap())),

        TagType::LONG => Value::Long(u32::from_ne_bytes(data[..4].try_into().unwrap())),
        TagType::SLONG => Value::SLong(i32::from_ne_bytes(data[..4].try_into().unwrap())),

        TagType::LONG8 => Value::Long8(u64::from_ne_bytes(data[..8].try_into().unwrap())),
        TagType::SLONG8 => Value::SLong8(i64::from_ne_bytes(data[..8].try_into().unwrap())),

        TagType::RATIONAL => Value::Rational(
            u32::from_ne_bytes(data[..4].try_into().unwrap()),
            u32::from_ne_bytes(data[4..8].try_into().unwrap()),
        ),
        TagType::SRATIONAL => Value::SRational(
            i32::from_ne_bytes(data[..4].try_into().unwrap()),
            i32::from_ne_bytes(data[4..8].try_into().unwrap()),
        ),
        TagType::FLOAT => Value::Float(f32::from_ne_bytes(data[..4].try_into().unwrap())),
        TagType::DOUBLE => Value::Double(f64::from_ne_bytes(data[..8].try_into().unwrap())),

        TagType::ASCII => {
            if data[0] == 0 {
                Value::Ascii("".to_string())
            } else {
                return Err(TiffFormatError::InvalidTag.into());
            }
        }
        TagType::IFD | TagType::IFD8 => return Err(UsageError::IfdReadIntoEntry.into()),
    })
}

impl TryFrom<BufferedEntry> for Value {
    type Error = TiffError;
    fn try_from(entry: BufferedEntry) -> Result<Self, TiffError> {
        if entry.count == 1 {
            Ok(from_single(entry.tag_type, &entry.data)?)
        } else if entry.tag_type == TagType::ASCII {
            println!("decoding ascii value: {:?}", entry.data);
            if entry.data.is_ascii() && entry.data.ends_with(&[0]) {
                let v = std::str::from_utf8(&entry.data)?;
                let v = v.trim_matches(char::from(0));
                Ok(Value::Ascii(v.into()))
            } else {
                Err(TiffFormatError::InvalidTag.into())
            }
        } else {
            Ok(Value::List(
                entry
                    .data
                    .chunks_exact(entry.tag_type.size())
                    .map(|chunk| from_single(entry.tag_type, chunk))
                    .collect::<TiffResult<Vec<Value>>>()?,
            ))
        }
    }
}

mod test_entry {
    use super::*;

    // -----------------------------------------------------------------
    // tests below are copy-pasted from Ifd. Make sure to update there
    // accordingly
    // -----------------------------------------------------------------

    #[test]
    #[rustfmt::skip]
    fn test_single_fits_notbig() {
        // personal sanity checks
        assert_eq!(u16::from_le_bytes([42,0]),42);
        assert_eq!(u16::from_be_bytes([0,42]),42);
        assert_eq!(f32::from_le_bytes([0x42,0,0,0]),f32::from_bits(0x00_00_00_42));
        assert_eq!(f32::from_be_bytes([0,0,0,0x42]),f32::from_bits(0x00_00_00_42));
        let cases = [
        // type   count    offset
        // / \  /     \   /     \
        ([1, 0, 1,0,0,0, 42, 0, 0, 0], ByteOrder::LittleEndian, Value::Byte      (42)                ),
        ([0, 1, 0,0,0,1, 42, 0, 0, 0], ByteOrder::BigEndian,    Value::Byte      (42)                ),
        ([6, 0, 1,0,0,0, 42, 0, 0, 0], ByteOrder::LittleEndian, Value::SignedByte(42)                ),
        ([0, 6, 0,0,0,1, 42, 0, 0, 0], ByteOrder::BigEndian,    Value::SignedByte(42)                ),
        ([7, 0, 1,0,0,0, 42, 0, 0, 0], ByteOrder::LittleEndian, Value::Undefined (42)                ),
        ([0, 7, 0,0,0,1, 42, 0, 0, 0], ByteOrder::BigEndian,    Value::Undefined (42)                ),
        ([2, 0, 1,0,0,0,  0, 0, 0, 0], ByteOrder::LittleEndian, Value::Ascii     ("".into())         ),
        ([0, 2, 0,0,0,1,  0, 0, 0, 0], ByteOrder::BigEndian,    Value::Ascii     ("".into())         ),
        ([3, 0, 1,0,0,0, 42, 0, 0, 0], ByteOrder::LittleEndian, Value::Short     (42)                ),
        ([0, 3, 0,0,0,1,  0,42, 0, 0], ByteOrder::BigEndian,    Value::Short     (42)                ),
        ([8, 0, 1,0,0,0, 42, 0, 0, 0], ByteOrder::LittleEndian, Value::SShort    (42)                ),
        ([0, 8, 0,0,0,1,  0,42, 0, 0], ByteOrder::BigEndian,    Value::SShort    (42)                ),
        ([4, 0, 1,0,0,0, 42, 0, 0, 0], ByteOrder::LittleEndian, Value::Long      (42)                ),
        ([0, 4, 0,0,0,1,  0, 0, 0,42], ByteOrder::BigEndian,    Value::Long      (42)                ),
        ([9, 0, 1,0,0,0, 42, 0, 0, 0], ByteOrder::LittleEndian, Value::SLong     (42)                ),
        ([0, 9, 0,0,0,1,  0, 0, 0,42], ByteOrder::BigEndian,    Value::SLong     (42)                ),
        ([11,0, 1,0,0,0, 42, 0, 0, 0], ByteOrder::LittleEndian, Value::Float     (f32::from_bits(42))),
        ([0,11, 0,0,0,1,  0, 0, 0,42], ByteOrder::BigEndian,    Value::Float     (f32::from_bits(42))),
        // Double doesn't fit, neither 8-types and we special-case IFD
        ];
        for (buf, byte_order, res) in cases {
            let mut r = EndianReader::wrap(io::Cursor::new(buf), byte_order);
            assert_eq!(IfdEntry::from_reader(&mut r, false).unwrap(), IfdEntry::Value(res));
        }
    }

    #[test]
    #[rustfmt::skip]
    fn test_single_fits_big() {
        // personal sanity checks
        assert_eq!(u16::from_le_bytes([42,0]),42);
        assert_eq!(u16::from_be_bytes([0,42]),42);

        assert_eq!(f32::from_le_bytes([0x42,0,0,0]),f32::from_bits(0x00_00_00_42));
        assert_eq!(f32::from_be_bytes([0,0,0,0x42]),f32::from_bits(0x00_00_00_42));
        let cases = [
        //type       count            offset
        // / \  1 2 3 4 5 6 7 8   1  2  3  4  5  6  7  8
        ([1, 0, 1,0,0,0,0,0,0,0, 42, 0, 0, 0, 0, 0, 0, 0], ByteOrder::LittleEndian, Value::Byte      (42)                ),
        ([0, 1, 0,0,0,0,0,0,0,1, 42, 0, 0, 0, 0, 0, 0, 0], ByteOrder::BigEndian,    Value::Byte      (42)                ),
        ([6, 0, 1,0,0,0,0,0,0,0, 42, 0, 0, 0, 0, 0, 0, 0], ByteOrder::LittleEndian, Value::SignedByte(42)                ),
        ([0, 6, 0,0,0,0,0,0,0,1, 42, 0, 0, 0, 0, 0, 0, 0], ByteOrder::BigEndian,    Value::SignedByte(42)                ),
        ([7, 0, 1,0,0,0,0,0,0,0, 42, 0, 0, 0, 0, 0, 0, 0], ByteOrder::LittleEndian, Value::Undefined (42)                ),
        ([0, 7, 0,0,0,0,0,0,0,1, 42, 0, 0, 0, 0, 0, 0, 0], ByteOrder::BigEndian,    Value::Undefined (42)                ),
        ([2, 0, 1,0,0,0,0,0,0,0,  0, 0, 0, 0, 0, 0, 0, 0], ByteOrder::LittleEndian, Value::Ascii     ("".into())         ),
        ([0, 2, 0,0,0,0,0,0,0,1,  0, 0, 0, 0, 0, 0, 0, 0], ByteOrder::BigEndian,    Value::Ascii     ("".into())         ),
        ([3, 0, 1,0,0,0,0,0,0,0, 42, 0, 0, 0, 0, 0, 0, 0], ByteOrder::LittleEndian, Value::Short     (42)                ),
        ([0, 3, 0,0,0,0,0,0,0,1,  0,42, 0, 0, 0, 0, 0, 0], ByteOrder::BigEndian,    Value::Short     (42)                ),
        ([8, 0, 1,0,0,0,0,0,0,0, 42, 0, 0, 0, 0, 0, 0, 0], ByteOrder::LittleEndian, Value::SShort    (42)                ),
        ([0, 8, 0,0,0,0,0,0,0,1,  0,42, 0, 0, 0, 0, 0, 0], ByteOrder::BigEndian,    Value::SShort    (42)                ),
        ([4, 0, 1,0,0,0,0,0,0,0, 42, 0, 0, 0, 0, 0, 0, 0], ByteOrder::LittleEndian, Value::Long      (42)                ),
        ([0, 4, 0,0,0,0,0,0,0,1,  0, 0, 0,42, 0, 0, 0, 0], ByteOrder::BigEndian,    Value::Long      (42)                ),
        ([9, 0, 1,0,0,0,0,0,0,0, 42, 0, 0, 0, 0, 0, 0, 0], ByteOrder::LittleEndian, Value::SLong     (42)                ),
        ([0, 9, 0,0,0,0,0,0,0,1,  0, 0, 0,42, 0, 0, 0, 0], ByteOrder::BigEndian,    Value::SLong     (42)                ),
        ([11,0, 1,0,0,0,0,0,0,0, 42, 0, 0, 0, 0, 0, 0, 0], ByteOrder::LittleEndian, Value::Float     (f32::from_bits(42))),
        ([0,11, 0,0,0,0,0,0,0,1,  0, 0, 0,42, 0, 0, 0, 0], ByteOrder::BigEndian,    Value::Float     (f32::from_bits(42))),
        ([12,0, 1,0,0,0,0,0,0,0, 42, 0, 0, 0, 0, 0, 0, 0], ByteOrder::LittleEndian, Value::Double    (f64::from_bits(42))),
        ([0,12, 0,0,0,0,0,0,0,1,  0, 0, 0, 0, 0, 0, 0,42], ByteOrder::BigEndian,    Value::Double    (f64::from_bits(42))),
        ([5, 0, 1,0,0,0,0,0,0,0,  42,0, 0, 0,13, 0, 0, 0], ByteOrder::LittleEndian, Value::Rational  (42, 13)            ),
        ([0, 5, 0,0,0,0,0,0,0,1,  0, 0, 0,42, 0, 0, 0,13], ByteOrder::BigEndian,    Value::Rational  (42, 13)            ),
        ([10,0, 1,0,0,0,0,0,0,0, 42, 0, 0, 0,13, 0, 0, 0], ByteOrder::LittleEndian, Value::SRational (42, 13)            ),
        ([0,10, 0,0,0,0,0,0,0,1,  0, 0, 0,42, 0, 0, 0,13], ByteOrder::BigEndian,    Value::SRational (42, 13)            ),
        // we special-case IFD
        ];
        for (buf, byte_order, res) in cases {
            let mut r = EndianReader::wrap(io::Cursor::new(buf), byte_order);
            assert_eq!(IfdEntry::from_reader(&mut r, true).unwrap(), IfdEntry::Value(res));
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
        //n_tags tag type  count    offset
        // //    // /  \  /     \   /     \
        ([1, 0, 4,0,0,0, 42,42,42,42], ByteOrder::LittleEndian, Value::List(vec![Value::Byte      (42); 4])     ),
        ([0, 1, 0,0,0,4, 42,42,42,42], ByteOrder::BigEndian,    Value::List(vec![Value::Byte      (42); 4])     ),
        ([6, 0, 4,0,0,0, 42,42,42,42], ByteOrder::LittleEndian, Value::List(vec![Value::SignedByte(42); 4])     ),
        ([0, 6, 0,0,0,4, 42,42,42,42], ByteOrder::BigEndian,    Value::List(vec![Value::SignedByte(42); 4])     ),
        ([7, 0, 4,0,0,0, 42,42,42,42], ByteOrder::LittleEndian, Value::List(vec![Value::Undefined (42); 4])     ),
        ([0, 7, 0,0,0,4, 42,42,42,42], ByteOrder::BigEndian,    Value::List(vec![Value::Undefined (42); 4])     ),
        ([2, 0, 4,0,0,0, 42,42,42, 0], ByteOrder::LittleEndian, Value::Ascii                      ("***".into())),
        ([0, 2, 0,0,0,4, 42,42,42, 0], ByteOrder::BigEndian,    Value::Ascii                      ("***".into())),
        ([3, 0, 2,0,0,0, 42, 0,42, 0], ByteOrder::LittleEndian, Value::List(vec![Value::Short     (42); 2])     ),
        ([0, 3, 0,0,0,2,  0,42, 0,42], ByteOrder::BigEndian,    Value::List(vec![Value::Short     (42); 2])     ),
        ([8, 0, 2,0,0,0, 42, 0,42, 0], ByteOrder::LittleEndian, Value::List(vec![Value::SShort    (42); 2])     ),
        ([0, 8, 0,0,0,2,  0,42, 0,42], ByteOrder::BigEndian,    Value::List(vec![Value::SShort    (42); 2])     ),

        ([0, 2, 0,0,0,4, b'A',b'B',b'C',0], ByteOrder::BigEndian, Value::Ascii("ABC".into())),
        // others don't fit, neither 8-types and we special-case IFD
        ];
        for (buf, byte_order, res) in cases {
            let mut r = EndianReader::wrap(io::Cursor::new(buf), byte_order);
            assert_eq!(IfdEntry::from_reader(&mut r, false).unwrap(), IfdEntry::Value(res));
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
        // type       count            offset
        // / \  1 2 3 4 5 6 7 8   1  2  3  4  5  6  7  8
        ([1, 0, 8,0,0,0,0,0,0,0, 42,42,42,42,42,42,42,42], ByteOrder::LittleEndian, Value::List(vec![Value::Byte      (42)                ; 8])),
        ([0, 1, 0,0,0,0,0,0,0,8, 42,42,42,42,42,42,42,42], ByteOrder::BigEndian,    Value::List(vec![Value::Byte      (42)                ; 8])),
        ([6, 0, 8,0,0,0,0,0,0,0, 42,42,42,42,42,42,42,42], ByteOrder::LittleEndian, Value::List(vec![Value::SignedByte(42)                ; 8])),
        ([0, 6, 0,0,0,0,0,0,0,8, 42,42,42,42,42,42,42,42], ByteOrder::BigEndian,    Value::List(vec![Value::SignedByte(42)                ; 8])),
        ([7, 0, 8,0,0,0,0,0,0,0, 42,42,42,42,42,42,42,42], ByteOrder::LittleEndian, Value::List(vec![Value::Undefined (42)                ; 8])),
        ([0, 7, 0,0,0,0,0,0,0,8, 42,42,42,42,42,42,42,42], ByteOrder::BigEndian,    Value::List(vec![Value::Undefined (42)                ; 8])),
        ([2, 0, 8,0,0,0,0,0,0,0, 42,42,42,42,42,42,42, 0], ByteOrder::LittleEndian, Value::Ascii                      ("*******".into())       ),
        ([0, 2, 0,0,0,0,0,0,0,8, 42,42,42,42,42,42,42, 0], ByteOrder::BigEndian,    Value::Ascii                      ("*******".into())       ),
        ([3, 0, 4,0,0,0,0,0,0,0, 42, 0,42, 0,42, 0,42, 0], ByteOrder::LittleEndian, Value::List(vec![Value::Short     (42)                ; 4])),
        ([0, 3, 0,0,0,0,0,0,0,4,  0,42, 0,42, 0,42, 0,42], ByteOrder::BigEndian,    Value::List(vec![Value::Short     (42)                ; 4])),
        ([8, 0, 4,0,0,0,0,0,0,0, 42, 0,42, 0,42, 0,42, 0], ByteOrder::LittleEndian, Value::List(vec![Value::SShort    (42)                ; 4])),
        ([0, 8, 0,0,0,0,0,0,0,4,  0,42, 0,42, 0,42, 0,42], ByteOrder::BigEndian,    Value::List(vec![Value::SShort    (42)                ; 4])),
        ([4, 0, 2,0,0,0,0,0,0,0, 42, 0, 0, 0,42, 0, 0, 0], ByteOrder::LittleEndian, Value::List(vec![Value::Long      (42)                ; 2])),
        ([0, 4, 0,0,0,0,0,0,0,2,  0, 0, 0,42, 0, 0, 0,42], ByteOrder::BigEndian,    Value::List(vec![Value::Long      (42)                ; 2])),
        ([9, 0, 2,0,0,0,0,0,0,0, 42, 0, 0, 0,42, 0, 0, 0], ByteOrder::LittleEndian, Value::List(vec![Value::SLong     (42)                ; 2])),
        ([0, 9, 0,0,0,0,0,0,0,2,  0, 0, 0,42, 0, 0, 0,42], ByteOrder::BigEndian,    Value::List(vec![Value::SLong     (42)                ; 2])),
        ([11,0, 2,0,0,0,0,0,0,0, 42, 0, 0, 0,42, 0, 0, 0], ByteOrder::LittleEndian, Value::List(vec![Value::Float     (f32::from_bits(42)); 2])),
        ([0,11, 0,0,0,0,0,0,0,2,  0, 0, 0,42, 0, 0, 0,42], ByteOrder::BigEndian,    Value::List(vec![Value::Float     (f32::from_bits(42)); 2])),
        // we special-case IFD
        ];
        for (buf, byte_order, res) in cases {
            let mut r = EndianReader::wrap(io::Cursor::new(buf), byte_order);
            assert_eq!(IfdEntry::from_reader(&mut r, true).unwrap(), IfdEntry::Value(res));
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
        // type  count    offset
        // /\   /     \   /     \
        ([1, 0, 5,0,0,0, 42, 0, 0, 0], ByteOrder::LittleEndian, 5, TagType::BYTE      ),
        ([0, 1, 0,0,0,5,  0, 0, 0,42], ByteOrder::BigEndian   , 5, TagType::BYTE      ),
        ([6, 0, 5,0,0,0, 42, 0, 0, 0], ByteOrder::LittleEndian, 5, TagType::SBYTE     ),
        ([0, 6, 0,0,0,5,  0, 0, 0,42], ByteOrder::BigEndian   , 5, TagType::SBYTE     ),
        ([7, 0, 5,0,0,0, 42, 0, 0, 0], ByteOrder::LittleEndian, 5, TagType::UNDEFINED ),
        ([0, 7, 0,0,0,5,  0, 0, 0,42], ByteOrder::BigEndian   , 5, TagType::UNDEFINED ),
        ([2, 0, 5,0,0,0, 42, 0, 0, 0], ByteOrder::LittleEndian, 5, TagType::ASCII     ),
        ([0, 2, 0,0,0,5,  0, 0, 0,42], ByteOrder::BigEndian   , 5, TagType::ASCII     ),
        ([3, 0, 3,0,0,0, 42, 0, 0, 0], ByteOrder::LittleEndian, 3, TagType::SHORT     ),
        ([0, 3, 0,0,0,3,  0, 0, 0,42], ByteOrder::BigEndian   , 3, TagType::SHORT     ),
        ([8, 0, 3,0,0,0, 42, 0, 0, 0], ByteOrder::LittleEndian, 3, TagType::SSHORT    ),
        ([0, 8, 0,0,0,3,  0, 0, 0,42], ByteOrder::BigEndian   , 3, TagType::SSHORT    ),
        ([4, 0, 2,0,0,0, 42, 0, 0, 0], ByteOrder::LittleEndian, 2, TagType::LONG      ),
        ([0, 4, 0,0,0,2,  0, 0, 0,42], ByteOrder::BigEndian   , 2, TagType::LONG      ),
        ([9, 0, 2,0,0,0, 42, 0, 0, 0], ByteOrder::LittleEndian, 2, TagType::SLONG     ),
        ([0, 9, 0,0,0,2,  0, 0, 0,42], ByteOrder::BigEndian   , 2, TagType::SLONG     ),
        ([11,0, 2,0,0,0, 42, 0, 0, 0], ByteOrder::LittleEndian, 2, TagType::FLOAT     ),
        ([0,11, 0,0,0,2,  0, 0, 0,42], ByteOrder::BigEndian   , 2, TagType::FLOAT     ),
        ([12,0, 1,0,0,0, 42, 0, 0, 0], ByteOrder::LittleEndian, 1, TagType::DOUBLE    ),
        ([0,12, 0,0,0,1,  0, 0, 0,42], ByteOrder::BigEndian   , 1, TagType::DOUBLE    ),
        ([5, 0, 1,0,0,0, 42, 0, 0, 0], ByteOrder::LittleEndian, 1, TagType::RATIONAL  ),
        ([0, 5, 0,0,0,1,  0, 0, 0,42], ByteOrder::BigEndian   , 1, TagType::RATIONAL  ),
        ([10,0, 1,0,0,0, 42, 0, 0, 0], ByteOrder::LittleEndian, 1, TagType::SRATIONAL  ),
        ([0,10, 0,0,0,1,  0, 0, 0,42], ByteOrder::BigEndian   , 1, TagType::SRATIONAL  ),
        // Double doesn't fit, neither 8-types and we special-case IFD
        ];
        for (buf, byte_order, count, tag_type) in cases {
            let mut r = EndianReader::wrap(io::Cursor::new(buf), byte_order);
            assert_eq!(IfdEntry::from_reader(&mut r, false).unwrap(), IfdEntry::Offset { tag_type, count, offset: 42 });
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
        // type       count            offset
        // / \  1 2 3 4 5 6 7 8   1  2  3  4  5  6  7  8
        ([1, 0, 9,0,0,0,0,0,0,0, 42, 0, 0, 0, 0, 0, 0, 0], ByteOrder::LittleEndian, 9, TagType::BYTE      ),
        ([0, 1, 0,0,0,0,0,0,0,9,  0, 0, 0, 0, 0, 0, 0,42], ByteOrder::BigEndian   , 9, TagType::BYTE      ),
        ([6, 0, 9,0,0,0,0,0,0,0, 42, 0, 0, 0, 0, 0, 0, 0], ByteOrder::LittleEndian, 9, TagType::SBYTE     ),
        ([0, 6, 0,0,0,0,0,0,0,9,  0, 0, 0, 0, 0, 0, 0,42], ByteOrder::BigEndian   , 9, TagType::SBYTE     ),
        ([7, 0, 9,0,0,0,0,0,0,0, 42, 0, 0, 0, 0, 0, 0, 0], ByteOrder::LittleEndian, 9, TagType::UNDEFINED ),
        ([0, 7, 0,0,0,0,0,0,0,9,  0, 0, 0, 0, 0, 0, 0,42], ByteOrder::BigEndian   , 9, TagType::UNDEFINED ),
        ([2, 0, 9,0,0,0,0,0,0,0, 42, 0, 0, 0, 0, 0, 0, 0], ByteOrder::LittleEndian, 9, TagType::ASCII     ),
        ([0, 2, 0,0,0,0,0,0,0,9,  0, 0, 0, 0, 0, 0, 0,42], ByteOrder::BigEndian   , 9, TagType::ASCII     ),
        ([3, 0, 5,0,0,0,0,0,0,0, 42, 0, 0, 0, 0, 0, 0, 0], ByteOrder::LittleEndian, 5, TagType::SHORT     ),
        ([0, 3, 0,0,0,0,0,0,0,5,  0, 0, 0, 0, 0, 0, 0,42], ByteOrder::BigEndian   , 5, TagType::SHORT     ),
        ([8, 0, 5,0,0,0,0,0,0,0, 42, 0, 0, 0, 0, 0, 0, 0], ByteOrder::LittleEndian, 5, TagType::SSHORT    ),
        ([0, 8, 0,0,0,0,0,0,0,5,  0, 0, 0, 0, 0, 0, 0,42], ByteOrder::BigEndian   , 5, TagType::SSHORT    ),
        ([4, 0, 3,0,0,0,0,0,0,0, 42, 0, 0, 0, 0, 0, 0, 0], ByteOrder::LittleEndian, 3, TagType::LONG      ),
        ([0, 4, 0,0,0,0,0,0,0,3,  0, 0, 0, 0, 0, 0, 0,42], ByteOrder::BigEndian   , 3, TagType::LONG      ),
        ([9, 0, 3,0,0,0,0,0,0,0, 42, 0, 0, 0, 0, 0, 0, 0], ByteOrder::LittleEndian, 3, TagType::SLONG     ),
        ([0, 9, 0,0,0,0,0,0,0,3,  0, 0, 0, 0, 0, 0, 0,42], ByteOrder::BigEndian   , 3, TagType::SLONG     ),
        ([11,0, 3,0,0,0,0,0,0,0, 42, 0, 0, 0, 0, 0, 0, 0], ByteOrder::LittleEndian, 3, TagType::FLOAT     ),
        ([0,11, 0,0,0,0,0,0,0,3,  0, 0, 0, 0, 0, 0, 0,42], ByteOrder::BigEndian   , 3, TagType::FLOAT     ),
        ([12,0, 2,0,0,0,0,0,0,0, 42, 0, 0, 0, 0, 0, 0, 0], ByteOrder::LittleEndian, 2, TagType::DOUBLE    ),
        ([0,12, 0,0,0,0,0,0,0,2,  0, 0, 0, 0, 0, 0, 0,42], ByteOrder::BigEndian   , 2, TagType::DOUBLE    ),
        ([5, 0, 2,0,0,0,0,0,0,0, 42, 0, 0, 0, 0, 0, 0, 0], ByteOrder::LittleEndian, 2, TagType::RATIONAL  ),
        ([0, 5, 0,0,0,0,0,0,0,2,  0, 0, 0, 0, 0, 0, 0,42], ByteOrder::BigEndian   , 2, TagType::RATIONAL  ),
        ([10,0, 2,0,0,0,0,0,0,0, 42, 0, 0, 0, 0, 0, 0, 0], ByteOrder::LittleEndian, 2, TagType::SRATIONAL  ),
        ([0,10, 0,0,0,0,0,0,0,2,  0, 0, 0, 0, 0, 0, 0,42], ByteOrder::BigEndian   , 2, TagType::SRATIONAL  ),
        // we special-case IFD
        ];
        for (buf, byte_order, count, tag_type) in cases {
            let mut r = EndianReader::wrap(io::Cursor::new(buf), byte_order);
            assert_eq!(IfdEntry::from_reader(&mut r, true).unwrap(), IfdEntry::Offset { tag_type, count, offset: 42 });
        }
    }
}
