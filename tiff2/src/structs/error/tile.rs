use std::error::Error;
use std::fmt::Display;

use crate::structs::error::ErrorStatus;
use crate::structs::metadata::tags::CompressionMethod;

#[non_exhaustive]
#[derive(Debug, Clone, PartialEq)]
pub struct CodingError {
    pub status: ErrorStatus,
    pub kind: CodingErrorKind,
}
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq)]
pub enum CodingErrorKind {
    UnsupportedCompression(CompressionMethod),
    UnsupportedBitDepth { bit_depth: u8, reason: &'static str },
    InvalidTileIndex { x: u32, y: u32 },
    Incomplete { coded: usize, required: usize },
    Failed { message: &'static str },
}

impl CodingError {
    pub(crate) fn unsupported_compression(compression: CompressionMethod) -> Self {
        Self {
            status: ErrorStatus::Permanent,
            kind: CodingErrorKind::UnsupportedCompression(compression),
        }
    }

    pub(crate) fn unsupported_bit_depth(bit_depth: u8, reason: &'static str) -> Self {
        Self {
            status: ErrorStatus::Permanent,
            kind: CodingErrorKind::UnsupportedBitDepth { bit_depth, reason },
        }
    }

    pub(crate) fn invalid_tile_index(x: u32, y: u32) -> Self {
        Self {
            status: ErrorStatus::Permanent,
            kind: CodingErrorKind::InvalidTileIndex { x, y },
        }
    }

    pub fn incomplete(coded: usize, required: usize) -> Self {
        Self {
            status: ErrorStatus::Permanent,
            kind: CodingErrorKind::Incomplete { coded, required },
        }
    }

    pub fn failed(message: &'static str) -> Self {
        Self {
            status: ErrorStatus::Permanent,
            kind: CodingErrorKind::Failed { message },
        }
    }
}

impl Display for CodingError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.status, self.kind)
    }
}
impl Error for CodingError {}
impl Display for CodingErrorKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self {
            CodingErrorKind::UnsupportedCompression(compression) => {
                write!(f, "unsupported compression {compression:?}")
            }
            CodingErrorKind::UnsupportedBitDepth { bit_depth, reason } => {
                write!(f, "{bit_depth}-bit unsupported when {reason}")
            }
            CodingErrorKind::InvalidTileIndex { x, y } => {
                write!(f, "invalid tile index ({x},{y})")
            }
            CodingErrorKind::Incomplete { coded, required } => {
                write!(f, "coded {coded} out of {required} bytes")
            }
            CodingErrorKind::Failed { message } => write!(f, "{message}"),
        }
    }
}
