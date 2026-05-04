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

use std::{any::Any, collections::BTreeSet, fmt::Debug, ops::Range};

use bytes::Bytes;
use derive_more::Display;
use exn::Result;

use crate::structs::TagData;

pub trait TiffExtension: Any {
    fn as_any(&self) -> &dyn std::any::Any;
}

#[derive(Debug, Display, Clone, PartialEq, Eq)]
pub struct TiffExtError(String);
impl std::error::Error for TiffExtError {}

pub trait TiffExtLoader: Debug + Send + Sync {
    /// Tags supported by this extension, these and only these will be fed into this Loader
    fn supported_tags(&self) -> &BTreeSet<u16>;
    /// Insert the tag into this extension
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
    /// This may at some point become `Vec<Range<u64>>`
    fn deferred_ranges(&self) -> impl std::iter::Iterator<Item = Range<u64>> {
        std::iter::empty()
    }
    /// Load deferred data
    ///
    /// This will probably stay an impl Iter
    fn load(&mut self, data: impl std::iter::Iterator<Item = Bytes>) -> Result<(), TiffExtError> {
        Ok(())
    }
    /// Finish parsing and return an extension
    fn finish(self) -> Result<Option<Box<dyn TiffExtension>>, TiffExtError>;
}
