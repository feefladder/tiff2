use std::error::Error;
use std::fmt::Display;
use std::ops::{Bound, Range};

use crate::structs::error::ErrorStatus;
use crate::structs::Tag;

/// The error that most caches should implement imho
///
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CacheMiss(pub Bound<usize>, pub Bound<usize>);

/// The main metadata-related error
///
///
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct MetaError {
    /// Course-grained status
    ///
    /// Based on this, a retry/raise decision can be made
    pub status: ErrorStatus,
    /// Finer grained details of the error
    ///
    /// Based on this, the behaviour of the retry/raise can be adjusted
    pub kind: MetaErrorKind,
    /// User-facing ino
    pub message: String,
}
/// The kind of error
///
/// This is a more fine-grained "what should I do?" type of error
#[derive(Debug, Clone, PartialEq)]
pub enum MetaErrorKind {
    /// The tiff file was invalid/corrupted
    ///
    /// This can be a variety of reasons, such as a cycle in offsets, an invalid
    /// tag value etc. Generally, this type of error is not solvable.
    ///
    /// However, if the reader is broken, this error can also be thrown.
    InvalidTiff,
    /// The provided buffer for an operation was invalid
    InvalidBuffer,
    /// An ImageFileDirectory was not completely loaded
    IncompleteIfd {
        ifd_offset: u64,
        missing_tags: Vec<Tag>,
    },
    /// Everything went well, but more data is needed.
    ///
    /// A middleware should intercept non-fatal errors to become this error.
    ///
    /// _Can the order of entropy be reversed?_
    NeedMoreData,
}

impl Display for CacheMiss {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "cache miss for range ")?;
        match self.0 {
            Bound::Excluded(v) => {
                write!(f, "({v})")?;
            }
            Bound::Included(v) => {
                write!(f, "[{v}")?;
            }
            Bound::Unbounded => {}
        }
        match self.1 {
            Bound::Excluded(v) => write!(f, "..{v})"),
            Bound::Included(v) => write!(f, "..={v}]"),
            Bound::Unbounded => write!(f, ".."),
        }
    }
}
impl Error for CacheMiss {}

impl MetaError {
    pub(crate) fn permanent(message: String) -> Self {
        Self {
            status: ErrorStatus::Permanent,
            kind: MetaErrorKind::InvalidTiff,
            message,
        }
    }

    pub(crate) fn invalid_buffer(range: Range<u64>, message: String) -> Self {
        Self {
            status: ErrorStatus::MissingRange { required: range },
            kind: MetaErrorKind::InvalidBuffer,
            message,
        }
    }

    pub(crate) fn invalid_ifd(ifd_offset: u64, missing_tags: Vec<Tag>, message: String) -> Self {
        Self {
            status: ErrorStatus::Permanent,
            kind: MetaErrorKind::IncompleteIfd {
                ifd_offset,
                missing_tags,
            },
            message,
        }
    }

    pub(crate) fn invalid_tag(tag: Tag) -> Self {
        Self {
            status: ErrorStatus::Permanent,
            kind: MetaErrorKind::InvalidTiff,
            message: format!("value for {tag:?} was invalid"),
        }
    }

    pub(crate) fn deferred_ifd(
        ranges: Vec<Range<u64>>,
        ifd_offset: u64,
        missing_tags: Vec<Tag>,
        message: String,
    ) -> Self {
        Self {
            status: ErrorStatus::MissingRanges { required: ranges },
            kind: MetaErrorKind::IncompleteIfd {
                ifd_offset,
                missing_tags,
            },
            message,
        }
    }
}

impl std::fmt::Display for MetaError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.status, self.message)
    }
}
impl std::error::Error for MetaError {}
