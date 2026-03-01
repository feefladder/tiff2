use std::error::Error;
use std::fmt::Display;

use crate::structs::error::ErrorStatus;
use crate::structs::tags::{CompressionMethod, SampleFormat};

#[non_exhaustive]
#[derive(Debug, Clone, PartialEq)]
pub struct ChunkOptsError {
    pub status: ErrorStatus,
    pub kind: ChunkOptsErrorKind,
}
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq)]
pub enum ChunkOptsErrorKind {
    IndexError {
        given: usize,
        max: usize,
    },
    InvalidDataType {
        sample_format: SampleFormat,
        bit_depth: u8,
    },
}

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

impl ChunkOptsError {
    pub(crate) fn index_error(given: usize, max: usize) -> Self {
        Self {
            status: ErrorStatus::Permanent,
            kind: ChunkOptsErrorKind::IndexError { given, max },
        }
    }

    pub(crate) fn invalid_datatype(sample_format: SampleFormat, bit_depth: u8) -> Self {
        Self {
            status: ErrorStatus::Permanent,
            kind: ChunkOptsErrorKind::InvalidDataType {
                sample_format,
                bit_depth,
            },
        }
    }
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

impl Display for ChunkOptsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.status, self.kind)
    }
}
impl Error for ChunkOptsError {}
impl Display for ChunkOptsErrorKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self {
            ChunkOptsErrorKind::IndexError { given, max } => {
                write!(f, "given index {given} must be smaller than {max}")
            }
            ChunkOptsErrorKind::InvalidDataType {
                sample_format,
                bit_depth,
            } => write!(f, "{sample_format:?} with {bit_depth} unsupported"),
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
