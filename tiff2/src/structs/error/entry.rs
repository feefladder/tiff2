use derive_more::Display;
use smallvec::SmallVec;

use crate::structs::TagType;

#[derive(Debug, Display, Clone, PartialEq)]
#[display("cast error: {kind}")]
pub struct CastError {
    pub kind: CastErrorKind,
}
impl std::error::Error for CastError {}
#[derive(Debug, Display, Clone, PartialEq)]
pub enum CastErrorKind {
    #[display("{}", 0)]
    Other(String),
    #[display("casting {tag_type:?} to {target_type} is invalid")]
    InvalidCast {
        tag_type: TagType,
        target_type: &'static str,
    },
    #[display("{value} overflowed {target_type}")]
    Overflow {
        value: i128,
        target_type: &'static str,
    },
    #[display("expected single value, found {number_of_values}")]
    MultipleValues { number_of_values: usize },
    #[display("string {str:?} not 0-ended ASCII")]
    InvalidString { str: SmallVec<[u8; 8]> },
    #[display("can't fit {req_len} tag data in buffer of size {len}")]
    InvalidBuffer { len: usize, req_len: usize },
}

impl CastError {
    pub(crate) fn invalid_cast(tag_type: TagType, target_type: &'static str) -> Self {
        Self {
            kind: CastErrorKind::InvalidCast {
                tag_type,
                target_type,
            },
        }
    }

    pub(crate) fn overflow(value: i128, target_type: &'static str) -> Self {
        Self {
            kind: CastErrorKind::Overflow { value, target_type },
        }
    }

    pub(crate) fn multiple_values(number_of_values: usize) -> Self {
        Self {
            kind: CastErrorKind::MultipleValues { number_of_values },
        }
    }

    pub(crate) fn invalid_string(str: SmallVec<[u8; 8]>) -> Self {
        Self {
            kind: CastErrorKind::InvalidString { str },
        }
    }

    pub(crate) fn invalid_buffer(len: usize, req_len: usize) -> Self {
        Self {
            kind: CastErrorKind::InvalidBuffer { len, req_len },
        }
    }
}
