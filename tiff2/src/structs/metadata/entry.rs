use std::any::type_name;
use std::borrow::Cow;
use std::ops::Range;

use exn::{ensure, Exn, ResultExt};
use smallvec::{smallvec, SmallVec};

use crate::structs::error::{CastError, CastErrorKind};
use crate::structs::TagType;
use crate::util::fix_endianness;
use crate::{ByteOrder, NATIVE_ENDIAN};

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
    pub fn tag_type(&self) -> TagType {
        match &self {
            Self::Offset(o) => o.tag_type,
            Self::Value(v) => v.tag_type(),
        }
    }

    /// Get the number of elements in this entry
    pub fn count(&self) -> u64 {
        match &self {
            Self::Offset(o) => o.count,
            Self::Value(v) => u64::try_from(v.len()).unwrap(),
        }
    }

    /// Load the data into this entry
    ///
    /// Note that this will lose the "entry offset" information
    pub(crate) fn load(&mut self, buf: &[u8], byte_order: ByteOrder) -> exn::Result<(), CastError> {
        if let IfdEntry::Offset(o) = self {
            let count = usize::try_from(o.count)
                .or_raise(|| CastError::overflow(o.count.into(), type_name::<usize>()))?;
            let tag_data = TagData::from_buffer(buf, o.tag_type, count, byte_order)?;
            *self = IfdEntry::Value(tag_data);
            Ok(())
        } else {
            Err(CastError {
                kind: CastErrorKind::Other("was already a value".into()),
            }
            .into())
        }
    }

    /// Save this entries' data and convert it to an offset
    pub(crate) fn save(
        &mut self,
        buf: &mut [u8],
        offset: u64,
        byte_order: ByteOrder,
    ) -> exn::Result<(), CastError> {
        if let IfdEntry::Value(v) = self {
            let count = u64::try_from(v.len()).expect("don't support 128-bit arch");
            v.to_buffer(buf, byte_order);
            *self = IfdEntry::Offset(Offset {
                tag_type: v.tag_type(),
                count,
                offset,
            });
            Ok(())
        } else {
            Err(CastError {
                kind: CastErrorKind::Other("was already an offset".into()),
            }
            .into())
        }
    }
}

/// Entry with buffered data that is properly aligned.
///
/// Therefore, it is an enum over all possible tag types, to also be able to
/// easily distinguish them (not needing to keep [`TagType`] around)
/// ```
/// # use std::io::Read;
/// use tiff2::structs::{TagType, TagData};
/// # let mut reader = std::io::Cursor::new(vec![42u8;42]);
/// let mut entry = TagData::new(TagType::FLOAT, 42);
/// reader.read_exact(entry.as_mut());
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

    /// Create this `TagData` from a buffer
    ///
    /// Errors only if the buffer is too small
    /// ```
    /// use tiff2::ByteOrder;
    /// use tiff2::structs::{TagType, TagData};
    /// const tag_type: TagType = TagType::LONG;
    /// const count: usize = 42;
    /// let buf = [42;tag_type.size()*count -1];
    /// assert!(TagData::from_buffer(&buf, tag_type, count, ByteOrder::LittleEndian).is_err());
    /// ```
    #[inline]
    pub fn from_buffer(
        buf: &[u8],
        tag_type: TagType,
        count: usize,
        byte_order: ByteOrder,
    ) -> exn::Result<Self, CastError> {
        let mut e = Self::new(tag_type, count);
        let req_len = e.as_mut().len();
        ensure!(
            req_len <= buf.len(),
            CastError::invalid_buffer(buf.len(), req_len)
        );
        e.as_mut().copy_from_slice(&buf[..req_len]);
        fix_endianness(
            e.as_mut(),
            byte_order,
            NATIVE_ENDIAN,
            u16::from(tag_type.primitive_size()) * 8,
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
            u16::from(self.tag_type().primitive_size()) * 8,
        );
        req_len
    }

    /// The length in values of the underlying datatype
    ///
    /// ```
    /// # use smallvec::{smallvec, SmallVec};
    /// # use tiff2::structs::TagData;
    /// let ascii = TagData::Ascii(SmallVec::from_vec(b"hello world\0".to_vec()));//
    /// assert_eq!(ascii.len(), 12);
    ///
    /// let fracs = TagData::Rational(smallvec![[43,42], [42,43]]); // 43/42 42/43
    /// assert_eq!(fracs.len(), 2);
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

    pub fn is_empty(&self) -> bool {
        self.len() == 0
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
    /// Get the underlying data as a `&[u8]`
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

macro_rules! impl_tagdata_casts {
    (
        $target:ty,
        [$($from_small:ident),*],
        [$($from_exact:ident),+],
        [$($from_large:ident),*]
    ) => {
        impl TryFrom<&TagData> for $target {
            type Error = Exn<CastError>;

            fn try_from(val: &TagData) -> Result<Self, Self::Error> {
                ensure!(val.len() == 1,CastError::multiple_values(val.len()));
                match val {
                    $(TagData::$from_small(v) => Ok(Self::from(v[0])),)*
                    $(TagData::$from_exact(v) => Ok(v[0]),)+
                    $(TagData::$from_large(v) => Self::try_from(v[0]).or_raise(|| CastError::overflow(v[0] as _, type_name::<$target>())),)*
                    _ => Err(CastError::invalid_cast(val.tag_type(), type_name::<$target>()).into()),
                }
            }
        }

        impl TryFrom<TagData> for $target {
            type Error = Exn<CastError>;

            fn try_from(val: TagData) -> Result<Self, Self::Error> {
                <$target>::try_from(&val)
            }
        }

        // // Inverse: Convert from integer to TagData
        // impl From<$target> for TagData {
        //     fn from(value: $target) -> Self {
        //         TagData::$from_exact(smallvec![value])
        //     }
        // }


        impl<'a> TryFrom<&'a TagData> for Cow<'a, [$target]> {
            type Error = Exn<CastError>;
            fn try_from(val: &'a TagData) -> Result<Self, Self::Error> {
                match &val {
                    $(TagData::$from_small(v) => Ok(Cow::from(v.into_iter().map(|val| <$target>::from(*val)).collect::<Vec<_>>())),)*
                    $(TagData::$from_exact(v) => Ok(Cow::from(&v[..])),)+
                    $(TagData::$from_large(v) => Ok(Cow::from(v
                        .into_iter()
                        .map(|val| <$target>::try_from(*val)
                            .or_raise(|| CastError::overflow(*val as _, type_name::<Vec<$target>>())
                        )).collect::<Result<Self, _>>()?)),)*
                    _ => Err(CastError::invalid_cast(val.tag_type(), type_name::<$target>()).into())
                }
            }
        }

        impl TryFrom<TagData> for Vec<$target> {
            type Error = Exn<CastError>;

            fn try_from(val: TagData) -> Result<Self, Self::Error> {
                match val {
                    // https://stackoverflow.com/q/48308759/14681457
                    $(TagData::$from_small(v) => Ok(v.into_iter().map(<$target>::from).collect()),)*
                    $(TagData::$from_exact(v) => Ok(v.into_vec()),)+
                    $(TagData::$from_large(v) => v.into_iter().map(|val| <$target>::try_from(val).or_raise(|| CastError::overflow(val as _, type_name::<Vec<$target>>()))).collect::<Result<Self, _>>(),)*
                    _ => Err(CastError::invalid_cast(val.tag_type(), type_name::<$target>()).into()),
                }
            }
        }

        impl<'a> TryFrom<&'a TagData> for &'a[$target] {
            type Error = CastError;
            fn try_from(val: &'a TagData) -> Result<Self, Self::Error> {
                match &val {
                    $(TagData::$from_exact(v) => Ok(&v),)+
                    _ => Err(CastError::invalid_cast(val.tag_type(), type_name::<&[$target]>())),
                }
            }
        }


        impl<'a> TryFrom<&'a mut TagData> for &'a mut [$target] {
            type Error = CastError;
            fn try_from(val: &'a mut TagData) -> Result<Self, Self::Error> {
                match val {
                    $(TagData::$from_exact(ref mut v) => Ok(v),)+
                    _ => Err(CastError::invalid_cast(val.tag_type(), type_name::<&mut [$target]>())),
                }
            }
        }

        // // Slice conversion
        // impl From<&[$target]> for TagData {
        //     fn from(slice: &[$target]) -> Self {
        //         TagData::$from_exact(SmallVec::from(slice))
        //     }
        // }

        // // Vector consumption
        // impl From<Vec<$target>> for TagData {
        //     fn from(vec: Vec<$target>) -> Self {
        //         $(TagData::$from_exact(SmallVec::from(vec)))+
        //     }
        // }

        // impl From<SmallVec<[$target; 8/std::mem::size_of::<$target>()]>> for TagData {
        //     fn from(smallvec: SmallVec<[$target; 8/std::mem::size_of::<$target>()]>) -> Self {
        //         TagData::$from_exact(smallvec)
        //     }
        // }
    };
}

#[allow(unused_imports)]
pub use macro_impls::*;
#[rustfmt::skip]
mod macro_impls {
    use super::*;
    impl_tagdata_casts!(u8, [],[Ascii, Undefined, Byte],[Short,Ifd, Long, Ifd8, Long8]   );
    impl_tagdata_casts!(u16,                   [Byte],[Short],[Ifd, Long, Ifd8, Long8]   );
    impl_tagdata_casts!(u32,                   [Byte, Short], [Ifd, Long],[Ifd8, Long8]   );
    impl_tagdata_casts!(u64,                   [Byte, Short, Ifd, Long],[Ifd8, Long8], []);

    impl_tagdata_casts!(i8, [],[SByte],[SShort, SLong, SLong8]   );
    impl_tagdata_casts!(i16,   [SByte],[SShort],[SLong, SLong8]   );
    impl_tagdata_casts!(i32,   [SByte, SShort],[SLong],[SLong8]   );
    impl_tagdata_casts!(i64,   [SByte, SShort, SLong],[SLong8], []);

    impl_tagdata_casts!(f32, [],[Float],          []);
    impl_tagdata_casts!(f64,    [Float],[Double], []);

    // #[cfg(target_pointer_width = "32")]
    // impl_try_from_processed_entry!(usize, [Byte],Short,[Ifd, Long, Ifd8, Long8], UnsignedIntegerExpected);
    // #[cfg(target_pointer_width = "64")]
    // impl_try_from_processed_entry!(usize, [Byte,Short, Long, Ifd, Ifd8], Long8, [], UnsignedIntegerExpected);
}
impl TryFrom<&TagData> for (u32, u32) {
    type Error = Exn<CastError>;

    fn try_from(val: &TagData) -> Result<Self, Self::Error> {
        ensure!(val.len() == 1, CastError::multiple_values(val.len()));
        match &val {
            TagData::Rational(v) => Ok((v[0][0], v[0][1])),
            _ => Err(CastError::invalid_cast(val.tag_type(), type_name::<(u32, u32)>()).into()),
        }
    }
}

impl TryFrom<&TagData> for (i32, i32) {
    type Error = Exn<CastError>;

    fn try_from(val: &TagData) -> Result<Self, Self::Error> {
        ensure!(val.len() == 1, CastError::multiple_values(val.len()));
        match &val {
            TagData::SRational(v) => Ok((v[0][0], v[0][1])),
            _ => Err(CastError::invalid_cast(val.tag_type(), type_name::<(i32, i32)>()).into()),
        }
    }
}

impl<'a> TryFrom<&'a TagData> for &'a str {
    type Error = Exn<CastError>;

    fn try_from(val: &'a TagData) -> Result<Self, Self::Error> {
        match val {
            TagData::Byte(v) | TagData::Ascii(v) | TagData::Undefined(v) => {
                if v.is_ascii() && v.ends_with(&[0]) {
                    let v = std::str::from_utf8(v).unwrap();
                    let v = v.trim_matches(char::from(0));
                    Ok(v)
                } else {
                    Err(CastError::invalid_string(v.clone()).into())
                }
            }
            _ => Err(CastError::invalid_cast(val.tag_type(), type_name::<(i32, i32)>()).into()),
        }
    }
}

#[cfg(test)]
#[allow(unused_imports, clippy::useless_conversion)]
mod test_entry {
    use std::any::type_name;
    use std::io;

    use TagType::*;

    use super::*;
    use crate::ByteOrder;

    #[test]
    fn test_bufferedentry_into_u8slice() {
        let data = smallvec![42u8; 42];
        let entry = TagData::Byte(data.clone());
        assert_eq!(<&[u8]>::try_from(&entry).unwrap(), &data[..]);
    }

    /// test conversion for single value, slice and too big numbers
    /// actually not nice that
    macro_rules! test_tagdata_into_single {
        ($t:ty,  $name:ident, $(($type:ident, $st:ty)),+) => {
            #[test]
            fn $name() {
                let val = 42 as $t;
                $(
                    let source_val = val as $st;
                    let e = TagData::$type(smallvec![source_val]);
                    println!("testing for single type {}, {:?}", std::any::type_name::<$t>(), e);
                    dbg!(&e);
                    // test is ok: test assertion
                    assert_eq!(val, <$t>::try_from(&e).unwrap());

                    // test for overflow handling
                    if std::mem::size_of::<$t>() < std::mem::size_of::<$st>() {
                        let sv = <$st>::MAX;
                        println!("{sv} should not fit in {}", std::any::type_name::<$t>());
                        let entry = TagData::$type(smallvec![sv]);
                        // https://stackoverflow.com/a/68919527/14681457
                        println!("{e:?}");
                        assert_eq!(
                            <$t>::try_from(&entry)
                                .unwrap_err()
                                .frame()
                                .error()
                                .downcast_ref::<CastError>()
                                .unwrap(),
                            &CastError::overflow(sv.into(), type_name::<$t>())
                        );
                    }

                )+

            }
        };
    }

    macro_rules! test_tagdata_into_invalid_cast {
        ($t:ty, $name:ident, $($type:ident),+) => {
            #[test]
            fn $name() {
                $(
                    let z = 0 as $t;
                    let e = TagData::$type(smallvec![0;16]);
                    println!("testing for type {}, {:?}", std::any::type_name::<$t>(), $type);
                    // First check: converting data manually
                    assert_eq!(z, <$t>::from_ne_bytes(e.as_ref().try_into().unwrap()));
                    // sanity: sizes match
                    assert_eq!(e.as_ref().len(), e.tag_type().size());
                    // test is ok: test assertion
                    assert_eq!(
                        <$t>::try_from(&e).unwrap_err(),
                        &CastError::invalid_cast(e.tag_type(), type_name::<$t>()),
                    );
                )+
            }
        };
    }

    #[rustfmt::skip]
    mod into{
        use super::*;
        use TagData::*;
        // test_tagdata_into_single!(f32, test_f32_into_type,  (Float, f32));//, (DOUBLE, f64));
        // test_tagdata_into_single!(f64, test_f64_into_type,  (Float, f32), (Double, f64));
        test_tagdata_into_single!(u8 ,  test_u8_into_type,  ( Byte, u8), ( Short, u16), (Ifd, u32), ( Long, u32), (Ifd8, u64), ( Long8, u64));
        test_tagdata_into_single!(u16, test_u16_into_type,  ( Byte, u8), ( Short, u16), (Ifd, u32), ( Long, u32), (Ifd8, u64), ( Long8, u64));
        test_tagdata_into_single!(u32, test_u32_into_type,  ( Byte, u8), ( Short, u16), (Ifd, u32), ( Long, u32), (Ifd8, u64), ( Long8, u64));
        test_tagdata_into_single!(u64, test_u64_into_type,  ( Byte, u8), ( Short, u16), (Ifd, u32), ( Long, u32), (Ifd8, u64), ( Long8, u64));
        test_tagdata_into_single!(i8 ,  test_i8_into_type,  (SByte, i8), (SShort, i16),             (SLong, i32),              (SLong8, i64));
        test_tagdata_into_single!(i16, test_i16_into_type,  (SByte, i8), (SShort, i16),             (SLong, i32),              (SLong8, i64));
        test_tagdata_into_single!(i32, test_i32_into_type,  (SByte, i8), (SShort, i16),             (SLong, i32),              (SLong8, i64));
        test_tagdata_into_single!(i64, test_i64_into_type,  (SByte, i8), (SShort, i16),             (SLong, i32),              (SLong8, i64));

        // test_bufferedentry_into_wrongsize!(u8 , test_into_wrongsize_1, Byte , SByte , Undefined, Ascii);
        // test_bufferedentry_into_wrongsize!(u16, test_into_wrongsize_2, Short, SShort);
        // test_bufferedentry_into_wrongsize!(u32, test_into_wrongsize_4, Long, SLong , Ifd , Float );
        // test_bufferedentry_into_wrongsize!(u64, test_into_wrongsize_8, Long8, SLong8, Ifd8, Double, Rational, SRational);

        // test_tagdata_into_invalid_cast! (i8 , test_i8_into_noint    , Byte,  Short, Undefined, Ascii,  Long, Ifd, Long8, Ifd8, Rational, SRational, Float, Double);
        // test_tagdata_into_invalid_cast! (u8 , test_u8_into_nouint   ,SByte, SShort, Undefined, Ascii, SLong,     SLong8,       Rational, SRational, Float, Double);
        // test_tagdata_into_invalid_cast! (f32, test_f32_into_nofloat, Byte,  Short, Undefined, Ascii,  Long, Ifd, Long8, Ifd8, Rational, SRational,        Double,
        //                                                              SByte, SShort,                   SLong,     SLong8);
        // test_bufferedentry_into_no_float!(f64, test_f62_into_nofloat, Byte,  Short, Undefined, Ascii,  Long, Ifd, Long8, Ifd8, Rational, SRational,
        //                                                              SByte, SShort,                   SLong,     SLong8);
    }

    macro_rules! test_tagdata_into_slice {
        ($t:ty, $tag_type:ident, $name:ident) => {
            #[test]
            fn $name() {
                let v = smallvec![42 as $t; 2];
                let e = TagData::$tag_type(v.clone());
                println!("testing for type {}", std::any::type_name::<$t>());
                dbg!(&e);
                assert_eq!(&v[..], <&[$t]>::try_from(&e).unwrap());
            }
        };
    }

    #[rustfmt::skip]
    mod into_slice {
        use super::*;

        test_tagdata_into_slice!(i8 , SByte , test_i8_slice     );
        test_tagdata_into_slice!(i16, SShort, test_i16_slice    );
        test_tagdata_into_slice!(i32, SLong , test_i32_slice    );
        test_tagdata_into_slice!(i64, SLong8, test_i64_slice    );
        test_tagdata_into_slice!(u8 , Byte  , test_u8_slice     );
        test_tagdata_into_slice!(u16, Short , test_u16_slice    );
        test_tagdata_into_slice!(u32, Ifd   , test_u32_ifd_slice);
        test_tagdata_into_slice!(u32, Long  , test_u32_slice    );
        test_tagdata_into_slice!(u64, Ifd8  , test_u64_ifd_slice);
        test_tagdata_into_slice!(u64, Long8 , test_u64_slice    );
        test_tagdata_into_slice!(f32, Float , test_f32_slice    );
        test_tagdata_into_slice!(f64, Double, test_f64_slice    );
    }
}
