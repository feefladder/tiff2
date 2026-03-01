use std::ops::Range;

mod tile;
pub use tile::{ChunkOptsError, ChunkOptsErrorKind, CodingError, CodingErrorKind};
mod entry;
pub use entry::{CastError, CastErrorKind};
mod ifd;
pub use ifd::{IfdError, IfdErrorKind};

/// The error status.
///
/// This is a coarse-grained "Can I retry" flag.
#[derive(Debug, Clone, PartialEq)]
pub enum ErrorStatus {
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

impl std::fmt::Display for ErrorStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self {
            ErrorStatus::Permanent => write!(f, "permanent error"),
            ErrorStatus::MissingRange { required } => {
                write!(f, "range {required:?} required, please retry")
            }
            ErrorStatus::MissingRanges { required } => {
                write!(f, "ranges {required:?} required, please retry")
            }
        }
    }
}
