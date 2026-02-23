//! Tile error types
//!
//! Currently these look a lot like MetaErrors
//!
// TODO:
use std::ops::Range;

use crate::loader::metadata::error::MetaErrorStatus;
use crate::loader::tile::Tile;

#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct TileError {
    status: MetaErrorStatus,
    message: String,
}

impl TileError {
    pub(crate) fn permanent(message: String) -> Self {
        Self {
            status: MetaErrorStatus::Permanent,
            message,
        }
    }

    pub(crate) fn missing_range(range: Range<u64>, message: String) -> Self {
        Self {
            status: MetaErrorStatus::MissingRange { required: range },
            message,
        }
    }

    pub(crate) fn missing_ranges(
        ranges: &mut dyn Iterator<Item = Range<u64>>,
        message: String,
    ) -> Self {
        Self {
            status: MetaErrorStatus::MissingRanges {
                required: ranges.collect(),
            },
            message,
        }
    }
}

impl std::fmt::Display for TileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.status, self.message)
    }
}
impl std::error::Error for TileError {}
