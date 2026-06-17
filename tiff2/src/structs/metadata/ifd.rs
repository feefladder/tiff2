use std::any::TypeId;
use std::collections::{BTreeMap, HashMap};

use crate::structs::error::{IfdError, USIZE64};
use crate::structs::{IfdEntry, Offset, Tag, TagData, TagType, TiffExtension};

type IfdResult<T> = exn::Result<T, IfdError>;
type Directory = BTreeMap<Tag, IfdEntry>;

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
#[non_exhaustive]
pub struct Ifd {
    // TODO: should an Ifd know its offset?
    // I think not, since it should be in a tiff and the tiff knows its offset
    /// Tags loaded into this ifd
    pub tags: BTreeMap<Tag, TagData>,
    /// Tags not loaded, but stored as offsets
    // TODO: I'm conflicted about having tag offsets here, ideally an IFD should be fully loaded
    // I'll keep it here for now, but ideally there'd be a nice way to ??keep IfdLoaders around while also loading tiles??
    pub tag_offsets: BTreeMap<Tag, Offset>,
    /// Tags parsed as an extensions
    pub extensions: Vec<Box<dyn TiffExtension>>,
    pub ext_typeid_idx: HashMap<TypeId, usize>,
    pub ext_tag_idx: BTreeMap<u16, usize>,
}

/// Base IFD struct without any special-cased metadata
impl Ifd {
    /// The number of entries in this ifd
    pub fn count(&self) -> usize {
        self.tags.len() + self.tag_offsets.len() + self.ext_tag_idx.len()
    }

    pub(crate) fn from_tags(data: BTreeMap<Tag, IfdEntry>) -> Self {
        let mut tags = BTreeMap::new();
        let mut tag_offsets = BTreeMap::new();
        for (tag, entry) in data {
            match entry {
                IfdEntry::Value(v) => {
                    tags.insert(tag, v);
                }
                IfdEntry::Offset(o) => {
                    tag_offsets.insert(tag, o);
                }
            }
        }
        Self {
            tags,
            tag_offsets,
            ..Default::default()
        }
    }

    // /// remove a required tag from this Ifd, so we can use it as fast-access
    // /// in a wrapping struct.
    // ///
    // /// edge-case: tag is present, but value not loaded:
    // /// The tag gets re-inserted into the dict and RequiredTagNotLoaded is returned
    // pub fn remove_required_val(&mut self, tag: &Tag) -> TiffResult<TagData> {
    //     match self
    //         .data
    //         .remove(tag)
    //         .ok_or(TiffFormatError::RequiredTagNotFound(*tag))?
    //     {
    //         IfdEntry::Offset(o) => {
    //             // insert back into the ifd
    //             self.data.insert(*tag, IfdEntry::Offset(o));
    //             Err(UsageError::RequiredTagNotLoaded(*tag, o).into())
    //         }
    //         IfdEntry::Value(be) => Ok(be),
    //     }
    // }

    // /// remove an optional tag from this Ifd, so it can be used as fast-access
    // /// in a wrapping struct.
    // ///
    // /// edge-case: tag is present, but value not loaded:
    // /// The tag gets re-inserted into the dict and RequiredTagNotLoaded is returned
    // pub fn remove_optional_val(&mut self, tag: &Tag) -> TiffResult<Option<TagData>> {
    //     match self.data.remove(tag) {
    //         Some(IfdEntry::Offset(o)) => {
    //             self.data.insert(*tag, IfdEntry::Offset(o));
    //             Err(UsageError::RequiredTagNotLoaded(*tag, o).into())
    //         }
    //         Some(IfdEntry::Value(v)) => Ok(Some(v)),
    //         None => Ok(None),
    //     }
    // }

    /// Get a tag, returning error if not present or loaded
    pub(crate) fn require_val(&self, tag: &Tag) -> IfdResult<&TagData> {
        if let Some(val) = self.tags.get(tag) {
            Ok(val)
        } else {
            if let Some(o) = self.tag_offsets.get(tag) {
                Err(IfdError::not_loaded(*tag, o.range()).into())
            } else {
                Err(IfdError::not_found(*tag).into())
            }
        }
    }

    /// Get the byte length of a tag
    pub(crate) fn tag_byte_length(&self, tag: &Tag) -> IfdResult<u64> {
        if let Some(val) = self.tags.get(tag) {
            Ok(val.blen().try_into().expect(USIZE64))
        } else if let Some(o) = self.tag_offsets.get(tag) {
            Ok(o.len())
        } else {
            Err(IfdError::not_found(*tag).into())
        }
    }

    pub(crate) fn tag_n_values(&self, tag: &Tag) -> IfdResult<u64> {
        if let Some(val) = self.tags.get(tag) {
            Ok(u64::try_from(val.len()).expect(USIZE64))
        } else if let Some(o) = self.tag_offsets.get(tag) {
            Ok(o.len() / u64::try_from(o.tag_type.size()).unwrap())
        } else {
            Err(IfdError::not_found(*tag).into())
        }
    }

    pub(crate) fn contains_tag(&self, tag: &Tag) -> bool {
        self.tags.contains_key(tag)
            || self.tag_offsets.contains_key(tag)
            || self.ext_tag_idx.contains_key(&tag.to_u16())
    }

    // /// get a tag, returning error if not loaded, Ok(None) if not present
    // pub fn get_tag_value(&self, tag: &Tag) -> TiffResult<Option<&TagData>> {
    //     if let Some(be) = self.get_tag(tag) {
    //         match be {
    //             IfdEntry::Offset(o) => Err(UsageError::RequiredTagNotLoaded(*tag, *o).into()),
    //             IfdEntry::Value(be) => Ok(Some(be)),
    //         }
    //     } else {
    //         Ok(None)
    //     }
    // }

    // pub fn contains_key(&self, tag: &Tag) -> bool {
    //     self.data.contains_key(tag)
    // }

    // /// Insert related tag data into this IFD. Will error if the tag is not present
    // pub fn insert_tag_data(&mut self, tag_data: BTreeMap<Tag, TagData>) -> TiffResult<()> {
    //     for (tag, value) in tag_data {
    //         self.data
    //             .insert(tag, IfdEntry::Value(value))
    //             .ok_or(UsageError::TagOfDataNotPresent(tag))?;
    //     }
    //     Ok(())
    // }
}

#[allow(unused_imports, clippy::useless_conversion)]
mod test_ifd {
    use smallvec::smallvec;

    use super::*;
    use crate::structs::{Offset, TagData, TagType};
}
