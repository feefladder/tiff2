use std::any::{Any, TypeId};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt::Debug;

use exn::{ensure, Result};

// TODO: move to sensible place if it's shared
use crate::loader::DuplicateError;
use crate::structs::{Tag, TagData};

/// A registry for tiff extensions - on the saver side.
///
/// This should generally be a per-use-case dictionary of supported extensions
/// that is passed to all tiffs for saving.
///
/// ```
/// todo!()
/// ```
pub struct TiffExtSaverRegistry {
    factories: Vec<Box<dyn TiffExtSaverFactory>>,
    id_idx: HashMap<TypeId, usize>,
    tag_idx: BTreeMap<u16, usize>,
}

// This should stay up-to-date with TiffExtLoaderRegistry
impl TiffExtSaverRegistry {
    pub fn register(
        &mut self,
        extension: Box<dyn TiffExtSaverFactory>,
    ) -> Result<(), DuplicateError> {
        ensure!(
            !self.id_idx.contains_key(&extension.as_ref().type_id()),
            DuplicateError::AlreadyRegistered
        );
        let overlapping_tags = extension
            .supported_tags()
            .iter()
            .filter(|t| self.tag_idx.contains_key(t))
            .copied()
            .collect::<Vec<u16>>();
        ensure!(
            overlapping_tags.is_empty(),
            DuplicateError::OverlappingTags { overlapping_tags }
        );
        let idx = self.factories.len();
        self.id_idx.insert(extension.as_ref().type_id(), idx);
        extension.supported_tags().iter().for_each(|t| {
            self.tag_idx.insert(*t, idx);
        });
        self.factories.push(extension);
        Ok(())
    }

    /// Build this registry into loaders and tag index
    ///
    /// Currently used by `IfdSaver`
    pub(crate) fn build(&self) -> (Vec<Box<dyn TiffExtSaver>>, BTreeMap<u16, usize>) {
        (
            self.factories.iter().map(|f| f.create_saver()).collect(),
            // Since we do not drop any factories, we can copy over the index
            self.tag_idx.clone(),
        )
    }
}

impl From<Vec<Box<dyn TiffExtSaverFactory>>> for TiffExtSaverRegistry {
    fn from(value: Vec<Box<dyn TiffExtSaverFactory>>) -> Self {
        // yes, copy for clarity and probably very small perf hit
        let mut r = TiffExtSaverRegistry {
            factories: Vec::with_capacity(value.len()),
            id_idx: HashMap::with_capacity(value.len()),
            tag_idx: BTreeMap::new(),
        };
        for v in value {
            if let Err(e) = r.register(v) {
                eprintln!("extensions not registered, due to {e}")
            }
        }
        r
    }
}
pub trait TiffExtSaverFactory: Debug + Send + Sync {
    fn supported_tags(&self) -> BTreeSet<u16>;
    fn create_saver(&self) -> Box<dyn TiffExtSaver>;
}

pub trait TiffExtSaver: Debug + Any + TiffExtSaverEqClone + Send + Sync {
    /// Convert this saver into equivalent tags
    fn to_tags(self) -> BTreeMap<Tag, TagData>;
    /// The number of tags this extension will write
    fn count(&self) -> usize;
    fn as_any(&self) -> &dyn Any;
}

impl PartialEq for dyn TiffExtSaver + '_ {
    fn eq(&self, other: &Self) -> bool {
        self.eq_dyn(other)
    }
}

impl Clone for Box<dyn TiffExtSaver> {
    fn clone(&self) -> Self {
        self.clone_dyn()
    }
}

pub trait TiffExtSaverEqClone {
    fn eq_dyn(&self, other: &dyn TiffExtSaver) -> bool;
    fn clone_dyn(&self) -> Box<dyn TiffExtSaver>;
}

impl<T: TiffExtSaver + PartialEq + Clone> TiffExtSaverEqClone for T {
    fn eq_dyn(&self, other: &dyn TiffExtSaver) -> bool {
        other.as_any().downcast_ref::<Self>() == Some(self)
    }
    fn clone_dyn(&self) -> Box<dyn TiffExtSaver> {
        Box::new(self.clone())
    }
}
