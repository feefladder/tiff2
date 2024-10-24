use crate::{
    decoder::EndianReader,
    error::{TiffError, TiffFormatError, TiffResult, UsageError},
    structs::{
        value::Value,
        Tag,
        TagType::{
            self,
            // self, ASCII, BYTE, DOUBLE, FLOAT, IFD, IFD8, LONG, RATIONAL, SBYTE, SHORT, SLONG,
            // SRATIONAL, SSHORT, UNDEFINED, LONG8,
        },
    },
    util::fix_endianness,
};

use std::{collections::BTreeMap, io::Read};
pub type Directory = BTreeMap<Tag, IfdEntry>;

///
#[derive(Debug, PartialEq)]
pub enum IfdEntry {
    Offset {
        tag_type: TagType,
        count: u64,
        offset: u64,
    },
    Value(BufferedEntry),
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
            Ok(IfdEntry::Value(BufferedEntry {
                tag_type,
                count,
                data: offset,
            }))
        }
    }
}

/// Entry with buffered data.
///
/// Should not be used for tags where the data fits in the offset field
/// byte-order of the data should be native-endian in this buffer
#[derive(Debug, PartialEq, Clone)]
pub struct BufferedEntry {
    pub tag_type: TagType,
    pub count: u64,
    pub data: Vec<u8>,
}

impl BufferedEntry {
    pub fn data(&self) -> &[u8] {
        &self.data
    }

    pub fn new(tag_type: TagType, count: u64) -> TiffResult<Self> {
        Ok(BufferedEntry {
            tag_type,
            count,
            data: vec![0u8; tag_type.size() * usize::try_from(count)?],
        })
    }

    #[rustfmt::skip]
    pub fn get_u64(&self, index: usize) -> TiffResult<u64> {
            if usize::try_from(self.count)? <= index {
                return Err(TiffError::LimitsExceeded);
            }
            match self.tag_type {
                TagType::BYTE                  => Ok(<&[u8 ]>::try_from(self)?[index].into()),
                TagType::SHORT                 => Ok(<&[u16]>::try_from(self)?[index].into()),
                TagType::LONG  | TagType::IFD  => Ok(<&[u32]>::try_from(self)?[index].into()),
                TagType::LONG8 | TagType::IFD8 => Ok(<&[u64]>::try_from(self)?[index].into()),
                _ => Err(TiffFormatError::UnsignedIntegerExpected(self.clone()).into()),
            }
        }
}

// Conversion logic
// ----------------
// structured as follows:
// - f32/f64
// - unsigned
// - signed
//
// with the following:
// - single value - fails if multiple values
// - slice - only for the exact type (u64->u64)
// - vec - also for other types (creates an owned copy of underlying data)

impl TryFrom<&BufferedEntry> for f32 {
    type Error = TiffError;

    fn try_from(val: &BufferedEntry) -> Result<Self, Self::Error> {
        if val.data.len() != val.tag_type.size() {
            return Err(TiffFormatError::InconsistentSizesEncountered(val.clone()).into());
        }
        match val.tag_type {
            TagType::FLOAT => Ok(bytemuck::cast(<[u8; 4]>::try_from(val.data()).unwrap())),
            _ => Err(TiffFormatError::FloatExpected(val.clone()).into()),
        }
    }
}

#[rustfmt::skip]
impl TryFrom<&BufferedEntry> for f64 {
    type Error = TiffError;

    fn try_from(val: &BufferedEntry) -> Result<Self, Self::Error> {
        if val.data.len() != val.tag_type.size() {
            return Err(TiffFormatError::InconsistentSizesEncountered(val.clone()).into());
        }
        match val.tag_type {
            TagType::FLOAT  => Ok(Self::from(bytemuck::cast::<_, f32>(<[u8; 4]>::try_from(val.data()).unwrap()))),
            TagType::DOUBLE => Ok(           bytemuck::cast          (<[u8; 8]>::try_from(val.data()).unwrap()) ),
            _ =>  Err(TiffFormatError::FloatExpected(val.clone()).into())
        }
    }
}

#[rustfmt::skip]
impl TryFrom<&BufferedEntry> for u8 {
    type Error = TiffError;

    fn try_from(val: &BufferedEntry) -> Result<Self, Self::Error> {
        if val.data.len() != val.tag_type.size() {
            dbg!(val.data.len() != val.tag_type.size());
            return Err(TiffFormatError::InconsistentSizesEncountered(val.clone()).into());
        }
        match val.tag_type {
            // because we do `<[u8; n]>::try_from()` in stead of
            // `<&[u8;n]>`, we copy over the data, but IDontCare.
            TagType::BYTE                  => Ok(               bytemuck::cast::<_, u8 >(<[u8; 1]>::try_from(val.data()).unwrap())  ),
            TagType::SHORT                 => Ok(Self::try_from(bytemuck::cast::<_, u16>(<[u8; 2]>::try_from(val.data()).unwrap()))?),
            TagType::LONG  | TagType::IFD  => Ok(Self::try_from(bytemuck::cast::<_, u32>(<[u8; 4]>::try_from(val.data()).unwrap()))?),
            TagType::LONG8 | TagType::IFD8 => Ok(Self::try_from(bytemuck::cast::<_, u64>(<[u8; 8]>::try_from(val.data()).unwrap()))?),
            _ => Err(TiffFormatError::UnsignedIntegerExpected(val.clone()).into()),
        }
    }
}

#[rustfmt::skip]
impl TryFrom<&BufferedEntry> for u16 {
    type Error = TiffError;

    fn try_from(val: &BufferedEntry) -> Result<Self, Self::Error> {
        if val.data.len() != val.tag_type.size() {
            dbg!(val.data.len() != val.tag_type.size());
            return Err(TiffFormatError::InconsistentSizesEncountered(val.clone()).into());
        }
        match val.tag_type {
            // because we do `<[u8; n]>::try_from()` in stead of
            // `<&[u8;n]>`, we copy over the data, but IDontCare.
            TagType::BYTE                  => Ok(Self::    from(bytemuck::cast::<_, u8 >(<[u8; 1]>::try_from(val.data()).unwrap())) ),
            TagType::SHORT                 => Ok(               bytemuck::cast::<_, u16>(<[u8; 2]>::try_from(val.data()).unwrap())  ),
            TagType::LONG  | TagType::IFD  => Ok(Self::try_from(bytemuck::cast::<_, u32>(<[u8; 4]>::try_from(val.data()).unwrap()))?),
            TagType::LONG8 | TagType::IFD8 => Ok(Self::try_from(bytemuck::cast::<_, u64>(<[u8; 8]>::try_from(val.data()).unwrap()))?),
            _ => Err(TiffFormatError::UnsignedIntegerExpected(val.clone()).into()),
        }
    }
}

#[rustfmt::skip]
impl TryFrom<&BufferedEntry> for u32 {
    type Error = TiffError;

    fn try_from(val: &BufferedEntry) -> Result<Self, Self::Error> {
        if val.data.len() != val.tag_type.size() {
            dbg!(val.data.len() != val.tag_type.size());
            return Err(TiffFormatError::InconsistentSizesEncountered(val.clone()).into());
        }
        match val.tag_type {
            // because we do `<[u8; n]>::try_from()` in stead of
            // `<&[u8;n]>`, we copy over the data, but IDontCare.
            TagType::BYTE                  => Ok(Self::    from(bytemuck::cast::<_, u8 >(<[u8; 1]>::try_from(val.data()).unwrap())) ),
            TagType::SHORT                 => Ok(Self::    from(bytemuck::cast::<_, u16>(<[u8; 2]>::try_from(val.data()).unwrap())) ),
            TagType::LONG  | TagType::IFD  => Ok(               bytemuck::cast::<_, u32>(<[u8; 4]>::try_from(val.data()).unwrap())  ),
            TagType::LONG8 | TagType::IFD8 => Ok(Self::try_from(bytemuck::cast::<_, u64>(<[u8; 8]>::try_from(val.data()).unwrap()))?),
            _ => Err(TiffFormatError::UnsignedIntegerExpected(val.clone()).into()),
        }
    }
}

#[rustfmt::skip]
impl TryFrom<&BufferedEntry> for u64 {
    type Error = TiffError;

    fn try_from(val: &BufferedEntry) -> Result<Self, Self::Error> {
        if val.data.len() != val.tag_type.size() {
            dbg!(val.data.len() != val.tag_type.size());
            return Err(TiffFormatError::InconsistentSizesEncountered(val.clone()).into());
        }
        match val.tag_type {
            // because we do `<[u8; n]>::try_from()` in stead of
            // `<&[u8;n]>`, we copy over the data, but IDontCare.
            TagType::BYTE                  => Ok(Self::    from(bytemuck::cast::<_, u8 >(<[u8; 1]>::try_from(val.data()).unwrap())) ),
            TagType::SHORT                 => Ok(Self::    from(bytemuck::cast::<_, u16>(<[u8; 2]>::try_from(val.data()).unwrap())) ),
            TagType::LONG  | TagType::IFD  => Ok(Self::    from(bytemuck::cast::<_, u32>(<[u8; 4]>::try_from(val.data()).unwrap())) ),
            TagType::LONG8 | TagType::IFD8 => Ok(               bytemuck::cast::<_, u64>(<[u8; 8]>::try_from(val.data()).unwrap())  ),
            _ => Err(TiffFormatError::UnsignedIntegerExpected(val.clone()).into()),
        }
    }
}

#[rustfmt::skip]
impl TryFrom<&BufferedEntry> for i8 {
    type Error = TiffError;

    fn try_from(val: &BufferedEntry) -> Result<Self, Self::Error> {
        if val.data.len() != val.tag_type.size() {
            return Err(TiffFormatError::InconsistentSizesEncountered(val.clone()).into());
        }
        match val.tag_type {
            // because we do `<[u8; n]>::try_from()` in stead of
            // `<&[u8;n]>`, we copy over the data, but IDC.
            TagType::SBYTE  => Ok(               bytemuck::cast::<_, i8 >(<[u8; 1]>::try_from(val.data()).unwrap())  ),
            TagType::SSHORT => Ok(Self::try_from(bytemuck::cast::<_, i16>(<[u8; 2]>::try_from(val.data()).unwrap()))?),
            TagType::SLONG  => Ok(Self::try_from(bytemuck::cast::<_, i32>(<[u8; 4]>::try_from(val.data()).unwrap()))?),
            TagType::SLONG8 => Ok(Self::try_from(bytemuck::cast::<_, i64>(<[u8; 8]>::try_from(val.data()).unwrap()))?),
            _ => Err(TiffFormatError::SignedIntegerExpected(val.clone()).into())
        }
    }
}

#[rustfmt::skip]
impl TryFrom<&BufferedEntry> for i16 {
    type Error = TiffError;

    fn try_from(val: &BufferedEntry) -> Result<Self, Self::Error> {
        if val.data.len() != val.tag_type.size() {
            return Err(TiffFormatError::InconsistentSizesEncountered(val.clone()).into());
        }
        match val.tag_type {
            // because we do `<[u8; n]>::try_from()` in stead of
            // `<&[u8;n]>`, we copy over the data, but IDC.
            TagType::SBYTE  => Ok(Self::    from(bytemuck::cast::<_, i8 >(<[u8; 1]>::try_from(val.data()).unwrap())) ),
            TagType::SSHORT => Ok(               bytemuck::cast::<_, i16>(<[u8; 2]>::try_from(val.data()).unwrap())  ),
            TagType::SLONG  => Ok(Self::try_from(bytemuck::cast::<_, i32>(<[u8; 4]>::try_from(val.data()).unwrap()))?),
            TagType::SLONG8 => Ok(Self::try_from(bytemuck::cast::<_, i64>(<[u8; 8]>::try_from(val.data()).unwrap()))?),
            _ => Err(TiffFormatError::SignedIntegerExpected(val.clone()).into())
        }
    }
}

#[rustfmt::skip]
impl TryFrom<&BufferedEntry> for i32 {
    type Error = TiffError;

    fn try_from(val: &BufferedEntry) -> Result<Self, Self::Error> {
        if val.data.len() != val.tag_type.size() {
            return Err(TiffFormatError::InconsistentSizesEncountered(val.clone()).into());
        }
        match val.tag_type {
            // because we do `<[u8; n]>::try_from()` in stead of
            // `<&[u8;n]>`, we copy over the data, but IDC.
            TagType::SBYTE  => Ok(Self::    from(bytemuck::cast::<_, i8 >(<[u8; 1]>::try_from(val.data()).unwrap())) ),
            TagType::SSHORT => Ok(Self::    from(bytemuck::cast::<_, i16>(<[u8; 2]>::try_from(val.data()).unwrap())) ),
            TagType::SLONG  => Ok(               bytemuck::cast::<_, i32>(<[u8; 4]>::try_from(val.data()).unwrap())  ),
            TagType::SLONG8 => Ok(Self::try_from(bytemuck::cast::<_, i64>(<[u8; 8]>::try_from(val.data()).unwrap()))?),
            _ => Err(TiffFormatError::SignedIntegerExpected(val.clone()).into())
        }
    }
}

#[rustfmt::skip]
impl TryFrom<&BufferedEntry> for i64 {
    type Error = TiffError;

    fn try_from(val: &BufferedEntry) -> Result<Self, Self::Error> {
        if val.data.len() != val.tag_type.size() {
            return Err(TiffFormatError::InconsistentSizesEncountered(val.clone()).into());
        }
        match val.tag_type {
            // because we do `<[u8; n]>::try_from()` in stead of
            // `<&[u8;n]>`, we copy over the data, but IDC.
            TagType::SBYTE  => Ok(Self::    from(bytemuck::cast::<_, i8 >(<[u8; 1]>::try_from(val.data()).unwrap())) ),
            TagType::SSHORT => Ok(Self::    from(bytemuck::cast::<_, i16>(<[u8; 2]>::try_from(val.data()).unwrap())) ),
            TagType::SLONG  => Ok(Self::    from(bytemuck::cast::<_, i32>(<[u8; 4]>::try_from(val.data()).unwrap())) ),
            TagType::SLONG8 => Ok(               bytemuck::cast::<_, i64>(<[u8; 8]>::try_from(val.data()).unwrap())  ),
            _ => Err(TiffFormatError::SignedIntegerExpected(val.clone()).into())
        }
    }
}

// ------
// Slices
// ------

// impl<'a> TryFrom<&'a BufferedEntry> for &'a [f32] {
//     type Error = TiffError;

//     fn try_from(val: &'a BufferedEntry) -> Result<Self, Self::Error> {
//         if val.data.len() != val.tag_type.size() * usize::try_from(val.count)? {
//             return Err(TiffFormatError::InconsistentSizesEncountered(val.clone()).into());
//         }
//         match val.tag_type {
//             TagType::FLOAT => Ok(bytemuck::cast_slice(&val.data()[..])),
//             _ => Err(TiffFormatError::FloatExpected(val.clone()).into()),
//         }
//     }
// }

// /// slice casting is more stringent and efficient.
// #[rustfmt::skip]
// impl<'a> TryFrom<&'a BufferedEntry> for &'a[f64] {
//     type Error = TiffError;

//     fn try_from(val: &'a BufferedEntry) -> Result<Self, Self::Error> {
//         if val.data.len() != val.tag_type.size() * usize::try_from(val.count)? {
//             return Err(TiffFormatError::InconsistentSizesEncountered(val.clone()).into());
//         }
//         match val.tag_type {
//             TagType::DOUBLE => Ok(           bytemuck::cast_slice          (&val.data()[..]) ),
//             _ =>  Err(TiffFormatError::FloatExpected(val.clone()).into())
//         }
//     }
// }

macro_rules! entry_tryfrom_slice {
    ($type:ty, $($tag_type:pat),+) => {
        #[rustfmt::skip]
        impl<'a> TryFrom<&'a BufferedEntry> for &'a[$type] {
            type Error = TiffError;

            fn try_from(val: &'a BufferedEntry) -> Result<Self, Self::Error> {
                if val.data.len() != val.tag_type.size() * usize::try_from(val.count)? {
                    dbg!(val.data.len() != val.tag_type.size());
                    return Err(TiffFormatError::InconsistentSizesEncountered(val.clone()).into());
                }
                match val.tag_type {
                    $(
                        $tag_type => Ok(bytemuck::cast_slice(&val.data()[..])),
                    )+
                    _ => Err(TiffFormatError::InconsistentSizesEncountered(val.clone()).into()),
                }
            }
        }
    };
}

entry_tryfrom_slice!(f32, TagType::FLOAT);
entry_tryfrom_slice!(f64, TagType::DOUBLE);
entry_tryfrom_slice!(u8, TagType::BYTE);
entry_tryfrom_slice!(u16, TagType::SHORT);
entry_tryfrom_slice!(u32, TagType::LONG, TagType::IFD);
entry_tryfrom_slice!(u64, TagType::LONG8, TagType::IFD8);
entry_tryfrom_slice!(i8, TagType::SBYTE);
entry_tryfrom_slice!(i16, TagType::SSHORT);
entry_tryfrom_slice!(i32, TagType::SLONG);
entry_tryfrom_slice!(i64, TagType::SLONG8);

// -------
// vectors
// -------

#[rustfmt::skip]
impl TryFrom<&BufferedEntry> for Vec<f64> {
    type Error = TiffError;

    fn try_from(val: &BufferedEntry) -> Result<Self, Self::Error> {
        if val.data.len() != val.tag_type.size() * usize::try_from(val.count)? {
            return Err(TiffFormatError::InconsistentSizesEncountered(val.clone()).into());
        }
        match val.tag_type {
            TagType::DOUBLE => Ok(bytemuck::cast_slice(&val.data()[..]).to_vec()),
            TagType::FLOAT =>  Ok(bytemuck::cast_slice::<_, f32>(&val.data()[..]).iter().map(|v| f64::from(*v)).collect()),
            _ =>  Err(TiffFormatError::FloatExpected(val.clone()).into())
        }
    }
}

#[rustfmt::skip]
impl TryFrom<&BufferedEntry> for Vec<f32> {
    type Error = TiffError;

    fn try_from(val: &BufferedEntry) -> Result<Self, Self::Error> {
        if val.data.len() != val.tag_type.size() * usize::try_from(val.count)? {
            return Err(TiffFormatError::InconsistentSizesEncountered(val.clone()).into());
        }
        match val.tag_type {
            TagType::FLOAT =>   Ok(bytemuck::cast_slice(&val.data()[..]).to_vec()),
            // TagType::DOUBLE =>  Ok(bytemuck::cast_slice::<_, f64>(&val.data()[..]).iter().map(|v| f32::try_from(*v)).collect()),
            _ =>  Err(TiffFormatError::FloatExpected(val.clone()).into())
        }
    }
}

// String
// -------

impl<'a> TryFrom<&'a BufferedEntry> for &'a str {
    type Error = TiffError;

    fn try_from(val: &'a BufferedEntry) -> Result<Self, Self::Error> {
        if val.data().len() != usize::try_from(val.count)? {
            return Err(TiffFormatError::InconsistentSizesEncountered(val.clone()).into());
        }
        match val.tag_type {
            TagType::ASCII | TagType::BYTE | TagType::UNDEFINED => {
                if val.data().is_ascii() && val.data().ends_with(&[0]) {
                    let v = std::str::from_utf8(val.data())?;
                    let v = v.trim_matches(char::from(0));
                    Ok(v)
                } else {
                    Err(TiffFormatError::InvalidTag.into())
                }
            }
            _ => Err(TiffFormatError::AsciiExpected(val.clone()).into()),
        }
    }
}

// macro_rules! entry_tryfrom_unsigned_vec {
//     ($type:ty) => {
//         #[rustfmt::skip]
//         impl TryFrom<&BufferedEntry> for Vec<$type> {
//             type Error = TiffError;

//             fn try_from(val: &BufferedEntry) -> Result<Self, Self::Error> {
//                 if val.data.len() != val.tag_type.size() {
//                     dbg!(val.data.len() != val.tag_type.size());
//                     return Err(TiffFormatError::InconsistentSizesEncountered(val.clone()).into());
//                 }
//                 match val.tag_type {
//                     // because we do `<[u8; n]>::try_from()` in stead of
//                     // `<&[u8;n]>`, we copy over the data, but IDontCare.
//                     TagType::BYTE                  => Ok(Self::try_from(bytemuck::cast_slice::<_, u8 >(val.data())).unwrap()),
//                     TagType::SHORT                 => Ok(Self::try_from(bytemuck::cast_slice::<_, u16>(val.data())).unwrap()),
//                     TagType::LONG  | TagType::IFD  => Ok(Self::try_from(bytemuck::cast_slice::<_, u32>(val.data())).unwrap()),
//                     TagType::LONG8 | TagType::IFD8 => Ok(Self::try_from(bytemuck::cast_slice::<_, u64>(val.data())).unwrap()),
//                     _ => Err(TiffFormatError::UnsignedIntegerExpected(val.clone()).into()),
//                 }
//             }
//         }
//     };
// }

// entry_tryfrom_unsigned_vec!(u8);

/// Should not be needed in future, since we do everything from BufferedEntry
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

#[rustfmt::skip]
impl TryFrom<Value> for BufferedEntry {
    type Error = TiffError;
    fn try_from(val: Value) -> Result<Self, Self::Error> {
        Ok(match val {
            Value::Byte(v)                     => BufferedEntry{ tag_type: TagType::BYTE     , count: 1, data: v.to_ne_bytes().to_vec()},
            Value::SignedByte(v)               => BufferedEntry{ tag_type: TagType::SBYTE    , count: 1, data: v.to_ne_bytes().to_vec()},
            Value::Ascii(v)                => BufferedEntry{ tag_type: TagType::ASCII    , count: u64::try_from(v.len() + 1)?, data: (v + "\0").as_bytes().to_vec() },
            Value::Undefined(v)                => BufferedEntry{ tag_type: TagType::UNDEFINED, count: 1, data: v.to_ne_bytes().to_vec() },
            Value::Short(v)                   => BufferedEntry{ tag_type: TagType::SHORT    , count: 1, data: v.to_ne_bytes().to_vec() },
            Value::SShort(v)                  => BufferedEntry{ tag_type: TagType::SSHORT    , count: 1, data: v.to_ne_bytes().to_vec() },
            Value::Long(v)                    => BufferedEntry{ tag_type: TagType::LONG     , count: 1, data: v.to_ne_bytes().to_vec() },
            Value::Ifd(v)                     => BufferedEntry{ tag_type: TagType::IFD      , count: 1, data: v.to_ne_bytes().to_vec() },
            Value::SLong(v)                   => BufferedEntry{ tag_type: TagType::SLONG    , count: 1, data: v.to_ne_bytes().to_vec() },
            Value::Long8(v)                   => BufferedEntry{ tag_type: TagType::LONG8    , count: 1, data: v.to_ne_bytes().to_vec() },
            Value::Ifd8(v)                    => BufferedEntry{ tag_type: TagType::IFD8     , count: 1, data: v.to_ne_bytes().to_vec() },
            Value::SLong8(v)                  => BufferedEntry{ tag_type: TagType::SLONG8   , count: 1, data: v.to_ne_bytes().to_vec() },
            Value::Float(v)                   => BufferedEntry{ tag_type: TagType::FLOAT    , count: 1, data: v.to_ne_bytes().to_vec() },
            Value::Double(v)                  => BufferedEntry{ tag_type: TagType::DOUBLE   , count: 1, data: v.to_ne_bytes().to_vec() },
            Value::Rational(num, denom)  => BufferedEntry{ tag_type: TagType::RATIONAL , count: 1, data: bytemuck::cast_slice(&[num, denom]).to_vec() },
            Value::SRational(num, denom) => BufferedEntry{ tag_type: TagType::SRATIONAL, count: 1, data: bytemuck::cast_slice(&[num, denom]).to_vec() },
            Value::List(vec) => {
                let mut buf = Self::try_from(vec[0].clone())?;
                for v in &vec[1..] {
                    let mut temp = Self::try_from(v.clone())?;
                    if temp.tag_type != buf.tag_type {
                        return Err(TiffFormatError::InvalidTag.into());
                    }
                    buf.data.append(&mut temp.data);
                    buf.count += temp.count;
                }
                buf
            },
        })
    }
}

#[allow(unused_imports)]
mod test_entry {
    use super::*;
    use crate::ByteOrder;
    use std::io;
    use TagType::{
        ASCII,
        // SINGLE BYTE
        BYTE,
        DOUBLE,
        FLOAT,
        IFD,
        IFD8,
        // 4-BYTE
        LONG,
        // 8-BYTE
        LONG8,
        RATIONAL,
        SBYTE,
        // 2-BYTE
        SHORT,
        SLONG,
        SLONG8,
        SRATIONAL,
        SSHORT,
        UNDEFINED,
    };

    #[test]
    fn test_bufferedentry_into_u8slice() {
        let data = vec![42u8;43];
        let entry = BufferedEntry{
            tag_type: BYTE,
            count: 43,
            data: data.clone(),
        };
        assert_eq!(<&[u8]>::try_from(&entry).unwrap(), data);
    }

    /// test conversion for single value, slice and too big numbers
    /// actually not nice that 
    macro_rules! test_bufferedentry_into {
        ($t:ty,  $name:ident, $(($tag_type:expr, $st:ty)),+) => {
            #[test]
            fn $name() {
                let val = 42 as $t;
                $(
                    let source_val = val as $st;
                    let e = BufferedEntry{
                        tag_type: $tag_type,
                        count: 1,
                        data: source_val.to_ne_bytes().to_vec()
                    };
                    println!("testing for single type {}, {:?}", std::any::type_name::<$t>(), $tag_type);
                    dbg!(&e);
                    // First check: converting data manually
                    assert_eq!(source_val, <$st>::from_ne_bytes(e.data.as_slice().try_into().unwrap()));
                    // sanity: sizes match
                    assert_eq!(e.data.len(), e.tag_type.size());
                    // test is ok: test assertion
                    assert_eq!(val, <$t>::try_from(&e).unwrap());

                    // test for overflow handling
                    if std::mem::size_of::<$t>() < std::mem::size_of::<$st>() {
                        let sv = <$st>::MAX;
                        println!("{sv} should not fit in {}", std::any::type_name::<$t>());
                        let entry = BufferedEntry{
                            tag_type: $tag_type,
                            count: 1,
                            data: sv.to_ne_bytes().to_vec()
                        };
                        // https://stackoverflow.com/a/68919527/14681457
                        match <$t>::try_from(&entry) {
                            Ok(v) => panic!("{v}"),
                            Err(e) => {
                                println!("{e:?}");
                                assert!(matches!(e, TiffError::IntSizeError));
                            },
                        }
                    }
                    
                )+
                
            }
        };
    }

    macro_rules! test_bufferedentry_into_wrongsize {
        ($t:ty, $name:ident, $($tag_type:expr),+) => {
            #[test]
            fn $name() {
              let size = std::mem::size_of::<$t>();
              $(
                let e = BufferedEntry{tag_type: $tag_type, count: 1, data: vec![0; size + 1]};
                println!("testing for type {}, {:?}", std::any::type_name::<$t>(), $tag_type);
                let TiffError::FormatError(err) = <$t>::try_from(&e).unwrap_err() else {
                    panic!("wrong error type, should be InconsistentSizesEncountered")
                };
                assert_eq!(
                    err,
                    TiffFormatError::InconsistentSizesEncountered(e.clone()),
                );

                let e = BufferedEntry{tag_type: $tag_type, count: 2, data: vec![0; size * 2]};
                println!("testing for type {}, {:?}", std::any::type_name::<$t>(), $tag_type);
                let TiffError::FormatError(err) = <$t>::try_from(&e).unwrap_err() else {
                    panic!("wrong error type, should be InconsistentSizesEncountered")
                };
                assert_eq!(
                    err,
                    TiffFormatError::InconsistentSizesEncountered(e.clone()),
                );
              )+
            }
        };
    }

    macro_rules! test_bufferedentry_into_no_int {
        ($t:ty, $name:ident, $($tag_type:expr),+) => {
            #[test]
            fn $name() {
                $(
                    let e = BufferedEntry{tag_type: $tag_type , count: 1, data: vec![0; $tag_type.size()]};
                    println!("testing for type {}, {:?}", std::any::type_name::<$t>(), $tag_type);
                    dbg!(&e);
                    // First check: converting data manually
                    // assert_eq!(val, <$t>::from_ne_bytes(e.data.as_slice().try_into().unwrap()));
                    // sanity: sizes match
                    assert_eq!(e.data.len(), e.tag_type.size());
                    // test is ok: test assertion
                    let TiffError::FormatError(err) = <$t>::try_from(&e).unwrap_err() else {
                        panic!("wrong error type, should be InconsistentSizesEncountered")
                    };
                    assert_eq!(
                        err,
                        TiffFormatError::SignedIntegerExpected(e.clone()),
                    );
                )+
            }
        };
    }

    macro_rules! test_bufferedentry_into_no_uint {
        ($t:ty, $name:ident, $($tag_type:expr),+) => {
            #[test]
            fn $name() {
                $(
                    let e = BufferedEntry{tag_type: $tag_type , count: 1, data: vec![0; $tag_type.size()]};
                    println!("testing for type {}, {:?}", std::any::type_name::<$t>(), $tag_type);
                    dbg!(&e);
                    // First check: converting data manually
                    // assert_eq!(val, <$t>::from_ne_bytes(e.data.as_slice().try_into().unwrap()));
                    // sanity: sizes match
                    assert_eq!(e.data.len(), e.tag_type.size());
                    // test is ok: test assertion
                    let TiffError::FormatError(err) = <$t>::try_from(&e).unwrap_err() else {
                        panic!("wrong error type, should be InconsistentSizesEncountered")
                    };
                    assert_eq!(
                        err,
                        TiffFormatError::UnsignedIntegerExpected(e.clone()),
                    );
                )+
            }
        };
    }

    macro_rules! test_bufferedentry_into_no_float {
        ($t:ty, $name:ident, $($tag_type:expr),+) => {
            #[test]
            fn $name() {
                $(
                    let e = BufferedEntry{tag_type: $tag_type , count: 1, data: vec![0; $tag_type.size()]};
                    println!("testing for type {}, {:?}", std::any::type_name::<$t>(), $tag_type);
                    dbg!(&e);
                    // First check: converting data manually
                    // assert_eq!(val, <$t>::from_ne_bytes(e.data.as_slice().try_into().unwrap()));
                    // sanity: sizes match
                    assert_eq!(e.data.len(), e.tag_type.size());
                    // test is ok: test assertion
                    let TiffError::FormatError(err) = <$t>::try_from(&e).unwrap_err() else {
                        panic!("wrong error type, should be InconsistentSizesEncountered")
                    };
                    assert_eq!(
                        err,
                        TiffFormatError::FloatExpected(e.clone()),
                    );
                )+
            }
        };
    }

    #[rustfmt::skip]
    mod into{
        use super::*;

        test_bufferedentry_into!(f32, test_f32_into_type,  (FLOAT, f32));//, (DOUBLE, f64));
        test_bufferedentry_into!(f64, test_f64_into_type,  (FLOAT, f32), (DOUBLE, f64));
        test_bufferedentry_into!(u8 ,  test_u8_into_type,  (BYTE , u8), (SHORT , u16), (IFD, u32), (LONG , u32), (IFD8, u64), (LONG8 , u64));
        test_bufferedentry_into!(u16, test_u16_into_type,  (BYTE , u8), (SHORT , u16), (IFD, u32), (LONG , u32), (IFD8, u64), (LONG8 , u64));
        test_bufferedentry_into!(u32, test_u32_into_type,  (BYTE , u8), (SHORT , u16), (IFD, u32), (LONG , u32), (IFD8, u64), (LONG8 , u64));
        test_bufferedentry_into!(u64, test_u64_into_type,  (BYTE , u8), (SHORT , u16), (IFD, u32), (LONG , u32), (IFD8, u64), (LONG8 , u64));
        test_bufferedentry_into!(i8 ,  test_i8_into_type,  (SBYTE, i8), (SSHORT, i16),             (SLONG, i32),              (SLONG8, i64));
        test_bufferedentry_into!(i16, test_i16_into_type,  (SBYTE, i8), (SSHORT, i16),             (SLONG, i32),              (SLONG8, i64));
        test_bufferedentry_into!(i32, test_i32_into_type,  (SBYTE, i8), (SSHORT, i16),             (SLONG, i32),              (SLONG8, i64));
        test_bufferedentry_into!(i64, test_i64_into_type,  (SBYTE, i8), (SSHORT, i16),             (SLONG, i32),              (SLONG8, i64));
        

        test_bufferedentry_into_wrongsize!(u8 , test_into_wrongsize_1, BYTE , SBYTE , UNDEFINED, ASCII);
        test_bufferedentry_into_wrongsize!(u16, test_into_wrongsize_2, SHORT, SSHORT);
        test_bufferedentry_into_wrongsize!(u32, test_into_wrongsize_4, LONG , SLONG , IFD , FLOAT );
        test_bufferedentry_into_wrongsize!(u64, test_into_wrongsize_8, LONG8, SLONG8, IFD8, DOUBLE, RATIONAL, SRATIONAL);
        
        test_bufferedentry_into_no_int! (i8 , test_i8_into_noint   , BYTE,  SHORT, UNDEFINED, ASCII,  LONG, IFD, LONG8, IFD8, RATIONAL, SRATIONAL, FLOAT, DOUBLE);
        test_bufferedentry_into_no_uint!(u8 , test_u8_into_nouint  ,SBYTE, SSHORT, UNDEFINED, ASCII, SLONG,     SLONG8,       RATIONAL, SRATIONAL, FLOAT, DOUBLE);
        test_bufferedentry_into_no_float!(f32, test_f32_into_nofloat, BYTE,  SHORT, UNDEFINED, ASCII,  LONG, IFD, LONG8, IFD8, RATIONAL, SRATIONAL,        DOUBLE,
                                                                SBYTE, SSHORT,                   SLONG,     SLONG8);
        test_bufferedentry_into_no_float!(f64, test_f62_into_nofloat, BYTE,  SHORT, UNDEFINED, ASCII,  LONG, IFD, LONG8, IFD8, RATIONAL, SRATIONAL,
                                                                SBYTE, SSHORT,                   SLONG,     SLONG8);
    }

    macro_rules! test_bufferedentry_into_slice {
        ($t:ty, $tag_type:expr, $name:ident) => {
            #[test]
            fn $name() {
                let v = vec![42 as $t; 2];
                let e = BufferedEntry {
                    tag_type: $tag_type,
                    count: 2,
                    data: bytemuck::cast_slice(&v[..]).to_vec(),
                };
                println!("testing for type {}", std::any::type_name::<$t>());
                dbg!(&e);
                // assert_eq!(v, <$t>::from_ne_bytes(e.data.as_slice().try_into().unwrap()));
                assert_eq!(
                    e.data.len(),
                    e.tag_type.size() * usize::try_from(e.count).unwrap()
                );
                assert_eq!(v, <&[$t]>::try_from(&e).unwrap());
            }
        };
    }

    #[rustfmt::skip]
    mod into_slice {
        use super::*;
        
        test_bufferedentry_into_slice!(i8 , SBYTE , test_i8_slice     );
        test_bufferedentry_into_slice!(i16, SSHORT, test_i16_slice    );
        test_bufferedentry_into_slice!(i32, SLONG , test_i32_slice    );
        test_bufferedentry_into_slice!(i64, SLONG8, test_i64_slice    );
        test_bufferedentry_into_slice!(u8 , BYTE  , test_u8_slice     );
        test_bufferedentry_into_slice!(u16, SHORT , test_u16_slice    );
        test_bufferedentry_into_slice!(u32, IFD   , test_u32_ifd_slice);
        test_bufferedentry_into_slice!(u32, LONG  , test_u32_slice    );
        test_bufferedentry_into_slice!(u64, IFD8  , test_u64_ifd_slice);
        test_bufferedentry_into_slice!(u64, LONG8 , test_u64_slice    );
        test_bufferedentry_into_slice!(f32, FLOAT , test_f32_slice    );
        test_bufferedentry_into_slice!(f64, DOUBLE, test_f64_slice    );
    }

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
            assert_eq!(IfdEntry::from_reader(&mut r, false).unwrap(), IfdEntry::Value(res.try_into().unwrap()));
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
        ([5, 0, 1,0,0,0,0,0,0,0,  42,0, 0, 0,43, 0, 0, 0], ByteOrder::LittleEndian, Value::Rational  (42, 43)            ),
        ([0, 5, 0,0,0,0,0,0,0,1,  0, 0, 0,42, 0, 0, 0,43], ByteOrder::BigEndian,    Value::Rational  (42, 43)            ),
        ([10,0, 1,0,0,0,0,0,0,0, 42, 0, 0, 0,43, 0, 0, 0], ByteOrder::LittleEndian, Value::SRational (42, 43)            ),
        ([0,10, 0,0,0,0,0,0,0,1,  0, 0, 0,42, 0, 0, 0,43], ByteOrder::BigEndian,    Value::SRational (42, 43)            ),
        // we special-case IFD
        ];
        for (buf, byte_order, res) in cases {
            let mut r = EndianReader::wrap(io::Cursor::new(buf), byte_order);
            assert_eq!(IfdEntry::from_reader(&mut r, true).unwrap(), IfdEntry::Value(res.try_into().unwrap()));
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
            assert_eq!(IfdEntry::from_reader(&mut r, false).unwrap(), IfdEntry::Value(res.try_into().unwrap()));
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
            assert_eq!(IfdEntry::from_reader(&mut r, true).unwrap(), IfdEntry::Value(res.try_into().unwrap()));
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
