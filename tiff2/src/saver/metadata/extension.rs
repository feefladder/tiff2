use std::any::{Any, TypeId};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt::Debug;

use crate::structs::{Tag, TagData};

pub struct TiffExtSaverRegistry {
    factories: Vec<Box<dyn TiffExtSaverFactory>>,
    id_idx: HashMap<TypeId, usize>,
    tag_idx: HashMap<u16, usize>,
}

pub trait TiffExtSaverFactory: Debug + Send + Sync {
    fn supported_tags(&self) -> BTreeSet<u16>;
    fn create_saver(&self) -> Box<dyn TiffExtSaver>;
}

pub trait TiffExtSaver: Debug + Any + Send + Sync {
    /// Convert this saver into equivalent tags
    fn to_tags(self) -> BTreeMap<Tag, TagData>;
    /// The number of tags this extension will write
    fn count(&self) -> usize;
}
