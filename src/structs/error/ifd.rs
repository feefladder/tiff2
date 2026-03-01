use std::{error::Error, fmt::Display, ops::Range};

use crate::structs::Tag;

use super::ErrorStatus;

#[derive(Debug, Clone, PartialEq)]
pub struct IfdError {
    status: ErrorStatus,
    kind: IfdErrorKind,
}

#[derive(Debug, Clone, PartialEq)]
pub enum IfdErrorKind {
    NotFound { tag: Tag },
    NotLoaded { tag: Tag, range: Range<u64> },
}

impl IfdError {
    pub(crate) fn not_found(tag: Tag) -> Self {
        Self {
            status: ErrorStatus::Permanent,
            kind: IfdErrorKind::NotFound { tag },
        }
    }

    pub(crate) fn not_loaded(tag: Tag, range: Range<u64>) -> Self {
        Self {
            status: ErrorStatus::Permanent,
            kind: IfdErrorKind::NotLoaded { tag, range },
        }
    }
}

impl Display for IfdError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.status, self.kind)
    }
}
impl Error for IfdError {}
impl Display for IfdErrorKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self {
            IfdErrorKind::NotFound { tag } => write!(f, "tag {tag:?} not found"),
            IfdErrorKind::NotLoaded { tag, range } => {
                write!(f, "tag {tag:?} at {range:?} not loaded")
            }
        }
    }
}
