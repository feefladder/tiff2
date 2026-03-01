use std::fmt::Display;

use smallvec::SmallVec;

use crate::structs::TagType;

#[derive(Debug, Clone, PartialEq)]
pub struct CastError {
    pub kind: CastErrorKind,
}

#[derive(Debug, Clone, PartialEq)]
pub enum CastErrorKind {
    InvalidCast {
        tag_type: TagType,
        target_type: &'static str,
    },
    Overflow {
        value: i128,
        target_type: &'static str,
    },
    MultipleValues {
        number_of_values: usize,
    },
    InvalidString {
        str: SmallVec<[u8; 8]>,
    },
    InvalidBuffer {
        len: usize,
        req_len: usize,
    },
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

impl Display for CastError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "cast error: {}", self.kind)
    }
}
impl std::error::Error for CastError {}
impl Display for CastErrorKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CastErrorKind::MultipleValues { number_of_values } => {
                write!(f, "expected single value, found {number_of_values}")
            }
            CastErrorKind::Overflow { value, target_type } => {
                write!(f, "{value} overflowed {target_type}")
            }
            CastErrorKind::InvalidCast {
                tag_type,
                target_type,
            } => write!(f, "casting {tag_type:?} to {target_type} is invalid"),
            CastErrorKind::InvalidString { str } => write!(f, "string {str:?} not 0-ended ASCII"),
            CastErrorKind::InvalidBuffer { len, req_len } => {
                write!(f, "can't fit {req_len} tag data in buffer of size {len}")
            }
        }
    }
}
