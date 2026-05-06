use std::any::Any;
use std::fmt::Debug;

use derive_more::Display;

/// A parsed tiff extension
pub trait TiffExtension: Any + Debug + TiffExtEqClone + Send + Sync {
    fn as_any(&self) -> &dyn std::any::Any;
}

impl PartialEq for dyn TiffExtension + '_ {
    fn eq(&self, other: &Self) -> bool {
        self.eq_dyn(other)
    }
}

impl Clone for Box<dyn TiffExtension> {
    fn clone(&self) -> Self {
        self.clone_dyn()
    }
}

/// helper trait for dyn-compatible equality checks
pub trait TiffExtEqClone {
    fn eq_dyn(&self, other: &dyn TiffExtension) -> bool;
    fn clone_dyn(&self) -> Box<dyn TiffExtension>;
}

impl<T: TiffExtension + PartialEq + Clone> TiffExtEqClone for T {
    fn eq_dyn(&self, other: &dyn TiffExtension) -> bool {
        other.as_any().downcast_ref::<Self>() == Some(self)
    }
    fn clone_dyn(&self) -> Box<dyn TiffExtension> {
        Box::new(self.clone())
    }
}

#[derive(Debug, Display, Clone, PartialEq, Eq)]
pub struct TiffExtError(String);
impl std::error::Error for TiffExtError {}
