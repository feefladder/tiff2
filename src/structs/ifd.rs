use std::collections::BTreeMap;

use crate::error::{TiffError, TiffFormatError, TiffResult, UsageError};
use crate::structs::{IfdEntry, Tag, TagData, TagType};

pub type Directory = BTreeMap<Tag, IfdEntry>;

/// The size of the `number of entries`
///
/// The start of an IFD is the number of entries in that IFD.
///
/// |small|big|
/// |-----|---|
/// |2    | 8 |
///
#[inline]
#[must_use]
pub const fn num_entries_size(bigtiff: bool) -> u64 {
    if bigtiff {
        8 // u64
    } else {
        2 // u16
    }
}

/// The size of an ifd entry
///
/// |field       |small|big|
/// |------------|:---:|:-:|
/// |tag         | 2   | 2 |
/// |type        | 2   | 2 |
/// |count       | 4   | 8 |
/// |value/offset| 4   | 8 |
/// |total       | 12  | 20|
#[inline]
#[must_use]
pub const fn entry_size(bigtiff: bool) -> u64 {
    if bigtiff {
        2 + 2 + 8 + 8
    } else {
        2 + 2 + 4 + 4
    }
}

/// The size of an offset to an IFD
///
/// This is the same in the header as at the end of each IFD.
///
/// |small|big|
/// |-----|---|
/// |4    |8  |
///
pub const fn offset_size(bigtiff: bool) -> u64 {
    if bigtiff {
        8 // u64
    } else {
        4 // u32
    }
}

/// The type of the offset field
///
/// This can also be used for the count field in an entry, since they are the same
#[inline]
pub(crate) const fn offset_tag_type(bigtiff: bool) -> TagType {
    if bigtiff {
        TagType::LONG8
    } else {
        TagType::LONG
    }
}

#[derive(Debug, PartialEq, Default, Clone)]
pub struct Ifd {
    // TODO: should an Ifd know its offset?
    pub data: Directory,
    // TODO: add custom tag registry/parsing
}

/// Base IFD struct without any special-cased metadata
impl Ifd {
    /// Iterate this IFD in increasing tag-order
    pub(crate) fn iter(&self) -> impl Iterator<Item = (&Tag, &IfdEntry)> {
        self.data.iter()
    }

    /// Iterate this IFD in increasing tag-order
    pub(crate) fn iter_mut(&mut self) -> impl Iterator<Item = (&Tag, &mut IfdEntry)> {
        self.data.iter_mut()
    }
    /// The number of entries in this ifd
    pub fn count(&self) -> usize {
        self.data.len()
    }

    pub(crate) fn from_tags(data: BTreeMap<Tag, IfdEntry>) -> Self {
        Self { data }
    }

    /// Get a tag. Will return None if the tag isn't present (in this tiff/Image)
    pub fn get_tag(&self, tag: &Tag) -> Option<&IfdEntry> {
        self.data.get(tag)
    }

    /// Get a tag, returning error if not present
    ///
    /// Can return `IfdEntry::Offset` if the tag is not loaded
    pub fn require_tag(&self, tag: &Tag) -> TiffResult<&IfdEntry> {
        self.data.get(tag).ok_or(TiffError::FormatError(
            TiffFormatError::RequiredTagNotFound(*tag),
        ))
    }

    /// remove a required tag from this Ifd, so we can use it as fast-access
    /// in a wrapping struct.
    ///
    /// edge-case: tag is present, but value not loaded:
    /// The tag gets re-inserted into the dict and RequiredTagNotLoaded is returned
    pub fn remove_required_val(&mut self, tag: &Tag) -> TiffResult<TagData> {
        match self
            .data
            .remove(tag)
            .ok_or(TiffFormatError::RequiredTagNotFound(*tag))?
        {
            IfdEntry::Offset(o) => {
                // insert back into the ifd
                self.data.insert(*tag, IfdEntry::Offset(o));
                Err(UsageError::RequiredTagNotLoaded(*tag, o).into())
            }
            IfdEntry::Value(be) => Ok(be),
        }
    }

    /// remove an optional tag from this Ifd, so it can be used as fast-access
    /// in a wrapping struct.
    ///
    /// edge-case: tag is present, but value not loaded:
    /// The tag gets re-inserted into the dict and RequiredTagNotLoaded is returned
    pub fn remove_optional_val(&mut self, tag: &Tag) -> TiffResult<Option<TagData>> {
        match self.data.remove(tag) {
            Some(IfdEntry::Offset(o)) => {
                self.data.insert(*tag, IfdEntry::Offset(o));
                Err(UsageError::RequiredTagNotLoaded(*tag, o).into())
            }
            Some(IfdEntry::Value(v)) => Ok(Some(v)),
            None => Ok(None),
        }
    }

    /// Get a tag, returning error if not present or loaded
    pub fn require_tag_value(&self, tag: &Tag) -> TiffResult<&TagData> {
        match self.require_tag(tag)? {
            IfdEntry::Offset(o) => Err(UsageError::RequiredTagNotLoaded(*tag, *o).into()),
            IfdEntry::Value(be) => Ok(be),
        }
    }

    /// get a tag, returning error if not loaded, Ok(None) if not present
    pub fn get_tag_value(&self, tag: &Tag) -> TiffResult<Option<&TagData>> {
        if let Some(be) = self.get_tag(tag) {
            match be {
                IfdEntry::Offset(o) => Err(UsageError::RequiredTagNotLoaded(*tag, *o).into()),
                IfdEntry::Value(be) => Ok(Some(be)),
            }
        } else {
            Ok(None)
        }
    }

    pub fn contains_key(&self, tag: &Tag) -> bool {
        self.data.contains_key(tag)
    }

    /// Insert related tag data into this IFD. Will error if the tag is not present
    pub fn insert_tag_data(&mut self, tag_data: BTreeMap<Tag, TagData>) -> TiffResult<()> {
        for (tag, value) in tag_data {
            self.data
                .insert(tag, IfdEntry::Value(value))
                .ok_or(UsageError::TagOfDataNotPresent(tag))?;
        }
        Ok(())
    }
}

impl From<Directory> for Ifd {
    fn from(data: Directory) -> Self {
        Self { data }
    }
}

#[allow(unused_imports, clippy::useless_conversion)]
mod test_ifd {
    use smallvec::smallvec;

    use super::*;
    use crate::structs::{Offset, TagData, TagType};
}
