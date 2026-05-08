//! # extra tags registry
//!
//! So in async-tiff, there is a builder pattern emerging for extra tags:
//!
//! ```
//! pub trait TiffExtensionFactory: std::fmt::Debug + Send + Sync {
//!     fn name(&self) -> &str;
//!     fn create_builder(&self) -> Box<dyn TiffExtensionBuilder>
//! }
//!
//! pub trait TiffExtensionBuilder: std::fmt::Debug + Send + Sync {
//!   fn supported_tags(&self) -> &HashSet<u16>;
//!   /// insert the tag into self
//!   fn insert(&mut self, tag: u16, value: TagValue);
//!   /// Finish parsing self, returning the TiffExtension on success
//!   fn finish(&mut self) -> Result<Option<Box<dyn TiffExtension>>, AsyncTiffError>
//! }
//!
//! /// the main thing is that it can be put in a HashMap. supertrait `std::any::Any` to get the underlying data structure through a `downcast_ref()`
//! pub trait TiffExtension: std::any::Any {
//!   fn as_any(&self) -> &dyn std::any::Any;
//! }
//! ```
//!
//! which also fits the loader-tiff-saver pattern here relatively well?
//! But the question is: how to do the encoding part?
//!
//! Ideally, it'd go from a `TiffExtension -> TiffExtensionSaver -> "there is no need for a builder at this point??"`
//!
//! And then either the saver will go down to tags that can be parsed with the normal machinery, _and_ can reserve "extra space" such as "custom ifds/ghost area"
//! That should happen _before_ tag parsing and it'll just be a bit of figuring out how to do that nicely..
//!
//! The main problem is that I don't want to enforce extensions to be bidirectional, but I _do_ want to support that...
//!
//! maybe a `TiffExtensionSaver::from_extension(ext: TiffExtension)` method that's implemented on the Saver trait.
//!

use std::any::{Any, TypeId};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt::Debug;
use std::ops::Range;

use bytes::Bytes;
use derive_more::{Display, Error};
use exn::{ensure, Result};

use crate::structs::{TagData, TiffExtError, TiffExtension};

/// A registry for tiff extensions - on the loader side.
///
/// This should generally be a per-use-case dictionary of supported extensions
/// that is passed to all tiffs for loading.
///
/// ```
/// todo!()
/// ```
#[derive(Debug)]
pub struct TiffExtLoaderRegistry {
    factories: Vec<Box<dyn TiffExtLoaderFactory>>,
    id_idx: HashMap<TypeId, usize>,
    tag_idx: BTreeMap<u16, usize>,
}

/// You tried to register an extension that overlaps with an already-registered
/// extension
#[derive(Debug, Display, Error, PartialEq)]
pub enum DuplicateError {
    #[display("tags {overlapping_tags:?} overlapped with already-registered extensions")]
    OverlappingTags {
        overlapping_tags: Vec<u16>,
    },
    AlreadyRegistered,
}

impl TiffExtLoaderRegistry {
    pub fn register(
        &mut self,
        extension: Box<dyn TiffExtLoaderFactory>,
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
    /// Currently used by `IfdLoader`
    pub(crate) fn build(&self) -> (Vec<Box<dyn TiffExtLoader>>, BTreeMap<u16, usize>) {
        (
            self.factories.iter().map(|f| f.create_loader()).collect(),
            // Since we do not drop any factories, we can copy over the index
            self.tag_idx.clone(),
        )
    }
}

impl From<Vec<Box<dyn TiffExtLoaderFactory>>> for TiffExtLoaderRegistry {
    fn from(value: Vec<Box<dyn TiffExtLoaderFactory>>) -> Self {
        // yes, copy for clarity and probably very small perf hit
        let mut r = TiffExtLoaderRegistry {
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

/// A factory for creating tiff extension loaders
///
/// This is so that each IFD of a tiff can get its own extension loader
pub trait TiffExtLoaderFactory: Debug + Send + Sync {
    /// Tags supported by this extension, these and only these will be fed into this Loader
    fn supported_tags(&self) -> &BTreeSet<u16>;
    fn create_loader(&self) -> Box<dyn TiffExtLoader>;
}

pub trait TiffExtLoader: Debug + Any + Send + Sync {
    /// Insert the tag into this extension
    ///
    /// ```ignore
    /// let ext_loader = todo!();
    ///
    /// for tag in ext_loader.supported_tags() {
    ///     ext_loader.insert_tag(self.tags[tag])
    /// }
    /// ```
    fn insert_tag(&mut self, tag: u16, value: TagData);
    // Do we need an additional "needed_ranges, feed_ranges" method or
    // something? I kind of have the idea... we'd sorta-copy over the
    // IfdLoader-like api? how does that work? it has a
    // deferred_tags/deferred_ranges-like api, but ifds only have tags,
    // Therefore, `IfdLoader.deferred_tags_mut().map(|(_,t)| t.load(range))` works
    // extensions may do anything, including reading external ifds/ghost area/whatevs
    // So they don't directly have (Or I don't want to enforce) an object-per-range-that-has-a-single-function-to-load-it
    // I think the easiest is just:
    /// Ranges that are deferred from this extension.
    ///
    /// These are ranges that are _not_ tag data, such as external IFDs or ghost area
    ///
    /// If an extension loads external IFDs, it is responsible for not creating cycles.
    fn deferred_ranges(&self) -> Vec<Range<u64>> {
        Vec::new()
    }

    /// Load deferred data
    ///
    /// ```ignore
    /// let ext_loader = todo!();
    ///
    /// while !ext_loader.deferred_ranges().is_empty() {
    ///     let data = self.fetch.fetch_ranges(ext_loader.deferred_ranges());
    ///     ext_loader.load_ranges(data).or_raise(|| SomeError);
    /// }
    /// ```
    #[allow(unused_variables)]
    fn load_ranges(&mut self, data: Vec<Bytes>) -> Result<(), TiffExtError> {
        Ok(())
    }

    /// Finish parsing and return an extension
    fn finish(self: Box<Self>) -> Result<Option<Box<dyn TiffExtension>>, TiffExtError>;
}
