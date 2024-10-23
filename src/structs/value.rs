use std::io::Read;

use crate::error::{TiffError, TiffFormatError, TiffResult};
use crate::{Tag, TagType};

use self::Value::{
    Ascii, Byte, Double, Float, List, Long, Long8, Rational, SLong, SLong8, SRational, SShort,
    Short, SignedByte,
};

/// Tag value
///
/// Stores tag data from an IFD
#[allow(unused_qualifications)]
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum Value {
    Byte(u8),
    SignedByte(i8),
    Undefined(u8),

    Short(u16),
    SShort(i16),

    Long(u32),
    SLong(i32),

    Long8(u64),
    SLong8(i64),

    Float(f32),
    Double(f64),

    Rational(u32, u32),
    SRational(i32, i32),

    Ascii(String),

    List(Vec<Value>),
    // RationalBig(u64, u64),

    // SRationalBig(i64, i64),

    Ifd(u32),
    Ifd8(u64),
}

impl std::fmt::Display for Value {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> Result<(), std::fmt::Error> {
        match self {
            Value::Byte(e) => write!(f, "{e}"),
            Value::SignedByte(e) => write!(f, "{e}"),
            Value::Undefined(e) => write!(f, "{e}"),

            Value::Short(e) => write!(f, "{e}"),
            Value::SShort(e) => write!(f, "{e}"),

            Value::Long(e) => write!(f, "{e}"),
            Value::Ifd(e) => write!(f, "{e}"),
            Value::SLong(e) => write!(f, "{e}"),

            Value::Long8(e) => write!(f, "{e}"),
            Value::Ifd8(e) => write!(f, "{e}"),
            Value::SLong8(e) => write!(f, "{e}"),

            Value::Float(e) => write!(f, "{e}"),
            Value::Double(e) => write!(f, "{e}"),
            Value::Rational(e1, e2) => {
                let a_mul = (*e1 as u128) * 1000;
                let b = *e2 as u128;
                let div = a_mul / b;

                let frac = div % 1000;
                let rest = div / 1000;

                if frac != 0 {
                    write!(f, "{rest}.{frac:#03}")
                } else {
                    write!(f, "{rest}")
                }
            }
            Value::SRational(e1, e2) => write!(f, "{e1}/{e2}"),
            Value::Ascii(e) => write!(f, "{e}"),

            Value::List(_) => todo!(),
        }
    }
}

impl Value {
    pub fn count(&self) -> usize {
        match self {
            Value::List(v) => v.len(),
            _ => 1,
        }
    }
    /// Get the tag type of this value.
    ///
    /// Will return `TagType::UNDEFINED` in the following cases:
    /// - List with inconsistent types
    /// - RationalBig | SRationalBig
    /// - Undefined
    pub fn tag_type(&self) -> TagType {
        match self {
            Value::Byte(_) => TagType::BYTE,
            Value::Short(_) => TagType::SHORT,
            Value::SignedByte(_) => TagType::SBYTE,
            Value::SShort(_) => TagType::SSHORT,
            Value::SLong(_) => TagType::SLONG,
            Value::SLong8(_) => TagType::SLONG8,
            Value::Long(_) => TagType::LONG,
            Value::Ifd(_) => TagType::IFD,
            Value::Long8(_) => TagType::LONG8,
            Value::Ifd8(_) => TagType::IFD8,
            Value::Float(_) => TagType::FLOAT,
            Value::Double(_) => TagType::DOUBLE,
            Value::Rational(_, _) => TagType::RATIONAL,
            Value::SRational(_, _) => TagType::SRATIONAL,
            Value::Ascii(_) => TagType::ASCII,
            Value::Undefined(_) => TagType::UNDEFINED,
            Value::List(v) => {
                if v.len() == 0 {
                    TagType::UNDEFINED
                } else {
                    let first = &v[0];
                    let first_type = first.tag_type();
                    let first_disc = std::mem::discriminant(first);
                    for it in v {
                        if std::mem::discriminant(it) != first_disc {
                            return TagType::UNDEFINED;
                        }
                    }
                    first_type
                }
            }
        }
    }

    // pub fn into_u8(self) -> TiffResult<u8> {
    //     match self {
    //         Byte(val) => Ok(val),
    //         val => Err(TiffError::FormatError(TiffFormatError::ByteExpected(val))),
    //     }
    // }
    // pub fn into_i8(self) -> TiffResult<i8> {
    //     match self {
    //         SignedByte(val) => Ok(val),
    //         val => Err(TiffError::FormatError(TiffFormatError::SignedByteExpected(
    //             val,
    //         ))),
    //     }
    // }

    // pub fn into_u16(self) -> TiffResult<u16> {
    //     match self {
    //         Short(val) => Ok(val),
    //         Long(val) => Ok(u16::try_from(val)?),
    //         Long8(val) => Ok(u16::try_from(val)?),
    //         val => Err(TiffError::FormatError(
    //             TiffFormatError::UnsignedIntegerExpected(val),
    //         )),
    //     }
    // }

    // pub fn into_i16(self) -> TiffResult<i16> {
    //     match self {
    //         SignedByte(val) => Ok(val.into()),
    //         SShort(val) => Ok(val),
    //         SLong(val) => Ok(i16::try_from(val)?),
    //         SLong8(val) => Ok(i16::try_from(val)?),
    //         val => Err(TiffError::FormatError(
    //             TiffFormatError::SignedShortExpected(val),
    //         )),
    //     }
    // }

    // pub fn into_u32(self) -> TiffResult<u32> {
    //     match self {
    //         Short(val) => Ok(val.into()),
    //         Long(val) => Ok(val),
    //         Long8(val) => Ok(u32::try_from(val)?),
    //         // Ifd(val) => Ok(val),
    //         // IfdBig(val) => Ok(u32::try_from(val)?),
    //         val => Err(TiffError::FormatError(
    //             TiffFormatError::UnsignedIntegerExpected(val),
    //         )),
    //     }
    // }

    // pub fn into_i32(self) -> TiffResult<i32> {
    //     match self {
    //         SignedByte(val) => Ok(val.into()),
    //         SShort(val) => Ok(val.into()),
    //         SLong(val) => Ok(val),
    //         SLong8(val) => Ok(i32::try_from(val)?),
    //         val => Err(TiffError::FormatError(
    //             TiffFormatError::SignedIntegerExpected(val),
    //         )),
    //     }
    // }

    // pub fn into_u64(self) -> TiffResult<u64> {
    //     match self {
    //         Short(val) => Ok(val.into()),
    //         Long(val) => Ok(val.into()),
    //         Long8(val) => Ok(val),
    //         // Ifd(val) => Ok(val.into()),
    //         // IfdBig(val) => Ok(val),
    //         val => Err(TiffError::FormatError(
    //             TiffFormatError::UnsignedIntegerExpected(val),
    //         )),
    //     }
    // }

    // pub fn into_i64(self) -> TiffResult<i64> {
    //     match self {
    //         SignedByte(val) => Ok(val.into()),
    //         SShort(val) => Ok(val.into()),
    //         SLong(val) => Ok(val.into()),
    //         SLong8(val) => Ok(val),
    //         val => Err(TiffError::FormatError(
    //             TiffFormatError::SignedIntegerExpected(val),
    //         )),
    //     }
    // }

    // pub fn into_f32(self) -> TiffResult<f32> {
    //     match self {
    //         Float(val) => Ok(val),
    //         val => Err(TiffError::FormatError(
    //             TiffFormatError::SignedIntegerExpected(val),
    //         )),
    //     }
    // }

    // pub fn into_f64(self) -> TiffResult<f64> {
    //     match self {
    //         Double(val) => Ok(val),
    //         val => Err(TiffError::FormatError(
    //             TiffFormatError::SignedIntegerExpected(val),
    //         )),
    //     }
    // }

    // pub fn into_string(self) -> TiffResult<String> {
    //     match self {
    //         Ascii(val) => Ok(val),
    //         val => Err(TiffError::FormatError(
    //             TiffFormatError::SignedIntegerExpected(val),
    //         )),
    //     }
    // }

    // pub fn into_u32_vec(self) -> TiffResult<Vec<u32>> {
    //     match self {
    //         List(vec) => {
    //             let mut new_vec = Vec::with_capacity(vec.len());
    //             for v in vec {
    //                 new_vec.push(v.into_u32()?)
    //             }
    //             Ok(new_vec)
    //         }
    //         Long(val) => Ok(vec![val]),
    //         Long8(val) => Ok(vec![u32::try_from(val)?]),
    //         Rational(numerator, denominator) => Ok(vec![numerator, denominator]),
    //         // RationalBig(numerator, denominator) => {
    //         //     Ok(vec![u32::try_from(numerator)?, u32::try_from(denominator)?])
    //         // }
    //         // Ifd(val) => Ok(vec![val]),
    //         // IfdBig(val) => Ok(vec![u32::try_from(val)?]),
    //         Ascii(val) => Ok(val.chars().map(u32::from).collect()),
    //         val => Err(TiffError::FormatError(
    //             TiffFormatError::UnsignedIntegerExpected(val),
    //         )),
    //     }
    // }

    // pub fn into_u8_vec(self) -> TiffResult<Vec<u8>> {
    //     match self {
    //         List(vec) => {
    //             let mut new_vec = Vec::with_capacity(vec.len());
    //             for v in vec {
    //                 new_vec.push(v.into_u8()?)
    //             }
    //             Ok(new_vec)
    //         }
    //         Byte(val) => Ok(vec![val]),

    //         val => Err(TiffError::FormatError(
    //             TiffFormatError::UnsignedIntegerExpected(val),
    //         )),
    //     }
    // }

    // pub fn into_u16_vec(self) -> TiffResult<Vec<u16>> {
    //     match self {
    //         List(vec) => {
    //             let mut new_vec = Vec::with_capacity(vec.len());
    //             for v in vec {
    //                 new_vec.push(v.into_u16()?)
    //             }
    //             Ok(new_vec)
    //         }
    //         Short(val) => Ok(vec![val]),
    //         val => Err(TiffError::FormatError(
    //             TiffFormatError::UnsignedIntegerExpected(val),
    //         )),
    //     }
    // }

    // pub fn into_i32_vec(self) -> TiffResult<Vec<i32>> {
    //     match self {
    //         List(vec) => {
    //             let mut new_vec = Vec::with_capacity(vec.len());
    //             for v in vec {
    //                 match v {
    //                     SRational(numerator, denominator) => {
    //                         new_vec.push(numerator);
    //                         new_vec.push(denominator);
    //                     }
    //                     // SRationalBig(numerator, denominator) => {
    //                     //     new_vec.push(i32::try_from(numerator)?);
    //                     //     new_vec.push(i32::try_from(denominator)?);
    //                     // }
    //                     _ => new_vec.push(v.into_i32()?),
    //                 }
    //             }
    //             Ok(new_vec)
    //         }
    //         SignedByte(val) => Ok(vec![val.into()]),
    //         SShort(val) => Ok(vec![val.into()]),
    //         SLong(val) => Ok(vec![val]),
    //         SLong8(val) => Ok(vec![i32::try_from(val)?]),
    //         SRational(numerator, denominator) => Ok(vec![numerator, denominator]),
    //         // SRationalBig(numerator, denominator) => {
    //         //     Ok(vec![i32::try_from(numerator)?, i32::try_from(denominator)?])
    //         // }
    //         val => Err(TiffError::FormatError(
    //             TiffFormatError::SignedIntegerExpected(val),
    //         )),
    //     }
    // }

    // pub fn into_f32_vec(self) -> TiffResult<Vec<f32>> {
    //     match self {
    //         List(vec) => {
    //             let mut new_vec = Vec::with_capacity(vec.len());
    //             for v in vec {
    //                 new_vec.push(v.into_f32()?)
    //             }
    //             Ok(new_vec)
    //         }
    //         Float(val) => Ok(vec![val]),
    //         val => Err(TiffError::FormatError(
    //             TiffFormatError::UnsignedIntegerExpected(val),
    //         )),
    //     }
    // }

    // pub fn into_f64_vec(self) -> TiffResult<Vec<f64>> {
    //     match self {
    //         List(vec) => {
    //             let mut new_vec = Vec::with_capacity(vec.len());
    //             for v in vec {
    //                 new_vec.push(v.into_f64()?)
    //             }
    //             Ok(new_vec)
    //         }
    //         Double(val) => Ok(vec![val]),
    //         val => Err(TiffError::FormatError(
    //             TiffFormatError::UnsignedIntegerExpected(val),
    //         )),
    //     }
    // }

    // pub fn into_u64_vec(self) -> TiffResult<Vec<u64>> {
    //     match self {
    //         List(vec) => {
    //             let mut new_vec = Vec::with_capacity(vec.len());
    //             for v in vec {
    //                 new_vec.push(v.into_u64()?)
    //             }
    //             Ok(new_vec)
    //         }
    //         Long(val) => Ok(vec![val.into()]),
    //         Long8(val) => Ok(vec![val]),
    //         Rational(numerator, denominator) => Ok(vec![numerator.into(), denominator.into()]),
    //         // RationalBig(numerator, denominator) => Ok(vec![numerator, denominator]),
    //         // Ifd(val) => Ok(vec![val.into()]),
    //         // IfdBig(val) => Ok(vec![val]),
    //         Ascii(val) => Ok(val.chars().map(u32::from).map(u64::from).collect()),
    //         val => Err(TiffError::FormatError(
    //             TiffFormatError::UnsignedIntegerExpected(val),
    //         )),
    //     }
    // }

    // pub fn into_i64_vec(self) -> TiffResult<Vec<i64>> {
    //     match self {
    //         List(vec) => {
    //             let mut new_vec = Vec::with_capacity(vec.len());
    //             for v in vec {
    //                 match v {
    //                     SRational(numerator, denominator) => {
    //                         new_vec.push(numerator.into());
    //                         new_vec.push(denominator.into());
    //                     }
    //                     // SRationalBig(numerator, denominator) => {
    //                     //     new_vec.push(numerator);
    //                     //     new_vec.push(denominator);
    //                     // }
    //                     _ => new_vec.push(v.into_i64()?),
    //                 }
    //             }
    //             Ok(new_vec)
    //         }
    //         SignedByte(val) => Ok(vec![val.into()]),
    //         SShort(val) => Ok(vec![val.into()]),
    //         SLong(val) => Ok(vec![val.into()]),
    //         SLong8(val) => Ok(vec![val]),
    //         SRational(numerator, denominator) => Ok(vec![numerator.into(), denominator.into()]),
    //         // SRationalBig(numerator, denominator) => Ok(vec![numerator, denominator]),
    //         val => Err(TiffError::FormatError(
    //             TiffFormatError::SignedIntegerExpected(val),
    //         )),
    //     }
    // }
}
