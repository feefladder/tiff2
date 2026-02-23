use std::collections::HashMap;
use std::io::{Read, Seek, SeekFrom, Write};
use std::ops::Range;

use smallvec::{smallvec, SmallVec};

use crate::error::{
    TiffError,
    TiffFormatError::{self, FloatExpected, SignedIntegerExpected, UnsignedIntegerExpected},
    TiffResult,
};
use crate::loader::EndianReader;
use crate::structs::{Tag, TagType};
use crate::util::fix_endianness;
use crate::{ByteOrder, NATIVE_ENDIAN};

pub type Directory = HashMap<Tag, IfdEntry>;

/// an offset into the field
#[derive(Debug, PartialEq, Clone, Copy)]
pub struct Offset {
    pub tag_type: TagType,
    pub count: u64,
    pub offset: u64,
}

impl Offset {
    pub(crate) fn len(&self) -> u64 {
        self.count * u64::try_from(self.tag_type.size()).unwrap()
    }
    /// get the in-file byte range required to load this Tag
    pub fn range(&self) -> Range<u64> {
        self.offset..self.offset + self.len()
    }
}

/// an IFD entry
///
/// Can be either an offset to the Entry's value, or the value itself
#[derive(Debug, PartialEq, Clone)]
pub enum IfdEntry {
    Offset(Offset),
    Value(TagData),
}

impl IfdEntry {
    /// Extract tag type
    ///
    pub(crate) fn tag_type(&self) -> TagType {
        match &self {
            Self::Offset(o) => o.tag_type,
            Self::Value(v) => v.tag_type(),
        }
    }

    /// Get the number of elements in this entry
    pub(crate) fn count(&self) -> u64 {
        match &self {
            Self::Offset(o) => o.count,
            Self::Value(v) => u64::try_from(v.len()).unwrap(),
        }
    }

    /// Create this entry from an EndianReader
    ///
    /// The reader should have its cursor at the start of tag_type, not at tag
    ///
    /// If the value fits in the offset field, it will be converted
    /// ```
    /// # use tiff2::ByteOrder;
    /// # use tiff2::{tags::TagType, TagData::Value, entry::IfdEntry, decoder::EndianReader};
    /// let entry_buf = [
    ///     0x03, 0x00,                         // Type (SHORT)
    ///     0x01, 0x00, 0x00, 0x00,             // Count (1)
    ///     0x2C, 0x01, 0x00, 0x00,             // Offset = Value (300)
    /// ];
    /// let mut r = EndianReader::wrap(std::io::Cursor::new(entry_buf), ByteOrder::LittleEndian);
    /// assert_eq!(IfdEntry::from_reader(&mut r, false).unwrap(), IfdEntry::Value(TagData::Short(300)));
    /// ```
    /// Otherwise an offset is saved
    /// ```
    /// # use tiff2::ByteOrder;
    /// # use tiff2::{tags::TagType, TagData::Value, entry::IfdEntry, decoder::EndianReader};
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
    pub(crate) fn from_reader<R: Read + Seek>(
        r: &mut EndianReader<R>,
        bigtiff: bool,
    ) -> TiffResult<Self> {
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
        if (!bigtiff && value_bytes > 4) || value_bytes > 8 {
            // we are too big, just insert the offset for now
            Ok(IfdEntry::Offset(Offset {
                tag_type,
                count,
                offset: if bigtiff {
                    r.read_u64()?
                } else {
                    r.read_u32()?.into()
                },
            }))
        } else {
            // create a buffer for the offset
            let mut offset = TagData::new(tag_type, count.try_into()?);
            r.read_exact(offset.as_mut())?;

            // discard remaining bytes of offset
            let rem = if bigtiff {
                8 - offset.len() * tag_type.size()
            } else {
                4 - offset.len() * tag_type.size()
            };
            if rem != 0 {
                r.seek(SeekFrom::Current(rem.try_into()?))?;
            }
            // std::io::copy(&mut r.as_ref().take(rem), &mut std::io::sink());

            // fix endianness and return
            fix_endianness(
                offset.as_mut(),
                r.byte_order,
                NATIVE_ENDIAN,
                8 * tag_type.primitive_size(),
            );
            Ok(IfdEntry::Value(offset))
        }
    }

    /// Write this entry to the writer
    ///
    /// ## Errors
    ///
    /// if the value doesn't fit in the offset field
    pub(crate) fn write_to<W: Write + Seek>(
        &self,
        w: &mut EndianReader<W>,
        bigtiff: bool,
    ) -> TiffResult<()> {
        match &self {
            IfdEntry::Offset(o) => {
                w.write_u16(o.tag_type.to_u16()).unwrap();
                if bigtiff {
                    w.write_u64(o.count).unwrap();
                    w.write_u64(o.offset).unwrap();
                } else {
                    w.write_u32(u32::try_from(o.count)?).unwrap();
                    w.write_u32(u32::try_from(o.offset)?).unwrap();
                }
            }
            IfdEntry::Value(v) => {
                w.write_u16(v.tag_type().to_u16()).unwrap();
                if bigtiff {
                    if v.as_ref().len() > 8 {
                        todo!("proper error handling")
                    }
                    w.write_u64(u64::try_from(v.len())?).unwrap();
                    // this part is broken, because we need to fix endianness
                    // TODO: ditch the reader and make everything work on buffers, so we can fix endianness
                    // or something...
                    // I don't really like the EndianReader
                    w.write(v.as_ref()).unwrap();
                    w.seek(SeekFrom::Current(8 - i64::try_from(v.as_ref().len())?))
                        .unwrap();
                } else {
                    if v.as_ref().len() > 4 {
                        todo!("proper error handling")
                    }
                    w.write_u32(u32::try_from(v.len())?).unwrap();
                    w.write(v.as_ref()).unwrap();
                    w.seek(SeekFrom::Current(4 - i64::try_from(v.as_ref().len())?))
                        .unwrap();
                }
            }
        };
        Ok(())
    }
}

/// Entry with buffered data that is properly aligned.
///
/// Therefore, it is an enum over all possible tag types, to also be able to
/// easily distinguish them (not needing to keep [`TagType`] around)
/// ```
/// # let mut reader = std::io::Cursor::new(vec![42u8;42]);
/// let mut entry = TagData::new(TagType::FLOAT, 42);
/// reader.read_exact((&mut entry).into())
/// ```
#[derive(Debug, PartialEq, Clone)]
#[non_exhaustive]
pub enum TagData {
    Byte(SmallVec<[u8; 8]>),
    SByte(SmallVec<[i8; 8]>),
    Undefined(SmallVec<[u8; 8]>),
    /// Ascii value, still bytes to avoid unsafe when giving it as a buffer
    /// That means we can hold an invalid string without UB when giving out the buffer
    /// Also: we keep the `0` byte at the end of the string
    Ascii(SmallVec<[u8; 8]>),
    Short(SmallVec<[u16; 4]>),
    SShort(SmallVec<[i16; 4]>),
    Long(SmallVec<[u32; 2]>),
    SLong(SmallVec<[i32; 2]>),
    Ifd(SmallVec<[u32; 2]>),
    Long8(SmallVec<[u64; 1]>),
    SLong8(SmallVec<[i64; 1]>),
    Ifd8(SmallVec<[u64; 1]>),
    Float(SmallVec<[f32; 2]>),
    Double(SmallVec<[f64; 1]>),
    Rational(SmallVec<[[u32; 2]; 1]>),
    SRational(SmallVec<[[i32; 2]; 1]>),
}

impl TagData {
    /// Create a new, zero-initialized version of Self.
    ///
    #[inline]
    #[rustfmt::skip]
    pub fn new(tag_type: TagType, count: usize) -> Self {
        // from the comment [on this
        // answer](https://stackoverflow.com/a/48218307/14681457), this is
        // actually efficient, since it calls `RawVec::with_capacity_zeroed()` ..apparently
        match tag_type {
            TagType::BYTE      => Self::Byte     (smallvec![0  ;   count]),
            TagType::SBYTE     => Self::SByte    (smallvec![0  ;   count]),
            TagType::UNDEFINED => Self::Undefined(smallvec![0  ;   count]),
            TagType::ASCII     => Self::Ascii    (smallvec![0  ;   count]),
            TagType::SHORT     => Self::Short    (smallvec![0  ;   count]),
            TagType::SSHORT    => Self::SShort   (smallvec![0  ;   count]),
            TagType::LONG      => Self::Long     (smallvec![0  ;   count]),
            TagType::SLONG     => Self::SLong    (smallvec![0  ;   count]),
            TagType::IFD       => Self::Ifd      (smallvec![0  ;   count]),
            TagType::LONG8     => Self::Long8    (smallvec![0  ;   count]),
            TagType::SLONG8    => Self::SLong8   (smallvec![0  ;   count]),
            TagType::IFD8      => Self::Ifd8     (smallvec![0  ;   count]),
            TagType::FLOAT     => Self::Float    (smallvec![0.0;   count]),
            TagType::DOUBLE    => Self::Double   (smallvec![0.0;   count]),
            TagType::RATIONAL  => Self::Rational (smallvec![[0;2]; count]),
            TagType::SRATIONAL => Self::SRational(smallvec![[0;2]; count]),
        }
    }

    #[inline]
    #[rustfmt::skip]
    pub fn tag_type(&self) -> TagType {
        match self {
            Self::Byte     (_) => TagType::BYTE     ,
            Self::SByte    (_) => TagType::SBYTE    ,
            Self::Undefined(_) => TagType::UNDEFINED,
            Self::Ascii    (_) => TagType::ASCII    ,
            Self::Short    (_) => TagType::SHORT    ,
            Self::SShort   (_) => TagType::SSHORT   ,
            Self::Long     (_) => TagType::LONG     ,
            Self::SLong    (_) => TagType::SLONG    ,
            Self::Ifd      (_) => TagType::IFD      ,
            Self::Long8    (_) => TagType::LONG8    ,
            Self::SLong8   (_) => TagType::SLONG8   ,
            Self::Ifd8     (_) => TagType::IFD8     ,
            Self::Float    (_) => TagType::FLOAT    ,
            Self::Double   (_) => TagType::DOUBLE   ,
            Self::Rational (_) => TagType::RATIONAL ,
            Self::SRational(_) => TagType::SRATIONAL,
        }
    }

    #[inline]
    pub fn from_buffer(
        buf: &[u8],
        tag_type: TagType,
        count: usize,
        byte_order: ByteOrder,
    ) -> TiffResult<Self> {
        let mut e = Self::new(tag_type, count);
        let req_len = e.as_mut().len();
        if req_len > buf.len() {
            return Err(TiffError::LimitsExceeded);
        }
        e.as_mut().copy_from_slice(&buf[..req_len]);
        fix_endianness(
            e.as_mut(),
            byte_order,
            NATIVE_ENDIAN,
            tag_type.primitive_size() * 8,
        );
        Ok(e)
    }

    #[inline]
    pub fn to_buffer(&self, buffer: &mut [u8], byte_order: ByteOrder) -> usize {
        let req_len = self.as_ref().len();
        buffer[..req_len].copy_from_slice(self.as_ref());
        fix_endianness(
            &mut buffer[..req_len],
            NATIVE_ENDIAN,
            byte_order,
            self.tag_type().primitive_size() * 8,
        );
        req_len
    }

    /// The length in values of the underlying datatype
    ///
    /// ```
    /// let ascii = TagData::Ascii(smallvec!::from(b"hello world");//
    /// let fracs = TagData::Rational(smallvec![43,42, 42,43]); // 43/42 42/43
    /// assert_eq!(fracs.len(), 2)
    /// ```
    #[rustfmt::skip]
    pub fn len(&self) -> usize {
        match self {
            Self::Byte     (v) => v.len(),
            Self::SByte    (v) => v.len(),
            Self::Undefined(v) => v.len(),
            Self::Ascii    (v) => v.len(),
            Self::Short    (v) => v.len(),
            Self::SShort   (v) => v.len(),
            Self::Long     (v) => v.len(),
            Self::SLong    (v) => v.len(),
            Self::Ifd      (v) => v.len(),
            Self::Long8    (v) => v.len(),
            Self::SLong8   (v) => v.len(),
            Self::Ifd8     (v) => v.len(),
            Self::Float    (v) => v.len(),
            Self::Double   (v) => v.len(),
            Self::Rational (v) => v.len(),
            Self::SRational(v) => v.len(),
        }
    }
}

impl AsMut<[u8]> for TagData {
    /// Get the underlying data as a `&mut [u8]`
    ///
    #[inline]
    #[rustfmt::skip]
    fn as_mut(&mut self) -> &mut [u8] {
        match self {
            Self::Byte     (v) => bytemuck::cast_slice_mut(&mut v[..]),
            Self::SByte    (v) => bytemuck::cast_slice_mut(&mut v[..]),
            Self::Undefined(v) => bytemuck::cast_slice_mut(&mut v[..]),
            Self::Ascii    (v) => bytemuck::cast_slice_mut(&mut v[..]),
            Self::Short    (v) => bytemuck::cast_slice_mut(&mut v[..]),
            Self::SShort   (v) => bytemuck::cast_slice_mut(&mut v[..]),
            Self::Long     (v) => bytemuck::cast_slice_mut(&mut v[..]),
            Self::SLong    (v) => bytemuck::cast_slice_mut(&mut v[..]),
            Self::Ifd      (v) => bytemuck::cast_slice_mut(&mut v[..]),
            Self::Long8    (v) => bytemuck::cast_slice_mut(&mut v[..]),
            Self::SLong8   (v) => bytemuck::cast_slice_mut(&mut v[..]),
            Self::Ifd8     (v) => bytemuck::cast_slice_mut(&mut v[..]),
            Self::Float    (v) => bytemuck::cast_slice_mut(&mut v[..]),
            Self::Double   (v) => bytemuck::cast_slice_mut(&mut v[..]),
            Self::Rational (v) => bytemuck::cast_slice_mut(&mut v[..]),
            Self::SRational(v) => bytemuck::cast_slice_mut(&mut v[..]),
        }
    }
}

impl AsRef<[u8]> for TagData {
    /// Get the underlying data as a `&mut [u8]`
    ///
    #[inline]
    #[rustfmt::skip]
    fn as_ref(& self) -> &[u8] {
        match self {
            Self::Byte     (v) => bytemuck::cast_slice(& v[..]),
            Self::SByte    (v) => bytemuck::cast_slice(& v[..]),
            Self::Undefined(v) => bytemuck::cast_slice(& v[..]),
            Self::Ascii    (v) => bytemuck::cast_slice(& v[..]),
            Self::Short    (v) => bytemuck::cast_slice(& v[..]),
            Self::SShort   (v) => bytemuck::cast_slice(& v[..]),
            Self::Long     (v) => bytemuck::cast_slice(& v[..]),
            Self::SLong    (v) => bytemuck::cast_slice(& v[..]),
            Self::Ifd      (v) => bytemuck::cast_slice(& v[..]),
            Self::Long8    (v) => bytemuck::cast_slice(& v[..]),
            Self::SLong8   (v) => bytemuck::cast_slice(& v[..]),
            Self::Ifd8     (v) => bytemuck::cast_slice(& v[..]),
            Self::Float    (v) => bytemuck::cast_slice(& v[..]),
            Self::Double   (v) => bytemuck::cast_slice(& v[..]),
            Self::Rational (v) => bytemuck::cast_slice(& v[..]),
            Self::SRational(v) => bytemuck::cast_slice(& v[..]),
        }
    }
}

macro_rules! impl_try_from_processed_entry {
    (
        $target:ty,
        [$($from_small:ident),*],
        $from_exact:ident,
        [$($from_large:ident),*],
        $error_type:ident
    ) => {
        impl TryFrom<&TagData> for $target {
            type Error = TiffError;

            fn try_from(val: &TagData) -> Result<Self, Self::Error> {
                if val.len() != 1 {
                    return Err(TiffFormatError::InconsistentSizesEncountered(val.clone()).into());
                }
                match val {
                    $(TagData::$from_small(v) => Ok(Self::from(v[0])),)*
                    TagData::$from_exact(v) => Ok(v[0]),
                    $(TagData::$from_large(v) => Ok(Self::try_from(v[0])?),)*
                    _ => Err($error_type(val.clone()).into()),
                }
            }
        }

        impl TryFrom<TagData> for $target {
            type Error = TiffError;

            fn try_from(val: TagData) -> Result<Self, Self::Error> {
                if val.len() != 1 {
                    return Err(TiffFormatError::InconsistentSizesEncountered(val.clone()).into());
                }
                match val {
                    $(TagData::$from_small(v) => Ok(Self::from(v[0])),)*
                    TagData::$from_exact(v) => Ok(v[0]),
                    $(TagData::$from_large(v) => Ok(Self::try_from(v[0])?),)*
                    _ => Err($error_type(val.clone()).into()),
                }
            }
        }

        impl TryFrom<TagData> for Vec<$target> {
            type Error = TiffError;

            fn try_from(val: TagData) -> Result<Self, Self::Error> {
                match val {
                    // https://stackoverflow.com/q/48308759/14681457
                    $(TagData::$from_small(v) => Ok(v.into_iter().map(<$target>::from).collect()),)*
                    TagData::$from_exact(v) => Ok(v.into_vec()),
                    $(TagData::$from_large(v) => Ok(v.into_iter().map(<$target>::try_from).collect::<Result<Self, _>>()?),)*
                    _ => Err($error_type(val.clone()).into()),
                }
            }
        }

        impl<'a> TryFrom<&'a TagData> for &'a[$target] {
            type Error = TiffError;
            fn try_from(val: &'a TagData) -> Result<Self, Self::Error> {
                match &val {
                    TagData::$from_exact(v) => Ok(&v),
                    _ => Err($error_type(val.clone()).into()),
                }
            }
        }


        impl<'a> TryFrom<&'a mut TagData> for &'a mut [$target] {
            type Error = TiffError;
            fn try_from(val: &'a mut TagData) -> Result<Self, Self::Error> {
                match val {
                    TagData::$from_exact(ref mut v) => Ok(v),
                    _ => Err($error_type(val.clone()).into()),
                }
            }
        }

        // Slice conversion
        impl From<&[$target]> for TagData {
            fn from(slice: &[$target]) -> Self {
                TagData::$from_exact(SmallVec::from(slice))
            }
        }

        // Vector consumption
        impl From<Vec<$target>> for TagData {
            fn from(vec: Vec<$target>) -> Self {
                TagData::$from_exact(SmallVec::from(vec))
            }
        }

        // Inverse: Convert from integer to TagData
        impl From<$target> for TagData {
            fn from(value: $target) -> Self {
                TagData::$from_exact(smallvec![value])
            }
        }
    };
}

#[allow(unused_imports)]
pub use macro_impls::*;
#[rustfmt::skip]
mod macro_impls {
    use super::*;
    impl_try_from_processed_entry!(u8, [Ascii, Undefined], Byte,[Short, Ifd, Long, Ifd8, Long8]   , UnsignedIntegerExpected);
    impl_try_from_processed_entry!(u16,                   [Byte],Short,[Ifd, Long, Ifd8, Long8]   , UnsignedIntegerExpected );
    impl_try_from_processed_entry!(u32,                   [Byte, Short, Ifd],Long,[Ifd8, Long8]   , UnsignedIntegerExpected );
    impl_try_from_processed_entry!(u64,                   [Byte, Short, Ifd, Long, Ifd8],Long8, [], UnsignedIntegerExpected );

    impl_try_from_processed_entry!(i8, [], SByte,[SShort, SLong, SLong8]   , SignedIntegerExpected );
    impl_try_from_processed_entry!(i16,   [SByte],SShort,[SLong, SLong8]   , SignedIntegerExpected);
    impl_try_from_processed_entry!(i32,   [SByte, SShort],SLong,[SLong8]   , SignedIntegerExpected);
    impl_try_from_processed_entry!(i64,   [SByte, SShort, SLong],SLong8, [], SignedIntegerExpected);

    impl_try_from_processed_entry!(f32, [], Float,          [], FloatExpected);
    impl_try_from_processed_entry!(f64,    [Float], Double, [], FloatExpected);

    // #[cfg(target_pointer_width = "32")]
    // impl_try_from_processed_entry!(usize, [Byte],Short,[Ifd, Long, Ifd8, Long8], UnsignedIntegerExpected);
    // #[cfg(target_pointer_width = "64")]
    // impl_try_from_processed_entry!(usize, [Byte,Short, Long, Ifd, Ifd8], Long8, [], UnsignedIntegerExpected);
}
impl TryFrom<&TagData> for (u32, u32) {
    type Error = TiffError;

    fn try_from(val: &TagData) -> Result<Self, Self::Error> {
        if val.len() != 2 {
            return Err(TiffFormatError::InconsistentSizesEncountered(val.clone()).into());
        }
        match &val {
            TagData::Rational(v) => Ok((v[0][0], v[0][1])),
            _ => Err(TiffFormatError::RationalExpected(val.clone()).into()),
        }
    }
}

impl TryFrom<&TagData> for (i32, i32) {
    type Error = TiffError;

    fn try_from(val: &TagData) -> Result<Self, Self::Error> {
        if val.len() != 2 {
            return Err(TiffFormatError::InconsistentSizesEncountered(val.clone()).into());
        }
        match &val {
            TagData::SRational(v) => Ok((v[0][0], v[0][1])),
            _ => Err(TiffFormatError::SignedRationalExpected(val.clone()).into()),
        }
    }
}

impl<'a> TryFrom<&'a TagData> for &'a str {
    type Error = TiffError;

    fn try_from(val: &'a TagData) -> Result<Self, Self::Error> {
        match val {
            TagData::Byte(v) | TagData::Ascii(v) | TagData::Undefined(v) => {
                if v.is_ascii() && v.ends_with(&[0]) {
                    let v = std::str::from_utf8(v)?;
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

#[cfg(test)]
#[allow(unused_imports, clippy::useless_conversion)]
mod test_entry {
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

    use super::*;
    use crate::ByteOrder;

    #[test]
    fn test_bufferedentry_into_u8slice() {
        let data = vec![42u8; 42];
        let entry = TagData::from(data.clone());
        assert_eq!(<&[u8]>::try_from(&entry).unwrap(), data);
    }

    // /// test conversion for single value, slice and too big numbers
    // /// actually not nice that
    // macro_rules! test_bufferedentry_into {
    //     ($t:ty,  $name:ident, $(($type:ident, $st:ty)),+) => {
    //         #[test]
    //         fn $name() {
    //             let val = 42 as $t;
    //             $(
    //                 let source_val = val as $st;
    //                 let e = TagData::$type(vec![source_val]);
    //                 println!("testing for single type {}, {:?}", std::any::type_name::<$t>(), $type);
    //                 dbg!(&e);
    //                 // test is ok: test assertion
    //                 assert_eq!(val, <$t>::try_from(&e).unwrap());

    //                 // test for overflow handling
    //                 if std::mem::size_of::<$t>() < std::mem::size_of::<$st>() {
    //                     let sv = <$st>::MAX;
    //                     println!("{sv} should not fit in {}", std::any::type_name::<$t>());
    //                     let entry = TagData::$type(vec![sv]);
    //                     // https://stackoverflow.com/a/68919527/14681457
    //                     match <$t>::try_from(&entry) {
    //                         Ok(v) => panic!("{v}"),
    //                         Err(e) => {
    //                             println!("{e:?}");
    //                             assert!(matches!(e, TiffError::IntSizeError(_)));
    //                         },
    //                     }
    //                 }

    //             )+

    //         }
    //     };
    // }

    // // macro_rules! test_bufferedentry_into_wrongsize {
    // //     ($t:ty, $name:ident, $($type:ident),+) => {
    // //         #[test]
    // //         fn $name() {
    // //           let size = std::mem::size_of::<$t>();
    // //           $(
    // //             let e = TagData::$type(vec![0]) BufferedEntry{tag_type: $tag_type, count: 1, data: vec![0; size + 1]};
    // //             println!("testing for type {}, {:?}", std::any::type_name::<$t>(), $tag_type);
    // //             let TiffError::FormatError(err) = <$t>::try_from(&e).unwrap_err() else {
    // //                 panic!("wrong error type, should be InconsistentSizesEncountered")
    // //             };
    // //             assert_eq!(
    // //                 err,
    // //                 TiffFormatError::InconsistentSizesEncountered(e.clone()),
    // //             );

    // //             let e = BufferedEntry{tag_type: $tag_type, count: 2, data: vec![0; size * 2]};
    // //             println!("testing for type {}, {:?}", std::any::type_name::<$t>(), $tag_type);
    // //             let TiffError::FormatError(err) = <$t>::try_from(&e).unwrap_err() else {
    // //                 panic!("wrong error type, should be InconsistentSizesEncountered")
    // //             };
    // //             assert_eq!(
    // //                 err,
    // //                 TiffFormatError::InconsistentSizesEncountered(e.clone()),
    // //             );
    // //           )+
    // //         }
    // //     };
    // // }

    // macro_rules! test_bufferedentry_into_no_int {
    //     ($t:ty, $name:ident, $($type:ident),+) => {
    //         #[test]
    //         fn $name() {
    //             $(
    //                 let z = 0 as $t;
    //                 let e = TagData::$type(vec![z]);//BufferedEntry{tag_type: $tag_type , count: 1, data: vec![0; $tag_type.size()]};
    //                 println!("testing for type {}, {:?}", std::any::type_name::<$t>(), $type);
    //                 dbg!(&e);
    //                 // First check: converting data manually
    //                 // assert_eq!(val, <$t>::from_ne_bytes(e.data.as_slice().try_into().unwrap()));
    //                 // sanity: sizes match
    //                 // assert_eq!(e.data.len(), e.tag_type.size());
    //                 // test is ok: test assertion
    //                 let TiffError::FormatError(err) = <$t>::try_from(&e).unwrap_err() else {
    //                     panic!("wrong error type, should be InconsistentSizesEncountered")
    //                 };
    //                 assert_eq!(
    //                     err,
    //                     TiffFormatError::SignedIntegerExpected(e.clone()),
    //                 );
    //             )+
    //         }
    //     };
    // }

    // macro_rules! test_bufferedentry_into_no_uint {
    //     ($t:ty, $name:ident, $($tag_type:expr),+) => {
    //         #[test]
    //         fn $name() {
    //             $(
    //                 let z = 0 as $t;
    //                 let e = TagData::$type(vec![z])//BufferedEntry{tag_type: $tag_type , count: 1, data: vec![0; $tag_type.size()]};
    //                 println!("testing for type {}, {:?}", std::any::type_name::<$t>(), $tag_type);
    //                 dbg!(&e);
    //                 // First check: converting data manually
    //                 // assert_eq!(val, <$t>::from_ne_bytes(e.data.as_slice().try_into().unwrap()));
    //                 // sanity: sizes match
    //                 assert_eq!(e.data.len(), e.tag_type.size());
    //                 // test is ok: test assertion
    //                 let TiffError::FormatError(err) = <$t>::try_from(&e).unwrap_err() else {
    //                     panic!("wrong error type, should be InconsistentSizesEncountered")
    //                 };
    //                 assert_eq!(
    //                     err,
    //                     TiffFormatError::UnsignedIntegerExpected(e.clone()),
    //                 );
    //             )+
    //         }
    //     };
    // }

    // macro_rules! test_bufferedentry_into_no_float {
    //     ($t:ty, $name:ident, $($tag_type:expr),+) => {
    //         #[test]
    //         fn $name() {
    //             $(
    //                 let e = BufferedEntry{tag_type: $tag_type , count: 1, data: vec![0; $tag_type.size()]};
    //                 println!("testing for type {}, {:?}", std::any::type_name::<$t>(), $tag_type);
    //                 dbg!(&e);
    //                 // First check: converting data manually
    //                 // assert_eq!(val, <$t>::from_ne_bytes(e.data.as_slice().try_into().unwrap()));
    //                 // sanity: sizes match
    //                 assert_eq!(e.data.len(), e.tag_type.size());
    //                 // test is ok: test assertion
    //                 let TiffError::FormatError(err) = <$t>::try_from(&e).unwrap_err() else {
    //                     panic!("wrong error type, should be InconsistentSizesEncountered")
    //                 };
    //                 assert_eq!(
    //                     err,
    //                     TiffFormatError::FloatExpected(e.clone()),
    //                 );
    //             )+
    //         }
    //     };
    // }

    // #[rustfmt::skip]
    // mod into{
    //     use super::*;

    //     test_bufferedentry_into!(f32, test_f32_into_type,  (Float, f32));//, (DOUBLE, f64));
    //     test_bufferedentry_into!(f64, test_f64_into_type,  (Float, f32), (Double, f64));
    //     test_bufferedentry_into!(u8 ,  test_u8_into_type,  ( Byte, u8), ( Short, u16), (Ifd, u32), ( Long, u32), (Ifd8, u64), ( Long8, u64));
    //     test_bufferedentry_into!(u16, test_u16_into_type,  ( Byte, u8), ( Short, u16), (Ifd, u32), ( Long, u32), (Ifd8, u64), ( Long8, u64));
    //     test_bufferedentry_into!(u32, test_u32_into_type,  ( Byte, u8), ( Short, u16), (Ifd, u32), ( Long, u32), (Ifd8, u64), ( Long8, u64));
    //     test_bufferedentry_into!(u64, test_u64_into_type,  ( Byte, u8), ( Short, u16), (Ifd, u32), ( Long, u32), (Ifd8, u64), ( Long8, u64));
    //     test_bufferedentry_into!(i8 ,  test_i8_into_type,  (SByte, i8), (SShort, i16),             (SLong, i32),              (SLong8, i64));
    //     test_bufferedentry_into!(i16, test_i16_into_type,  (SByte, i8), (SShort, i16),             (SLong, i32),              (SLong8, i64));
    //     test_bufferedentry_into!(i32, test_i32_into_type,  (SByte, i8), (SShort, i16),             (SLong, i32),              (SLong8, i64));
    //     test_bufferedentry_into!(i64, test_i64_into_type,  (SByte, i8), (SShort, i16),             (SLong, i32),              (SLong8, i64));

    //     // test_bufferedentry_into_wrongsize!(u8 , test_into_wrongsize_1, Byte , SByte , Undefined, Ascii);
    //     // test_bufferedentry_into_wrongsize!(u16, test_into_wrongsize_2, Short, SShort);
    //     // test_bufferedentry_into_wrongsize!(u32, test_into_wrongsize_4, Long, SLong , Ifd , Float );
    //     // test_bufferedentry_into_wrongsize!(u64, test_into_wrongsize_8, Long8, SLong8, Ifd8, Double, Rational, SRational);

    //     test_bufferedentry_into_no_int! (i8 , test_i8_into_noint    , Byte,  Short, Undefined, Ascii,  Long, Ifd, Long8, Ifd8, Rational, SRational, Float, Double);
    //     test_bufferedentry_into_no_uint!(u8 , test_u8_into_nouint   ,SByte, SShort, Undefined, Ascii, SLong,     SLong8,       Rational, SRational, Float, Double);
    //     test_bufferedentry_into_no_float!(f32, test_f32_into_nofloat, Byte,  Short, Undefined, Ascii,  Long, Ifd, Long8, Ifd8, Rational, SRational,        Double,
    //                                                                  SByte, SShort,                   SLong,     SLong8);
    //     test_bufferedentry_into_no_float!(f64, test_f62_into_nofloat, Byte,  Short, Undefined, Ascii,  Long, Ifd, Long8, Ifd8, Rational, SRational,
    //                                                                  SByte, SShort,                   SLong,     SLong8);
    // }

    // macro_rules! test_bufferedentry_into_slice {
    //     ($t:ty, $tag_type:expr, $name:ident) => {
    //         #[test]
    //         fn $name() {
    //             let v = vec![42 as $t; 2];
    //             let e = BufferedEntry {
    //                 tag_type: $tag_type,
    //                 count: 2,
    //                 data: bytemuck::cast_slice(&v[..]).to_vec(),
    //             };
    //             println!("testing for type {}", std::any::type_name::<$t>());
    //             dbg!(&e);
    //             // assert_eq!(v, <$t>::from_ne_bytes(e.data.as_slice().try_into().unwrap()));
    //             assert_eq!(
    //                 e.data.len(),
    //                 e.tag_type.size() * usize::try_from(e.count).unwrap()
    //             );
    //             assert_eq!(v, <&[$t]>::try_from(&e).unwrap());
    //         }
    //     };
    // }

    // #[rustfmt::skip]
    // mod into_slice {
    //     use super::*;

    //     test_bufferedentry_into_slice!(i8 , SBYTE , test_i8_slice     );
    //     test_bufferedentry_into_slice!(i16, SSHORT, test_i16_slice    );
    //     test_bufferedentry_into_slice!(i32, SLONG , test_i32_slice    );
    //     test_bufferedentry_into_slice!(i64, SLONG8, test_i64_slice    );
    //     test_bufferedentry_into_slice!(u8 , BYTE  , test_u8_slice     );
    //     test_bufferedentry_into_slice!(u16, SHORT , test_u16_slice    );
    //     test_bufferedentry_into_slice!(u32, IFD   , test_u32_ifd_slice);
    //     test_bufferedentry_into_slice!(u32, LONG  , test_u32_slice    );
    //     test_bufferedentry_into_slice!(u64, IFD8  , test_u64_ifd_slice);
    //     test_bufferedentry_into_slice!(u64, LONG8 , test_u64_slice    );
    //     test_bufferedentry_into_slice!(f32, FLOAT , test_f32_slice    );
    //     test_bufferedentry_into_slice!(f64, DOUBLE, test_f64_slice    );
    // }

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
        // type   count      offset
        // / \   /     \   /       \
        ([ 1, 0, 1,0,0,0, 42, 0, 0, 0], ByteOrder::LittleEndian, TagData::Byte      (smallvec![42])                ),
        ([ 0, 1, 0,0,0,1, 42, 0, 0, 0], ByteOrder::BigEndian,    TagData::Byte      (smallvec![42])                ),
        ([ 6, 0, 1,0,0,0, 42, 0, 0, 0], ByteOrder::LittleEndian, TagData::SByte     (smallvec![42])                ),
        ([ 0, 6, 0,0,0,1, 42, 0, 0, 0], ByteOrder::BigEndian,    TagData::SByte     (smallvec![42])                ),
        ([ 7, 0, 1,0,0,0, 42, 0, 0, 0], ByteOrder::LittleEndian, TagData::Undefined (smallvec![42])                ),
        ([ 0, 7, 0,0,0,1, 42, 0, 0, 0], ByteOrder::BigEndian,    TagData::Undefined (smallvec![42])                ),
        ([ 2, 0, 1,0,0,0,  0, 0, 0, 0], ByteOrder::LittleEndian, TagData::Ascii     (smallvec![0 ])                ),
        ([ 0, 2, 0,0,0,1,  0, 0, 0, 0], ByteOrder::BigEndian,    TagData::Ascii     (smallvec![0 ])                ),
        ([ 3, 0, 1,0,0,0, 42, 0, 0, 0], ByteOrder::LittleEndian, TagData::Short     (smallvec![42])                ),
        ([ 0, 3, 0,0,0,1,  0,42, 0, 0], ByteOrder::BigEndian,    TagData::Short     (smallvec![42])                ),
        ([ 8, 0, 1,0,0,0, 42, 0, 0, 0], ByteOrder::LittleEndian, TagData::SShort    (smallvec![42])                ),
        ([ 0, 8, 0,0,0,1,  0,42, 0, 0], ByteOrder::BigEndian,    TagData::SShort    (smallvec![42])                ),
        ([ 4, 0, 1,0,0,0, 42, 0, 0, 0], ByteOrder::LittleEndian, TagData::Long      (smallvec![42])                ),
        ([ 0, 4, 0,0,0,1,  0, 0, 0,42], ByteOrder::BigEndian,    TagData::Long      (smallvec![42])                ),
        ([ 9, 0, 1,0,0,0, 42, 0, 0, 0], ByteOrder::LittleEndian, TagData::SLong     (smallvec![42])                ),
        ([ 0, 9, 0,0,0,1,  0, 0, 0,42], ByteOrder::BigEndian,    TagData::SLong     (smallvec![42])                ),
        ([13, 0, 1,0,0,0, 42, 0, 0, 0], ByteOrder::LittleEndian, TagData::Ifd       (smallvec![42])                ),
        ([ 0,13, 0,0,0,1,  0, 0, 0,42], ByteOrder::BigEndian,    TagData::Ifd       (smallvec![42])                ),
        ([11, 0, 1,0,0,0, 42, 0, 0, 0], ByteOrder::LittleEndian, TagData::Float     (smallvec![f32::from_bits(42)])),
        ([ 0,11, 0,0,0,1,  0, 0, 0,42], ByteOrder::BigEndian,    TagData::Float     (smallvec![f32::from_bits(42)])),
        // Double doesn't fit, neither 8-types and we special-case IFD
        ];
        for (buf, byte_order, res) in cases {
            let mut r = EndianReader::wrap(io::Cursor::new(buf), byte_order);
            assert_eq!(IfdEntry::from_reader(&mut r, false).unwrap(), IfdEntry::Value(res.try_into().unwrap()));
            assert_eq!(r.stream_position().unwrap(), buf.len() as u64);
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
        ([ 1, 0, 1,0,0,0,0,0,0,0, 42, 0, 0, 0, 0, 0, 0, 0], ByteOrder::LittleEndian, TagData::Byte      (smallvec![42])                ),
        ([ 0, 1, 0,0,0,0,0,0,0,1, 42, 0, 0, 0, 0, 0, 0, 0], ByteOrder::BigEndian,    TagData::Byte      (smallvec![42])                ),
        ([ 6, 0, 1,0,0,0,0,0,0,0, 42, 0, 0, 0, 0, 0, 0, 0], ByteOrder::LittleEndian, TagData::SByte     (smallvec![42])                ),
        ([ 0, 6, 0,0,0,0,0,0,0,1, 42, 0, 0, 0, 0, 0, 0, 0], ByteOrder::BigEndian,    TagData::SByte     (smallvec![42])                ),
        ([ 7, 0, 1,0,0,0,0,0,0,0, 42, 0, 0, 0, 0, 0, 0, 0], ByteOrder::LittleEndian, TagData::Undefined (smallvec![42])                ),
        ([ 0, 7, 0,0,0,0,0,0,0,1, 42, 0, 0, 0, 0, 0, 0, 0], ByteOrder::BigEndian,    TagData::Undefined (smallvec![42])                ),
        ([ 2, 0, 1,0,0,0,0,0,0,0,  0, 0, 0, 0, 0, 0, 0, 0], ByteOrder::LittleEndian, TagData::Ascii     (smallvec![0 ])                ),
        ([ 0, 2, 0,0,0,0,0,0,0,1,  0, 0, 0, 0, 0, 0, 0, 0], ByteOrder::BigEndian,    TagData::Ascii     (smallvec![0 ])                ),
        ([ 3, 0, 1,0,0,0,0,0,0,0, 42, 0, 0, 0, 0, 0, 0, 0], ByteOrder::LittleEndian, TagData::Short     (smallvec![42])                ),
        ([ 0, 3, 0,0,0,0,0,0,0,1,  0,42, 0, 0, 0, 0, 0, 0], ByteOrder::BigEndian,    TagData::Short     (smallvec![42])                ),
        ([ 8, 0, 1,0,0,0,0,0,0,0, 42, 0, 0, 0, 0, 0, 0, 0], ByteOrder::LittleEndian, TagData::SShort    (smallvec![42])                ),
        ([ 0, 8, 0,0,0,0,0,0,0,1,  0,42, 0, 0, 0, 0, 0, 0], ByteOrder::BigEndian,    TagData::SShort    (smallvec![42])                ),
        ([ 4, 0, 1,0,0,0,0,0,0,0, 42, 0, 0, 0, 0, 0, 0, 0], ByteOrder::LittleEndian, TagData::Long      (smallvec![42])                ),
        ([ 0, 4, 0,0,0,0,0,0,0,1,  0, 0, 0,42, 0, 0, 0, 0], ByteOrder::BigEndian,    TagData::Long      (smallvec![42])                ),
        ([ 9, 0, 1,0,0,0,0,0,0,0, 42, 0, 0, 0, 0, 0, 0, 0], ByteOrder::LittleEndian, TagData::SLong     (smallvec![42])                ),
        ([ 0, 9, 0,0,0,0,0,0,0,1,  0, 0, 0,42, 0, 0, 0, 0], ByteOrder::BigEndian,    TagData::SLong     (smallvec![42])                ),
        ([13, 0, 1,0,0,0,0,0,0,0, 42, 0, 0, 0, 0, 0, 0, 0], ByteOrder::LittleEndian, TagData::Ifd       (smallvec![42])                ),
        ([ 0,13, 0,0,0,0,0,0,0,1,  0, 0, 0,42, 0, 0, 0, 0], ByteOrder::BigEndian,    TagData::Ifd       (smallvec![42])                ),
        ([16, 0, 1,0,0,0,0,0,0,0, 42, 0, 0, 0, 0, 0, 0, 0], ByteOrder::LittleEndian, TagData::Long8     (smallvec![42])                ),
        ([ 0,16, 0,0,0,0,0,0,0,1,  0, 0, 0, 0, 0, 0, 0,42], ByteOrder::BigEndian,    TagData::Long8     (smallvec![42])                ),
        ([17, 0, 1,0,0,0,0,0,0,0, 42, 0, 0, 0, 0, 0, 0, 0], ByteOrder::LittleEndian, TagData::SLong8    (smallvec![42])                ),
        ([ 0,17, 0,0,0,0,0,0,0,1,  0, 0, 0, 0, 0, 0, 0,42], ByteOrder::BigEndian,    TagData::SLong8    (smallvec![42])                ),
        ([18, 0, 1,0,0,0,0,0,0,0, 42, 0, 0, 0, 0, 0, 0, 0], ByteOrder::LittleEndian, TagData::Ifd8      (smallvec![42])                ),
        ([ 0,18, 0,0,0,0,0,0,0,1,  0, 0, 0, 0, 0, 0, 0,42], ByteOrder::BigEndian,    TagData::Ifd8      (smallvec![42])                ),
        ([11, 0, 1,0,0,0,0,0,0,0, 42, 0, 0, 0, 0, 0, 0, 0], ByteOrder::LittleEndian, TagData::Float     (smallvec![f32::from_bits(42)])),
        ([ 0,11, 0,0,0,0,0,0,0,1,  0, 0, 0,42, 0, 0, 0, 0], ByteOrder::BigEndian,    TagData::Float     (smallvec![f32::from_bits(42)])),
        ([12, 0, 1,0,0,0,0,0,0,0, 42, 0, 0, 0, 0, 0, 0, 0], ByteOrder::LittleEndian, TagData::Double    (smallvec![f64::from_bits(42)])),
        ([ 0,12, 0,0,0,0,0,0,0,1,  0, 0, 0, 0, 0, 0, 0,42], ByteOrder::BigEndian,    TagData::Double    (smallvec![f64::from_bits(42)])),
        ([ 5, 0, 1,0,0,0,0,0,0,0,  42,0, 0, 0,43, 0, 0, 0], ByteOrder::LittleEndian, TagData::Rational  (smallvec![[42, 43]])          ),
        ([ 0, 5, 0,0,0,0,0,0,0,1,  0, 0, 0,42, 0, 0, 0,43], ByteOrder::BigEndian,    TagData::Rational  (smallvec![[42, 43]])          ),
        ([ 10,0, 1,0,0,0,0,0,0,0, 42, 0, 0, 0,43, 0, 0, 0], ByteOrder::LittleEndian, TagData::SRational (smallvec![[42, 43]])          ),
        ([ 0,10, 0,0,0,0,0,0,0,1,  0, 0, 0,42, 0, 0, 0,43], ByteOrder::BigEndian,    TagData::SRational (smallvec![[42, 43]])          ),
        // we special-case IFD
        ];
        for (buf, byte_order, res) in cases {
            println!("testing {res:?}");
            let mut r = EndianReader::wrap(io::Cursor::new(buf), byte_order);
            assert_eq!(IfdEntry::from_reader(&mut r, true).unwrap(), IfdEntry::Value(res.try_into().unwrap()));
            assert_eq!(r.stream_position().unwrap(), buf.len() as u64);
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
        ([1, 0, 4,0,0,0, 42,42,42,42], ByteOrder::LittleEndian, TagData::Byte      (smallvec![42; 4]) ),
        ([0, 1, 0,0,0,4, 42,42,42,42], ByteOrder::BigEndian,    TagData::Byte      (smallvec![42; 4]) ),
        ([6, 0, 4,0,0,0, 42,42,42,42], ByteOrder::LittleEndian, TagData::SByte     (smallvec![42; 4]) ),
        ([0, 6, 0,0,0,4, 42,42,42,42], ByteOrder::BigEndian,    TagData::SByte     (smallvec![42; 4]) ),
        ([7, 0, 4,0,0,0, 42,42,42,42], ByteOrder::LittleEndian, TagData::Undefined (smallvec![42; 4]) ),
        ([0, 7, 0,0,0,4, 42,42,42,42], ByteOrder::BigEndian,    TagData::Undefined (smallvec![42; 4]) ),
        ([2, 0, 4,0,0,0, 42,42,42, 0], ByteOrder::LittleEndian, TagData::Ascii     ("***\0".as_bytes().into())),
        ([0, 2, 0,0,0,4, 42,42,42, 0], ByteOrder::BigEndian,    TagData::Ascii     ("***\0".as_bytes().into())),
        ([3, 0, 2,0,0,0, 42, 0,42, 0], ByteOrder::LittleEndian, TagData::Short     (smallvec![42; 2]) ),
        ([0, 3, 0,0,0,2,  0,42, 0,42], ByteOrder::BigEndian,    TagData::Short     (smallvec![42; 2]) ),
        ([8, 0, 2,0,0,0, 42, 0,42, 0], ByteOrder::LittleEndian, TagData::SShort    (smallvec![42; 2]) ),
        ([0, 8, 0,0,0,2,  0,42, 0,42], ByteOrder::BigEndian,    TagData::SShort    (smallvec![42; 2]) ),
        ([0, 2, 0,0,0,4, b'A',b'B',b'C',0], ByteOrder::BigEndian, TagData::Ascii("ABC\0".as_bytes().into())),
        // others don't fit, neither 8-types and we special-case IFD
        ];
        for (buf, byte_order, res) in cases {
            let mut r = EndianReader::wrap(io::Cursor::new(buf), byte_order);
            assert_eq!(IfdEntry::from_reader(&mut r, false).unwrap(), IfdEntry::Value(res.try_into().unwrap()));
            assert_eq!(r.stream_position().unwrap(), buf.len() as u64);
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
        ([ 1, 0, 8,0,0,0,0,0,0,0, 42,42,42,42,42,42,42,42], ByteOrder::LittleEndian, TagData::Byte      (smallvec![42                ; 8])),
        ([ 0, 1, 0,0,0,0,0,0,0,8, 42,42,42,42,42,42,42,42], ByteOrder::BigEndian,    TagData::Byte      (smallvec![42                ; 8])),
        ([ 6, 0, 8,0,0,0,0,0,0,0, 42,42,42,42,42,42,42,42], ByteOrder::LittleEndian, TagData::SByte     (smallvec![42                ; 8])),
        ([ 0, 6, 0,0,0,0,0,0,0,8, 42,42,42,42,42,42,42,42], ByteOrder::BigEndian,    TagData::SByte     (smallvec![42                ; 8])),
        ([ 7, 0, 8,0,0,0,0,0,0,0, 42,42,42,42,42,42,42,42], ByteOrder::LittleEndian, TagData::Undefined (smallvec![42                ; 8])),
        ([ 0, 7, 0,0,0,0,0,0,0,8, 42,42,42,42,42,42,42,42], ByteOrder::BigEndian,    TagData::Undefined (smallvec![42                ; 8])),
        ([ 2, 0, 8,0,0,0,0,0,0,0, 42,42,42,42,42,42,42, 0], ByteOrder::LittleEndian, TagData::Ascii     ("*******\0".as_bytes().into()   )),
        ([ 0, 2, 0,0,0,0,0,0,0,8, 42,42,42,42,42,42,42, 0], ByteOrder::BigEndian,    TagData::Ascii     ("*******\0".as_bytes().into()   )),
        ([ 3, 0, 4,0,0,0,0,0,0,0, 42, 0,42, 0,42, 0,42, 0], ByteOrder::LittleEndian, TagData::Short     (smallvec![42                ; 4])),
        ([ 0, 3, 0,0,0,0,0,0,0,4,  0,42, 0,42, 0,42, 0,42], ByteOrder::BigEndian,    TagData::Short     (smallvec![42                ; 4])),
        ([ 8, 0, 4,0,0,0,0,0,0,0, 42, 0,42, 0,42, 0,42, 0], ByteOrder::LittleEndian, TagData::SShort    (smallvec![42                ; 4])),
        ([ 0, 8, 0,0,0,0,0,0,0,4,  0,42, 0,42, 0,42, 0,42], ByteOrder::BigEndian,    TagData::SShort    (smallvec![42                ; 4])),
        ([ 4, 0, 2,0,0,0,0,0,0,0, 42, 0, 0, 0,42, 0, 0, 0], ByteOrder::LittleEndian, TagData::Long      (smallvec![42                ; 2])),
        ([ 0, 4, 0,0,0,0,0,0,0,2,  0, 0, 0,42, 0, 0, 0,42], ByteOrder::BigEndian,    TagData::Long      (smallvec![42                ; 2])),
        ([ 9, 0, 2,0,0,0,0,0,0,0, 42, 0, 0, 0,42, 0, 0, 0], ByteOrder::LittleEndian, TagData::SLong     (smallvec![42                ; 2])),
        ([ 0, 9, 0,0,0,0,0,0,0,2,  0, 0, 0,42, 0, 0, 0,42], ByteOrder::BigEndian,    TagData::SLong     (smallvec![42                ; 2])),
        ([13, 0, 2,0,0,0,0,0,0,0, 42, 0, 0, 0,42, 0, 0, 0], ByteOrder::LittleEndian, TagData::Ifd       (smallvec![42                ; 2])),
        ([ 0,13, 0,0,0,0,0,0,0,2,  0, 0, 0,42, 0, 0, 0,42], ByteOrder::BigEndian,    TagData::Ifd       (smallvec![42                ; 2])),
        ([11, 0, 2,0,0,0,0,0,0,0, 42, 0, 0, 0,42, 0, 0, 0], ByteOrder::LittleEndian, TagData::Float     (smallvec![f32::from_bits(42); 2])),
        ([ 0,11, 0,0,0,0,0,0,0,2,  0, 0, 0,42, 0, 0, 0,42], ByteOrder::BigEndian,    TagData::Float     (smallvec![f32::from_bits(42); 2])),
        // we special-case IFD
        ];
        for (buf, byte_order, res) in cases {
            let mut r = EndianReader::wrap(io::Cursor::new(buf), byte_order);
            assert_eq!(IfdEntry::from_reader(&mut r, true).unwrap(), IfdEntry::Value(res.try_into().unwrap()));
            assert_eq!(r.stream_position().unwrap(), buf.len() as u64);
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
        ([ 1, 0, 5,0,0,0, 42, 0, 0, 0], ByteOrder::LittleEndian, 5, TagType::BYTE      ),
        ([ 0, 1, 0,0,0,5,  0, 0, 0,42], ByteOrder::BigEndian   , 5, TagType::BYTE      ),
        ([ 6, 0, 5,0,0,0, 42, 0, 0, 0], ByteOrder::LittleEndian, 5, TagType::SBYTE     ),
        ([ 0, 6, 0,0,0,5,  0, 0, 0,42], ByteOrder::BigEndian   , 5, TagType::SBYTE     ),
        ([ 7, 0, 5,0,0,0, 42, 0, 0, 0], ByteOrder::LittleEndian, 5, TagType::UNDEFINED ),
        ([ 0, 7, 0,0,0,5,  0, 0, 0,42], ByteOrder::BigEndian   , 5, TagType::UNDEFINED ),
        ([ 2, 0, 5,0,0,0, 42, 0, 0, 0], ByteOrder::LittleEndian, 5, TagType::ASCII     ),
        ([ 0, 2, 0,0,0,5,  0, 0, 0,42], ByteOrder::BigEndian   , 5, TagType::ASCII     ),
        ([ 3, 0, 3,0,0,0, 42, 0, 0, 0], ByteOrder::LittleEndian, 3, TagType::SHORT     ),
        ([ 0, 3, 0,0,0,3,  0, 0, 0,42], ByteOrder::BigEndian   , 3, TagType::SHORT     ),
        ([ 8, 0, 3,0,0,0, 42, 0, 0, 0], ByteOrder::LittleEndian, 3, TagType::SSHORT    ),
        ([ 0, 8, 0,0,0,3,  0, 0, 0,42], ByteOrder::BigEndian   , 3, TagType::SSHORT    ),
        ([ 4, 0, 2,0,0,0, 42, 0, 0, 0], ByteOrder::LittleEndian, 2, TagType::LONG      ),
        ([ 0, 4, 0,0,0,2,  0, 0, 0,42], ByteOrder::BigEndian   , 2, TagType::LONG      ),
        ([13, 0, 2,0,0,0, 42, 0, 0, 0], ByteOrder::LittleEndian, 2, TagType::IFD       ),
        ([ 0,13, 0,0,0,2,  0, 0, 0,42], ByteOrder::BigEndian   , 2, TagType::IFD       ),
        ([ 9, 0, 2,0,0,0, 42, 0, 0, 0], ByteOrder::LittleEndian, 2, TagType::SLONG     ),
        ([ 0, 9, 0,0,0,2,  0, 0, 0,42], ByteOrder::BigEndian   , 2, TagType::SLONG     ),
        ([ 11,0, 2,0,0,0, 42, 0, 0, 0], ByteOrder::LittleEndian, 2, TagType::FLOAT     ),
        ([ 0,11, 0,0,0,2,  0, 0, 0,42], ByteOrder::BigEndian   , 2, TagType::FLOAT     ),
        ([ 12,0, 1,0,0,0, 42, 0, 0, 0], ByteOrder::LittleEndian, 1, TagType::DOUBLE    ),
        ([ 0,12, 0,0,0,1,  0, 0, 0,42], ByteOrder::BigEndian   , 1, TagType::DOUBLE    ),
        ([ 5, 0, 1,0,0,0, 42, 0, 0, 0], ByteOrder::LittleEndian, 1, TagType::RATIONAL  ),
        ([ 0, 5, 0,0,0,1,  0, 0, 0,42], ByteOrder::BigEndian   , 1, TagType::RATIONAL  ),
        ([ 10,0, 1,0,0,0, 42, 0, 0, 0], ByteOrder::LittleEndian, 1, TagType::SRATIONAL ),
        ([ 0,10, 0,0,0,1,  0, 0, 0,42], ByteOrder::BigEndian   , 1, TagType::SRATIONAL ),
        // Double doesn't fit, neither 8-types and we special-case IFD
        ];
        for (buf, byte_order, count, tag_type) in cases {
            let mut r = EndianReader::wrap(io::Cursor::new(buf), byte_order);
            assert_eq!(IfdEntry::from_reader(&mut r, false).unwrap(), IfdEntry::Offset(Offset { tag_type, count, offset: 42 }));
            assert_eq!(r.stream_position().unwrap(), buf.len() as u64);
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
        ([ 1, 0, 9,0,0,0,0,0,0,0, 42, 0, 0, 0, 0, 0, 0, 0], ByteOrder::LittleEndian, 9, TagType::BYTE      ),
        ([ 0, 1, 0,0,0,0,0,0,0,9,  0, 0, 0, 0, 0, 0, 0,42], ByteOrder::BigEndian   , 9, TagType::BYTE      ),
        ([ 6, 0, 9,0,0,0,0,0,0,0, 42, 0, 0, 0, 0, 0, 0, 0], ByteOrder::LittleEndian, 9, TagType::SBYTE     ),
        ([ 0, 6, 0,0,0,0,0,0,0,9,  0, 0, 0, 0, 0, 0, 0,42], ByteOrder::BigEndian   , 9, TagType::SBYTE     ),
        ([ 7, 0, 9,0,0,0,0,0,0,0, 42, 0, 0, 0, 0, 0, 0, 0], ByteOrder::LittleEndian, 9, TagType::UNDEFINED ),
        ([ 0, 7, 0,0,0,0,0,0,0,9,  0, 0, 0, 0, 0, 0, 0,42], ByteOrder::BigEndian   , 9, TagType::UNDEFINED ),
        ([ 2, 0, 9,0,0,0,0,0,0,0, 42, 0, 0, 0, 0, 0, 0, 0], ByteOrder::LittleEndian, 9, TagType::ASCII     ),
        ([ 0, 2, 0,0,0,0,0,0,0,9,  0, 0, 0, 0, 0, 0, 0,42], ByteOrder::BigEndian   , 9, TagType::ASCII     ),
        ([ 3, 0, 5,0,0,0,0,0,0,0, 42, 0, 0, 0, 0, 0, 0, 0], ByteOrder::LittleEndian, 5, TagType::SHORT     ),
        ([ 0, 3, 0,0,0,0,0,0,0,5,  0, 0, 0, 0, 0, 0, 0,42], ByteOrder::BigEndian   , 5, TagType::SHORT     ),
        ([ 8, 0, 5,0,0,0,0,0,0,0, 42, 0, 0, 0, 0, 0, 0, 0], ByteOrder::LittleEndian, 5, TagType::SSHORT    ),
        ([ 0, 8, 0,0,0,0,0,0,0,5,  0, 0, 0, 0, 0, 0, 0,42], ByteOrder::BigEndian   , 5, TagType::SSHORT    ),
        ([ 4, 0, 3,0,0,0,0,0,0,0, 42, 0, 0, 0, 0, 0, 0, 0], ByteOrder::LittleEndian, 3, TagType::LONG      ),
        ([ 0, 4, 0,0,0,0,0,0,0,3,  0, 0, 0, 0, 0, 0, 0,42], ByteOrder::BigEndian   , 3, TagType::LONG      ),
        ([ 9, 0, 3,0,0,0,0,0,0,0, 42, 0, 0, 0, 0, 0, 0, 0], ByteOrder::LittleEndian, 3, TagType::SLONG     ),
        ([ 0, 9, 0,0,0,0,0,0,0,3,  0, 0, 0, 0, 0, 0, 0,42], ByteOrder::BigEndian   , 3, TagType::SLONG     ),
        ([13, 0, 3,0,0,0,0,0,0,0, 42, 0, 0, 0, 0, 0, 0, 0], ByteOrder::LittleEndian, 3, TagType::IFD       ),
        ([ 0,13, 0,0,0,0,0,0,0,3,  0, 0, 0, 0, 0, 0, 0,42], ByteOrder::BigEndian   , 3, TagType::IFD       ),
        ([16, 0, 3,0,0,0,0,0,0,0, 42, 0, 0, 0, 0, 0, 0, 0], ByteOrder::LittleEndian, 3, TagType::LONG8     ),
        ([ 0,16, 0,0,0,0,0,0,0,3,  0, 0, 0, 0, 0, 0, 0,42], ByteOrder::BigEndian   , 3, TagType::LONG8     ),
        ([17, 0, 3,0,0,0,0,0,0,0, 42, 0, 0, 0, 0, 0, 0, 0], ByteOrder::LittleEndian, 3, TagType::SLONG8    ),
        ([ 0,17, 0,0,0,0,0,0,0,3,  0, 0, 0, 0, 0, 0, 0,42], ByteOrder::BigEndian   , 3, TagType::SLONG8    ),
        ([18, 0, 3,0,0,0,0,0,0,0, 42, 0, 0, 0, 0, 0, 0, 0], ByteOrder::LittleEndian, 3, TagType::IFD8      ),
        ([ 0,18, 0,0,0,0,0,0,0,3,  0, 0, 0, 0, 0, 0, 0,42], ByteOrder::BigEndian   , 3, TagType::IFD8      ),
        ([11, 0, 3,0,0,0,0,0,0,0, 42, 0, 0, 0, 0, 0, 0, 0], ByteOrder::LittleEndian, 3, TagType::FLOAT     ),
        ([ 0,11, 0,0,0,0,0,0,0,3,  0, 0, 0, 0, 0, 0, 0,42], ByteOrder::BigEndian   , 3, TagType::FLOAT     ),
        ([12, 0, 2,0,0,0,0,0,0,0, 42, 0, 0, 0, 0, 0, 0, 0], ByteOrder::LittleEndian, 2, TagType::DOUBLE    ),
        ([ 0,12, 0,0,0,0,0,0,0,2,  0, 0, 0, 0, 0, 0, 0,42], ByteOrder::BigEndian   , 2, TagType::DOUBLE    ),
        ([ 5, 0, 2,0,0,0,0,0,0,0, 42, 0, 0, 0, 0, 0, 0, 0], ByteOrder::LittleEndian, 2, TagType::RATIONAL  ),
        ([ 0, 5, 0,0,0,0,0,0,0,2,  0, 0, 0, 0, 0, 0, 0,42], ByteOrder::BigEndian   , 2, TagType::RATIONAL  ),
        ([10, 0, 2,0,0,0,0,0,0,0, 42, 0, 0, 0, 0, 0, 0, 0], ByteOrder::LittleEndian, 2, TagType::SRATIONAL ),
        ([ 0,10, 0,0,0,0,0,0,0,2,  0, 0, 0, 0, 0, 0, 0,42], ByteOrder::BigEndian   , 2, TagType::SRATIONAL ),
        // we special-case IFD
        ];
        for (buf, byte_order, count, tag_type) in cases {
            let mut r = EndianReader::wrap(io::Cursor::new(buf), byte_order);
            assert_eq!(IfdEntry::from_reader(&mut r, true).unwrap(), IfdEntry::Offset(Offset { tag_type, count, offset: 42 }));
            assert_eq!(r.stream_position().unwrap(), buf.len() as u64);
        }
    }
}
