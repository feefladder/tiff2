use std::ops::Range;

#[derive(Debug, Clone)]
pub struct MetaError {
    pub status: MetaErrorStatus,
    pub intermediate: Option<IntermediateResult>,
    pub message: String,
}

impl MetaError {
    pub fn permanent(message: String) -> Self {
        Self {
            status: MetaErrorStatus::Permanent,
            intermediate: None,
            message,
        }
    }

    pub fn missing_range(range: Range<u64>, message: String) -> Self {
        Self {
            status: MetaErrorStatus::MissingRange { required: range },
            intermediate: None,
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
        required: Vec<Range<u64>>,
    },
    Permanent,
}

#[derive(Debug, Clone)]
pub struct IntermediateResult;

impl std::fmt::Display for MetaError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.status {
            MetaErrorStatus::Permanent => write!(f, "Permanent error: {}", self.message),
            MetaErrorStatus::MissingRange { required } => write!(
                f,
                "Operation requires range {required:?}, please retry {}",
                self.message
            ),
            MetaErrorStatus::MissingRanges { required } => write!(
                f,
                "operation requires {required:?}, please retry {}",
                self.message
            ),
        }
    }
}
impl std::error::Error for MetaError {}
