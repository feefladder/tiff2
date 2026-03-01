//! Tile error types
//!
//! Currently these look a lot like MetaErrors
//!
// TODO: move shared stuff to more top-level, how many error types do I want????
use std::{error::Error, fmt::Display, ops::Range};

use crate::structs::{
    error::ErrorStatus,
    tags::{CompressionMethod, SampleFormat},
};

// error definitions
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq)]
pub struct TileError {
    pub status: ErrorStatus,
    pub message: String,
}

// creation functions
impl TileError {
    pub(crate) fn permanent(message: String) -> Self {
        Self {
            status: ErrorStatus::Permanent,
            message,
        }
    }

    pub(crate) fn missing_range(range: Range<u64>, message: String) -> Self {
        Self {
            status: ErrorStatus::MissingRange { required: range },
            message,
        }
    }

    pub(crate) fn missing_ranges(
        ranges: &mut dyn Iterator<Item = Range<u64>>,
        message: String,
    ) -> Self {
        Self {
            status: ErrorStatus::MissingRanges {
                required: ranges.collect(),
            },
            message,
        }
    }
}

// Display + Error impls
impl Display for TileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.status, self.message)
    }
}
impl Error for TileError {}
