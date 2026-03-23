use std::error::Error;
use std::fmt::Display;
use std::ops::{Bound, Range};

use derive_more::Display;

use crate::structs::Ifd;

/// The error that most caches should implement imho
///
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CacheMiss(pub Bound<usize>, pub Bound<usize>);

/// The main metadata-related error
///
///
#[derive(Debug, Display, Clone, PartialEq)]
#[non_exhaustive]
pub enum TiffLoadError {
    #[display("could not load ifd from buffer, need {required:?}")]
    InvalidBuffer { required: Range<u64> },
    #[display("ifd parsing failed: {message}")]
    Fatal { message: String },
}
impl Error for TiffLoadError {}

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

impl TiffLoadError {
    pub(crate) fn permanent(message: String) -> Self {
        Self::Fatal { message }
    }

    pub(crate) fn invalid_buffer(range: Range<u64>) -> Self {
        Self::InvalidBuffer { required: range }
    }
}
