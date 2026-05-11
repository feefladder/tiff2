use std::fmt::Display;

use crate::structs::Tag;

#[derive(Debug, Clone, PartialEq)]
pub struct TiffSaveError {
    pub status: SaverErrorStatus,
    pub kind: TiffSaveErrorKind,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum SaverErrorStatus {
    Permanent,
    EasyFix,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TiffSaveErrorKind {
    /// A number did not fit in the smalltiff format
    NeedBigTiff,
    /// Some data provided gave an irreversible error
    InvalidData,
    /// The provided buffer for an operation was invalid
    InvalidBuffer { required_len: u64 },
    /// An Ifd was not completely written yet, while you attempted to write it
    // TODO: remove, this is not an error, but part of the response
    IncompleteIfd { missing_tags: Vec<Tag> },
    /// Everything went well, but more writing space is needed.
    ///
    /// A middleware should intercept non-fatal errors to become this error
    ///
    /// _Will there be light?_
    NeedMoreSpace,
}

impl TiffSaveError {
    pub(crate) fn permanent(message: String) -> Self {
        Self {
            status: SaverErrorStatus::Permanent,
            kind: TiffSaveErrorKind::InvalidData,
            message,
        }
    }

    pub(crate) fn need_bigtiff(message: String) -> Self {
        Self {
            status: SaverErrorStatus::Permanent,
            kind: TiffSaveErrorKind::NeedBigTiff,
            message,
        }
    }

    pub(crate) fn invalid_buffer(required_len: u64, message: String) -> Self {
        Self {
            status: SaverErrorStatus::EasyFix,
            kind: TiffSaveErrorKind::InvalidBuffer { required_len },
            message,
        }
    }

    pub(crate) fn unfinished_ifd(todo_tags: impl Iterator<Item = Tag>, message: String) -> Self {
        Self {
            status: SaverErrorStatus::EasyFix,
            message,
            kind: TiffSaveErrorKind::IncompleteIfd {
                missing_tags: todo_tags.collect(),
            },
        }
    }
}

impl Display for TiffSaveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.status, self.message)
    }
}
impl std::error::Error for TiffSaveError {}

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
