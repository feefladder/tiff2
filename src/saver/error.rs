use std::fmt::Display;
use std::ops::Range;

use crate::structs::Tag;

#[derive(Debug, Clone, PartialEq)]
pub struct SaverError {
    pub status: SaverErrorStatus,
    pub kind: SaverErrorKind,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum SaverErrorStatus {
    Permanent,
    EasyFix,
}

#[derive(Debug, Clone, PartialEq)]
pub enum SaverErrorKind {
    /// A number did not fit in the smalltiff format
    NeedBigTiff,
    /// Some data provided gave an irreversible error
    InvalidData,
    /// The provided buffer for an operation was invalid
    InvalidBuffer {
        required_len: u64,
    },
    IncompleteIfd {
        missing_tags: Vec<Tag>,
    },
    /// Everything went well, but more writing space is needed.
    ///
    /// A middleware should intercept non-fatal errors to become this error
    ///
    /// _Will there be light?_
    NeedMoreSpace,
}

impl SaverError {
    pub(crate) fn permanent(message: String) -> Self {
        Self {
            status: SaverErrorStatus::Permanent,
            kind: SaverErrorKind::InvalidData,
            message,
        }
    }

    pub(crate) fn need_bigtiff(message: String) -> Self {
        Self {
            status: SaverErrorStatus::Permanent,
            kind: SaverErrorKind::NeedBigTiff,
            message,
        }
    }

    pub(crate) fn invalid_buffer(required_len: u64, message: String) -> Self {
        Self {
            status: SaverErrorStatus::EasyFix,
            kind: SaverErrorKind::InvalidBuffer { required_len },
            message,
        }
    }
}

impl Display for SaverError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.status, self.message)
    }
}
impl std::error::Error for SaverError {}

impl Display for SaverErrorStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self {
            SaverErrorStatus::Permanent => write!(f, "permanent error"),
            SaverErrorStatus::EasyFix => {
                write!(f, "easy fix")
            }
        }
    }
}
