use std::ops::Range;

use crate::structs::Tag;

#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct MetaError {
    pub status: MetaErrorStatus,
    pub kind: MetaErrorKind,
    pub message: String,
}

#[derive(Debug, Clone)]
pub enum MetaErrorKind {
    InvalidTiff,
    InvalidBuffer,
    IncompleteIfd {
        ifd_offset: u64,
        missing_tags: Vec<Tag>,
    },
}

impl MetaError {
    pub(crate) fn permanent(message: String) -> Self {
        Self {
            status: MetaErrorStatus::Permanent,
            kind: MetaErrorKind::InvalidTiff,
            message,
        }
    }

    pub(crate) fn invalid_buffer(range: Range<u64>, message: String) -> Self {
        Self {
            status: MetaErrorStatus::MissingRange { required: range },
            kind: MetaErrorKind::InvalidBuffer,
            message,
        }
    }

    pub(crate) fn incomplete_ifd(
        ranges: Vec<Range<u64>>,
        ifd_offset: u64,
        missing_tags: Vec<Tag>,
        message: String,
    ) -> Self {
        Self {
            status: MetaErrorStatus::MissingRanges { required: ranges },
            kind: MetaErrorKind::IncompleteIfd {
                ifd_offset,
                missing_tags,
            },
            message,
        }
    }
}

#[derive(Debug, Clone)]
pub enum MetaErrorStatus {
    /// Reading from the provided buffer failed
    ///
    /// please retry the operation, providing the required range
    MissingRange {
        required: Range<u64>,
    },
    /// Reading from the provided buffers failed
    ///
    /// please retry the operation, providing the required ranges
    MissingRanges {
        // should this become like a dyn Iterator<Item=Range<u64>>
        required: Vec<Range<u64>>,
    },
    Permanent,
}

impl std::fmt::Display for MetaErrorStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self {
            MetaErrorStatus::Permanent => write!(f, "permanent error"),
            MetaErrorStatus::MissingRange { required } => {
                write!(f, "range {required:?} required, please retry")
            }
            MetaErrorStatus::MissingRanges { required } => {
                write!(f, "ranges {required:?} required, please retry")
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct IntermediateResult;

impl std::fmt::Display for MetaError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.status, self.message)
    }
}
impl std::error::Error for MetaError {}
