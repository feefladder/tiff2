use std::any::Any;
use std::fmt::Debug;

use derive_more::Display;

/// A parsed tiff extension
pub trait TiffExtension: Any + Debug + TiffExtEq + Send + Sync {
    fn as_any(&self) -> &dyn std::any::Any;
}

impl PartialEq for dyn TiffExtension + '_ {
    fn eq(&self, other: &Self) -> bool {
        self.eq_dyn(other)
    }
}

/// helper trait for dyn-compatible equality checks
pub trait TiffExtEq {
    fn eq_dyn(&self, other: &dyn TiffExtension) -> bool;
}

impl<T: TiffExtension + PartialEq> TiffExtEq for T {
    fn eq_dyn(&self, other: &dyn TiffExtension) -> bool {
        other.as_any().downcast_ref::<Self>() == Some(self)
    }
}

#[derive(Debug, Display, Clone, PartialEq, Eq)]
pub struct TiffExtError(String);
impl std::error::Error for TiffExtError {}
