use std::collections::BTreeMap;
use std::io;

use log::debug;

use crate::error::{TiffError, TiffFormatError, TiffResult, UsageError};
use crate::loader::EndianReader;
use crate::structs::{IfdEntry, Tag, TagData};
use crate::ByteOrder;

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

#[derive(Debug, PartialEq, Default, Clone)]
pub struct Ifd {
    // TODO: should an Ifd know its offset?
    pub(crate) data: Directory,
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

    /// Creates this ifd from a buffer.
    ///
    /// Tags that fit in the offset field are directly added as an
    /// `IfdEntry::Value`, otherwise it will be an  `Offset{type, count, offset}` struct
    pub fn from_buffer(
        buf: &[u8],
        num_entries: u64,
        byte_order: ByteOrder,
        bigtiff: bool,
    ) -> TiffResult<(Self, u64)> {
        // maybe make this a parameter (num_entries), since we'd need to read
        // that in order for us to get the correct range of the IFD
        let mut r = EndianReader::wrap(io::Cursor::new(buf), byte_order);
        // let num_entries: u64 = if bigtiff {
        //     r.read_u64()?
        // } else {
        //     r.read_u16()?.into()
        // };

        let mut directory = BTreeMap::new();
        // Then this can be over a chunks_exact(buf, if bigtiff{})
        // so it turns out that parallel iteration is actually slower
        for _ in 0..num_entries {
            let tag = Tag::from_u16_exhaustive(r.read_u16()?);
            directory.insert(tag, IfdEntry::from_reader(&mut r, bigtiff)?);
        }
        let next = if bigtiff {
            r.read_u64()?
        } else {
            r.read_u32()?.into()
        };
        debug!("next ifd: {next}");
        Ok((directory.into(), next))
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
    // /// Put the data corresponding to tag in self
    // ///
    // /// Can be used like:
    // /// ```
    // /// # let ifd = Ifd::default();
    // /// # ifd.data.insert(Tag::TileOffsets, IfdEntry::Offset(TagType::LONG8, 1, 42));
    // /// let tag = Tag::TileOffsets;
    // /// if let IfdEntry::Offset(tag_type, count, offset) = ifd.get(Tag::TileOffsets) {
    // ///     let mut buf = BufferedEntry::new(tag_type, count);
    // ///     reader.read_tag_data(offset, &mut buf).await?;
    // ///     fix_endianness(&mut buf, byte_order);
    // ///     ifd.insert_tag_data_from_buffer(tag, buf);
    // /// }
    // /// ```
    // ///
    // /// # returns
    // /// The old value if it was present. If this was a BufferedEntry, this is
    // /// probably an error.
    // pub fn insert_tag_data_from_buffer(
    //     &mut self,
    //     tag: &Tag,
    //     data: ,
    // ) -> Option<IfdEntry> {
    //     self.data.insert(*tag, IfdEntry::Value(data))
    // }
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

    /// test reading multiple tags, esp. whether we skip over the offset properly
    #[test]
    #[rustfmt::skip]
    fn test_multitag() {
        let cases = [
            // tag type  count    offset      next ifd
            // // /  \  /     \   /     \    /     \
            ([1,1, 1,0, 1,0,0,0, 42, 0, 0, 0,
              0,1, 1,0, 1,0,0,0, 43, 0, 0, 0, 0,0,0,0], TagData::Byte(smallvec![42]), TagData::Byte(smallvec![43])),
            ([1,1, 4,0, 1,0,0,0, 42, 0, 0, 0,
              0,1, 4,0, 1,0,0,0, 43, 0, 0, 0, 0,0,0,0], TagData::Long(smallvec![42]), TagData::Long(smallvec![43])),
            ([1,1, 9,0, 1,0,0,0, 42, 0, 0, 0,
              0,1, 9,0, 1,0,0,0, 43, 0, 0, 0, 0,0,0,0], TagData::SLong(smallvec![42]), TagData::SLong(smallvec![43])),
            ([1,1, 1,0, 4,0,0,0, 42,42,42,42,
              0,1, 1,0, 4,0,0,0, 43,43,43,43, 0,0,0,0], TagData::Byte(smallvec![42;4]), TagData::Byte(smallvec![43;4])),
            ([1,1, 1,0, 3,0,0,0, 42,42,42, 0,
              0,1, 9,0, 1,0,0,0, 42, 0, 0, 0, 0,0,0,0], TagData::Byte(smallvec![42;3]), TagData::SLong(smallvec![42])),
        ];
        for (buf, res1, res2) in cases {
            let t1 = Tag::from_u16_exhaustive(0x0101); // image_length
            let t2 = Tag::from_u16_exhaustive(0x0100); // image_width
            let mut dir = Directory::new();
            dir.insert(t1, IfdEntry::Value(res1));
            dir.insert(t2, IfdEntry::Value(res2));
            assert_eq!(Ifd::from_buffer(&buf[..], 2, ByteOrder::LittleEndian, false).unwrap(), (Ifd{data: dir},0));
        }
    }

    // -----------------------------------------------------------------
    // tests below are copy-pasted from Entry. Make sure to update there
    // accordingly
    // -----------------------------------------------------------------
    #[test]
    #[rustfmt::skip]
    fn test_fits_single_notbig() {
        // personal sanity checks
        assert_eq!(u16::from_le_bytes([42,0]),42);
        assert_eq!(u16::from_be_bytes([0,42]),42);

        assert_eq!(f32::from_le_bytes([0x42,0,0,0]),f32::from_bits(0x00_00_00_42));
        assert_eq!(f32::from_be_bytes([0,0,0,0x42]),f32::from_bits(0x00_00_00_42));
        let cases = [
        //  tag type  count      offset    next ifd
        //  // /  \  /     \   /        \  /     \
        ([1,1, 1, 0, 1,0,0,0, 42, 0, 0, 0, 0,0,0,0], ByteOrder::LittleEndian, TagData::Byte      (smallvec![42])                ),
        ([1,1, 0, 1, 0,0,0,1, 42, 0, 0, 0, 0,0,0,0], ByteOrder::BigEndian,    TagData::Byte      (smallvec![42])                ),
        ([1,1, 6, 0, 1,0,0,0, 42, 0, 0, 0, 0,0,0,0], ByteOrder::LittleEndian, TagData::SByte     (smallvec![42])                ),
        ([1,1, 0, 6, 0,0,0,1, 42, 0, 0, 0, 0,0,0,0], ByteOrder::BigEndian,    TagData::SByte     (smallvec![42])                ),
        ([1,1, 7, 0, 1,0,0,0, 42, 0, 0, 0, 0,0,0,0], ByteOrder::LittleEndian, TagData::Undefined (smallvec![42])                ),
        ([1,1, 0, 7, 0,0,0,1, 42, 0, 0, 0, 0,0,0,0], ByteOrder::BigEndian,    TagData::Undefined (smallvec![42])                ),
        ([1,1, 2, 0, 1,0,0,0,  0, 0, 0, 0, 0,0,0,0], ByteOrder::LittleEndian, TagData::Ascii     (smallvec![0 ])                ),
        ([1,1, 0, 2, 0,0,0,1,  0, 0, 0, 0, 0,0,0,0], ByteOrder::BigEndian,    TagData::Ascii     (smallvec![0 ])                ),
        ([1,1, 3, 0, 1,0,0,0, 42, 0, 0, 0, 0,0,0,0], ByteOrder::LittleEndian, TagData::Short     (smallvec![42])                ),
        ([1,1, 0, 3, 0,0,0,1,  0,42, 0, 0, 0,0,0,0], ByteOrder::BigEndian,    TagData::Short     (smallvec![42])                ),
        ([1,1, 8, 0, 1,0,0,0, 42, 0, 0, 0, 0,0,0,0], ByteOrder::LittleEndian, TagData::SShort    (smallvec![42])                ),
        ([1,1, 0, 8, 0,0,0,1,  0,42, 0, 0, 0,0,0,0], ByteOrder::BigEndian,    TagData::SShort    (smallvec![42])                ),
        ([1,1, 4, 0, 1,0,0,0, 42, 0, 0, 0, 0,0,0,0], ByteOrder::LittleEndian, TagData::Long      (smallvec![42])                ),
        ([1,1, 0, 4, 0,0,0,1,  0, 0, 0,42, 0,0,0,0], ByteOrder::BigEndian,    TagData::Long      (smallvec![42])                ),
        ([1,1, 9, 0, 1,0,0,0, 42, 0, 0, 0, 0,0,0,0], ByteOrder::LittleEndian, TagData::SLong     (smallvec![42])                ),
        ([1,1, 0, 9, 0,0,0,1,  0, 0, 0,42, 0,0,0,0], ByteOrder::BigEndian,    TagData::SLong     (smallvec![42])                ),
        ([1,1,13, 0, 1,0,0,0, 42, 0, 0, 0, 0,0,0,0], ByteOrder::LittleEndian, TagData::Ifd       (smallvec![42])                ),
        ([1,1, 0,13, 0,0,0,1,  0, 0, 0,42, 0,0,0,0], ByteOrder::BigEndian,    TagData::Ifd       (smallvec![42])                ),
        ([1,1,11, 0, 1,0,0,0, 42, 0, 0, 0, 0,0,0,0], ByteOrder::LittleEndian, TagData::Float     (smallvec![f32::from_bits(42)])),
        ([1,1, 0,11, 0,0,0,1,  0, 0, 0,42, 0,0,0,0], ByteOrder::BigEndian,    TagData::Float     (smallvec![f32::from_bits(42)])),
        // Double doesn't fit, neither 8-types and we special-case IFD
        ];
        for (buf, byte_order, res) in cases {
            println!("Trying {buf:?}, with {byte_order:?} should become {res:?}");
            let mut dir = Directory::new();
            dir.insert(Tag::from_u16_exhaustive(0x01_01), IfdEntry::Value(res));
            assert_eq!(Ifd::from_buffer(&buf, 1, byte_order, false).unwrap(), (Ifd{data: dir},0));
        }
    }

    #[test]
    #[rustfmt::skip]
    fn test_fits_single_big() {
        // personal sanity checks
        assert_eq!(u16::from_le_bytes([42,0]),42);
        assert_eq!(u16::from_be_bytes([0,42]),42);

        assert_eq!(f32::from_le_bytes([0x42,0,0,0]),f32::from_bits(0x00_00_00_42));
        assert_eq!(f32::from_be_bytes([0,0,0,0x42]),f32::from_bits(0x00_00_00_42));
        let cases = [
        //tag   type       count            offset                next ifd
        // //  /   \ 1 2 3 4 5 6 7 8   1  2  3  4  5  6  7  8  1 2 3 4 5 6 7 8
        ([1,1, 1, 0, 1,0,0,0,0,0,0,0, 42, 0, 0, 0, 0, 0, 0, 0, 0,0,0,0,0,0,0,0], ByteOrder::LittleEndian, TagData::Byte      (smallvec![42])                ),
        ([1,1, 0, 1, 0,0,0,0,0,0,0,1, 42, 0, 0, 0, 0, 0, 0, 0, 0,0,0,0,0,0,0,0], ByteOrder::BigEndian,    TagData::Byte      (smallvec![42])                ),
        ([1,1, 6, 0, 1,0,0,0,0,0,0,0, 42, 0, 0, 0, 0, 0, 0, 0, 0,0,0,0,0,0,0,0], ByteOrder::LittleEndian, TagData::SByte     (smallvec![42])                ),
        ([1,1, 0, 6, 0,0,0,0,0,0,0,1, 42, 0, 0, 0, 0, 0, 0, 0, 0,0,0,0,0,0,0,0], ByteOrder::BigEndian,    TagData::SByte     (smallvec![42])                ),
        ([1,1, 7, 0, 1,0,0,0,0,0,0,0, 42, 0, 0, 0, 0, 0, 0, 0, 0,0,0,0,0,0,0,0], ByteOrder::LittleEndian, TagData::Undefined (smallvec![42])                ),
        ([1,1, 0, 7, 0,0,0,0,0,0,0,1, 42, 0, 0, 0, 0, 0, 0, 0, 0,0,0,0,0,0,0,0], ByteOrder::BigEndian,    TagData::Undefined (smallvec![42])                ),
        ([1,1, 2, 0, 1,0,0,0,0,0,0,0,  0, 0, 0, 0, 0, 0, 0, 0, 0,0,0,0,0,0,0,0], ByteOrder::LittleEndian, TagData::Ascii     (smallvec![0 ])               ),
        ([1,1, 0, 2, 0,0,0,0,0,0,0,1,  0, 0, 0, 0, 0, 0, 0, 0, 0,0,0,0,0,0,0,0], ByteOrder::BigEndian,    TagData::Ascii     (smallvec![0 ])               ),
        ([1,1, 3, 0, 1,0,0,0,0,0,0,0, 42, 0, 0, 0, 0, 0, 0, 0, 0,0,0,0,0,0,0,0], ByteOrder::LittleEndian, TagData::Short     (smallvec![42])                ),
        ([1,1, 0, 3, 0,0,0,0,0,0,0,1,  0,42, 0, 0, 0, 0, 0, 0, 0,0,0,0,0,0,0,0], ByteOrder::BigEndian,    TagData::Short     (smallvec![42])                ),
        ([1,1, 8, 0, 1,0,0,0,0,0,0,0, 42, 0, 0, 0, 0, 0, 0, 0, 0,0,0,0,0,0,0,0], ByteOrder::LittleEndian, TagData::SShort    (smallvec![42])                ),
        ([1,1, 0, 8, 0,0,0,0,0,0,0,1,  0,42, 0, 0, 0, 0, 0, 0, 0,0,0,0,0,0,0,0], ByteOrder::BigEndian,    TagData::SShort    (smallvec![42])                ),
        ([1,1, 4, 0, 1,0,0,0,0,0,0,0, 42, 0, 0, 0, 0, 0, 0, 0, 0,0,0,0,0,0,0,0], ByteOrder::LittleEndian, TagData::Long      (smallvec![42])                ),
        ([1,1, 0, 4, 0,0,0,0,0,0,0,1,  0, 0, 0,42, 0, 0, 0, 0, 0,0,0,0,0,0,0,0], ByteOrder::BigEndian,    TagData::Long      (smallvec![42])                ),
        ([1,1, 9, 0, 1,0,0,0,0,0,0,0, 42, 0, 0, 0, 0, 0, 0, 0, 0,0,0,0,0,0,0,0], ByteOrder::LittleEndian, TagData::SLong     (smallvec![42])                ),
        ([1,1, 0, 9, 0,0,0,0,0,0,0,1,  0, 0, 0,42, 0, 0, 0, 0, 0,0,0,0,0,0,0,0], ByteOrder::BigEndian,    TagData::SLong     (smallvec![42])                ),
        ([1,1,13, 0, 1,0,0,0,0,0,0,0, 42, 0, 0, 0, 0, 0, 0, 0, 0,0,0,0,0,0,0,0], ByteOrder::LittleEndian, TagData::Ifd       (smallvec![42])                ),
        ([1,1, 0,13, 0,0,0,0,0,0,0,1,  0, 0, 0,42, 0, 0, 0, 0, 0,0,0,0,0,0,0,0], ByteOrder::BigEndian,    TagData::Ifd       (smallvec![42])                ),
        ([1,1,16, 0, 1,0,0,0,0,0,0,0, 42, 0, 0, 0, 0, 0, 0, 0, 0,0,0,0,0,0,0,0], ByteOrder::LittleEndian, TagData::Long8     (smallvec![42])                ),
        ([1,1, 0,16, 0,0,0,0,0,0,0,1,  0, 0, 0, 0, 0, 0, 0,42, 0,0,0,0,0,0,0,0], ByteOrder::BigEndian,    TagData::Long8     (smallvec![42])                ),
        ([1,1,17, 0, 1,0,0,0,0,0,0,0, 42, 0, 0, 0, 0, 0, 0, 0, 0,0,0,0,0,0,0,0], ByteOrder::LittleEndian, TagData::SLong8    (smallvec![42])                ),
        ([1,1, 0,17, 0,0,0,0,0,0,0,1,  0, 0, 0, 0, 0, 0, 0,42, 0,0,0,0,0,0,0,0], ByteOrder::BigEndian,    TagData::SLong8    (smallvec![42])                ),
        ([1,1,18, 0, 1,0,0,0,0,0,0,0, 42, 0, 0, 0, 0, 0, 0, 0, 0,0,0,0,0,0,0,0], ByteOrder::LittleEndian, TagData::Ifd8      (smallvec![42])                ),
        ([1,1, 0,18, 0,0,0,0,0,0,0,1,  0, 0, 0, 0, 0, 0, 0,42, 0,0,0,0,0,0,0,0], ByteOrder::BigEndian,    TagData::Ifd8      (smallvec![42])                ),
        ([1,1,11, 0, 1,0,0,0,0,0,0,0, 42, 0, 0, 0, 0, 0, 0, 0, 0,0,0,0,0,0,0,0], ByteOrder::LittleEndian, TagData::Float     (smallvec![f32::from_bits(42)])),
        ([1,1, 0,11, 0,0,0,0,0,0,0,1,  0, 0, 0,42, 0, 0, 0, 0, 0,0,0,0,0,0,0,0], ByteOrder::BigEndian,    TagData::Float     (smallvec![f32::from_bits(42)])),
        ([1,1,12, 0, 1,0,0,0,0,0,0,0, 42, 0, 0, 0, 0, 0, 0, 0, 0,0,0,0,0,0,0,0], ByteOrder::LittleEndian, TagData::Double    (smallvec![f64::from_bits(42)])),
        ([1,1, 0,12, 0,0,0,0,0,0,0,1,  0, 0, 0, 0, 0, 0, 0,42, 0,0,0,0,0,0,0,0], ByteOrder::BigEndian,    TagData::Double    (smallvec![f64::from_bits(42)])),
        ([1,1, 5, 0, 1,0,0,0,0,0,0,0,  42,0, 0, 0,43, 0, 0, 0, 0,0,0,0,0,0,0,0], ByteOrder::LittleEndian, TagData::Rational  (smallvec![[42, 43]])          ),
        ([1,1, 0, 5, 0,0,0,0,0,0,0,1,  0, 0, 0,42, 0, 0, 0,43, 0,0,0,0,0,0,0,0], ByteOrder::BigEndian,    TagData::Rational  (smallvec![[42, 43]])          ),
        ([1,1, 10,0, 1,0,0,0,0,0,0,0, 42, 0, 0, 0,43, 0, 0, 0, 0,0,0,0,0,0,0,0], ByteOrder::LittleEndian, TagData::SRational (smallvec![[42, 43]])          ),
        ([1,1, 0,10, 0,0,0,0,0,0,0,1,  0, 0, 0,42, 0, 0, 0,43, 0,0,0,0,0,0,0,0], ByteOrder::BigEndian,    TagData::SRational (smallvec![[42, 43]])          ),
        // we special-case IFD
        ];
        for (buf, byte_order, res) in cases {
            println!("         tag   type       count                 offset");
            println!("       |1 2 |1  2 |1  2  3  4  5  6  7  8 |1  2  3  4  5  6  7  8|");
            println!("Trying {buf:?}, with {byte_order:?} should become {res:?}");
            let mut dir = Directory::new();
            dir.insert(Tag::from_u16_exhaustive(0x01_01), IfdEntry::Value(res.try_into().unwrap()));
            assert_eq!(Ifd::from_buffer(&buf, 1, byte_order, true).unwrap(), (Ifd{data: dir},0));
        }
    }

    #[test]
    #[rustfmt::skip]
    fn test_fits_multi_notbig() {
        // personal sanity checks
        assert_eq!(u16::from_le_bytes([42,0]),42);
        assert_eq!(u16::from_be_bytes([0,42]),42);

        assert_eq!(f32::from_le_bytes([0x42,0,0,0]),f32::from_bits(0x00_00_00_42));
        assert_eq!(f32::from_be_bytes([0,0,0,0x42]),f32::from_bits(0x00_00_00_42));
        let cases = [
        //  tag type  count    offset      next ifd
        //  // /  \  /     \   /     \     /     \
        ([1,1, 1, 0, 4,0,0,0, 42,42,42,42, 0,0,0,0], ByteOrder::LittleEndian, TagData::Byte      (smallvec![42; 4]) ),
        ([1,1, 0, 1, 0,0,0,4, 42,42,42,42, 0,0,0,0], ByteOrder::BigEndian,    TagData::Byte      (smallvec![42; 4]) ),
        ([1,1, 6, 0, 4,0,0,0, 42,42,42,42, 0,0,0,0], ByteOrder::LittleEndian, TagData::SByte     (smallvec![42; 4]) ),
        ([1,1, 0, 6, 0,0,0,4, 42,42,42,42, 0,0,0,0], ByteOrder::BigEndian,    TagData::SByte     (smallvec![42; 4]) ),
        ([1,1, 7, 0, 4,0,0,0, 42,42,42,42, 0,0,0,0], ByteOrder::LittleEndian, TagData::Undefined (smallvec![42; 4]) ),
        ([1,1, 0, 7, 0,0,0,4, 42,42,42,42, 0,0,0,0], ByteOrder::BigEndian,    TagData::Undefined (smallvec![42; 4]) ),
        ([1,1, 2, 0, 4,0,0,0, 42,42,42, 0, 0,0,0,0], ByteOrder::LittleEndian, TagData::Ascii     ("***\0".as_bytes().into())),
        ([1,1, 0, 2, 0,0,0,4, 42,42,42, 0, 0,0,0,0], ByteOrder::BigEndian,    TagData::Ascii     ("***\0".as_bytes().into())),
        ([1,1, 3, 0, 2,0,0,0, 42, 0,42, 0, 0,0,0,0], ByteOrder::LittleEndian, TagData::Short     (smallvec![42; 2]) ),
        ([1,1, 0, 3, 0,0,0,2,  0,42, 0,42, 0,0,0,0], ByteOrder::BigEndian,    TagData::Short     (smallvec![42; 2]) ),
        ([1,1, 8, 0, 2,0,0,0, 42, 0,42, 0, 0,0,0,0], ByteOrder::LittleEndian, TagData::SShort    (smallvec![42; 2]) ),
        ([1,1, 0, 8, 0,0,0,2,  0,42, 0,42, 0,0,0,0], ByteOrder::BigEndian,    TagData::SShort    (smallvec![42; 2]) ),
        ([1,1, 0, 2, 0,0,0,4, b'A',b'B',b'C',0, 0,0,0,0], ByteOrder::BigEndian, TagData::Ascii("ABC\0".as_bytes().into())),
        // others don't fit, neither 8-types and we special-case IFD
        ];
        for (buf, byte_order, res) in cases {
            println!("Trying {buf:?}, with {byte_order:?} should become {res:?}");
            let mut dir = Directory::new();
            dir.insert(Tag::from_u16_exhaustive(0x01_01), IfdEntry::Value(res.try_into().unwrap()));
            assert_eq!(Ifd::from_buffer(&buf, 1, byte_order, false).unwrap(), (Ifd{data: dir},0));
        }
    }

    #[test]
    #[rustfmt::skip]
    fn test_fits_multi_big() {
        // personal sanity checks
        assert_eq!(u16::from_le_bytes([42,0]),42);
        assert_eq!(u16::from_be_bytes([0,42]),42);

        assert_eq!(f32::from_le_bytes([0x42,0,0,0]),f32::from_bits(0x00_00_00_42));
        assert_eq!(f32::from_be_bytes([0,0,0,0x42]),f32::from_bits(0x00_00_00_42));
        let cases = [
        //tag   type       count            offset
        // //  /   \ 1 2 3 4 5 6 7 8   1  2  3  4  5  6  7  8  1 2 3 4 5 6 7 8
        ([1,1, 1, 0, 8,0,0,0,0,0,0,0, 42,42,42,42,42,42,42,42, 0,0,0,0,0,0,0,0], ByteOrder::LittleEndian, TagData::Byte      (smallvec![42                ; 8])),
        ([1,1, 0, 1, 0,0,0,0,0,0,0,8, 42,42,42,42,42,42,42,42, 0,0,0,0,0,0,0,0], ByteOrder::BigEndian,    TagData::Byte      (smallvec![42                ; 8])),
        ([1,1, 6, 0, 8,0,0,0,0,0,0,0, 42,42,42,42,42,42,42,42, 0,0,0,0,0,0,0,0], ByteOrder::LittleEndian, TagData::SByte     (smallvec![42                ; 8])),
        ([1,1, 0, 6, 0,0,0,0,0,0,0,8, 42,42,42,42,42,42,42,42, 0,0,0,0,0,0,0,0], ByteOrder::BigEndian,    TagData::SByte     (smallvec![42                ; 8])),
        ([1,1, 7, 0, 8,0,0,0,0,0,0,0, 42,42,42,42,42,42,42,42, 0,0,0,0,0,0,0,0], ByteOrder::LittleEndian, TagData::Undefined (smallvec![42                ; 8])),
        ([1,1, 0, 7, 0,0,0,0,0,0,0,8, 42,42,42,42,42,42,42,42, 0,0,0,0,0,0,0,0], ByteOrder::BigEndian,    TagData::Undefined (smallvec![42                ; 8])),
        ([1,1, 2, 0, 8,0,0,0,0,0,0,0, 42,42,42,42,42,42,42, 0, 0,0,0,0,0,0,0,0], ByteOrder::LittleEndian, TagData::Ascii     ("*******\0".as_bytes().into()   )),
        ([1,1, 0, 2, 0,0,0,0,0,0,0,8, 42,42,42,42,42,42,42, 0, 0,0,0,0,0,0,0,0], ByteOrder::BigEndian,    TagData::Ascii     ("*******\0".as_bytes().into()   )),
        ([1,1, 3, 0, 4,0,0,0,0,0,0,0, 42, 0,42, 0,42, 0,42, 0, 0,0,0,0,0,0,0,0], ByteOrder::LittleEndian, TagData::Short     (smallvec![42                ; 4])),
        ([1,1, 0, 3, 0,0,0,0,0,0,0,4,  0,42, 0,42, 0,42, 0,42, 0,0,0,0,0,0,0,0], ByteOrder::BigEndian,    TagData::Short     (smallvec![42                ; 4])),
        ([1,1, 8, 0, 4,0,0,0,0,0,0,0, 42, 0,42, 0,42, 0,42, 0, 0,0,0,0,0,0,0,0], ByteOrder::LittleEndian, TagData::SShort    (smallvec![42                ; 4])),
        ([1,1, 0, 8, 0,0,0,0,0,0,0,4,  0,42, 0,42, 0,42, 0,42, 0,0,0,0,0,0,0,0], ByteOrder::BigEndian,    TagData::SShort    (smallvec![42                ; 4])),
        ([1,1, 4, 0, 2,0,0,0,0,0,0,0, 42, 0, 0, 0,42, 0, 0, 0, 0,0,0,0,0,0,0,0], ByteOrder::LittleEndian, TagData::Long      (smallvec![42                ; 2])),
        ([1,1, 0, 4, 0,0,0,0,0,0,0,2,  0, 0, 0,42, 0, 0, 0,42, 0,0,0,0,0,0,0,0], ByteOrder::BigEndian,    TagData::Long      (smallvec![42                ; 2])),
        ([1,1, 9, 0, 2,0,0,0,0,0,0,0, 42, 0, 0, 0,42, 0, 0, 0, 0,0,0,0,0,0,0,0], ByteOrder::LittleEndian, TagData::SLong     (smallvec![42                ; 2])),
        ([1,1, 0, 9, 0,0,0,0,0,0,0,2,  0, 0, 0,42, 0, 0, 0,42, 0,0,0,0,0,0,0,0], ByteOrder::BigEndian,    TagData::SLong     (smallvec![42                ; 2])),
        ([1,1,13, 0, 2,0,0,0,0,0,0,0, 42, 0, 0, 0,42, 0, 0, 0, 0,0,0,0,0,0,0,0], ByteOrder::LittleEndian, TagData::Ifd       (smallvec![42                ; 2])),
        ([1,1, 0,13, 0,0,0,0,0,0,0,2,  0, 0, 0,42, 0, 0, 0,42, 0,0,0,0,0,0,0,0], ByteOrder::BigEndian,    TagData::Ifd       (smallvec![42                ; 2])),
        ([1,1,11, 0, 2,0,0,0,0,0,0,0, 42, 0, 0, 0,42, 0, 0, 0, 0,0,0,0,0,0,0,0], ByteOrder::LittleEndian, TagData::Float     (smallvec![f32::from_bits(42); 2])),
        ([1,1, 0,11, 0,0,0,0,0,0,0,2,  0, 0, 0,42, 0, 0, 0,42, 0,0,0,0,0,0,0,0], ByteOrder::BigEndian,    TagData::Float     (smallvec![f32::from_bits(42); 2])),
        // we special-case IFD
        ];
        for (buf, byte_order, res) in cases {
            println!("         tag   type       count                 offset");
            println!("       |1 2 |1  2 |1  2  3  4  5  6  7  8 |1  2  3  4  5  6  7  8|");
            println!("Trying {buf:?}, with {byte_order:?} should become {res:?}");
            let mut dir = Directory::new();
            dir.insert(Tag::from_u16_exhaustive(0x01_01), IfdEntry::Value(res.try_into().unwrap()));
            assert_eq!(Ifd::from_buffer(&buf, 1, byte_order, true).unwrap(), (Ifd{data: dir},0));
        }
    }

    #[test]
    #[rustfmt::skip]
    fn test_notfits_notbig() {
        // personal sanity checks
        assert_eq!(u16::from_le_bytes([42,0]),42);
        assert_eq!(u16::from_be_bytes([0,42]),42);

        assert_eq!(f32::from_le_bytes([0x42,0,0,0]),f32::from_bits(0x00_00_00_42));
        assert_eq!(f32::from_be_bytes([0,0,0,0x42]),f32::from_bits(0x00_00_00_42));
        let cases = [
        //  tag type  count    offset      next ifd
        //  // /  \  /     \   /     \     /     \
        ([1,1, 1, 0, 5,0,0,0, 42, 0, 0, 0, 0,0,0,0], ByteOrder::LittleEndian, 5, TagType::BYTE      ),
        ([1,1, 0, 1, 0,0,0,5,  0, 0, 0,42, 0,0,0,0], ByteOrder::BigEndian   , 5, TagType::BYTE      ),
        ([1,1, 6, 0, 5,0,0,0, 42, 0, 0, 0, 0,0,0,0], ByteOrder::LittleEndian, 5, TagType::SBYTE     ),
        ([1,1, 0, 6, 0,0,0,5,  0, 0, 0,42, 0,0,0,0], ByteOrder::BigEndian   , 5, TagType::SBYTE     ),
        ([1,1, 7, 0, 5,0,0,0, 42, 0, 0, 0, 0,0,0,0], ByteOrder::LittleEndian, 5, TagType::UNDEFINED ),
        ([1,1, 0, 7, 0,0,0,5,  0, 0, 0,42, 0,0,0,0], ByteOrder::BigEndian   , 5, TagType::UNDEFINED ),
        ([1,1, 2, 0, 5,0,0,0, 42, 0, 0, 0, 0,0,0,0], ByteOrder::LittleEndian, 5, TagType::ASCII     ),
        ([1,1, 0, 2, 0,0,0,5,  0, 0, 0,42, 0,0,0,0], ByteOrder::BigEndian   , 5, TagType::ASCII     ),
        ([1,1, 3, 0, 3,0,0,0, 42, 0, 0, 0, 0,0,0,0], ByteOrder::LittleEndian, 3, TagType::SHORT     ),
        ([1,1, 0, 3, 0,0,0,3,  0, 0, 0,42, 0,0,0,0], ByteOrder::BigEndian   , 3, TagType::SHORT     ),
        ([1,1, 8, 0, 3,0,0,0, 42, 0, 0, 0, 0,0,0,0], ByteOrder::LittleEndian, 3, TagType::SSHORT    ),
        ([1,1, 0, 8, 0,0,0,3,  0, 0, 0,42, 0,0,0,0], ByteOrder::BigEndian   , 3, TagType::SSHORT    ),
        ([1,1, 4, 0, 2,0,0,0, 42, 0, 0, 0, 0,0,0,0], ByteOrder::LittleEndian, 2, TagType::LONG      ),
        ([1,1, 0, 4, 0,0,0,2,  0, 0, 0,42, 0,0,0,0], ByteOrder::BigEndian   , 2, TagType::LONG      ),
        ([1,1,13, 0, 2,0,0,0, 42, 0, 0, 0, 0,0,0,0], ByteOrder::LittleEndian, 2, TagType::IFD       ),
        ([1,1, 0,13, 0,0,0,2,  0, 0, 0,42, 0,0,0,0], ByteOrder::BigEndian   , 2, TagType::IFD       ),
        ([1,1, 9, 0, 2,0,0,0, 42, 0, 0, 0, 0,0,0,0], ByteOrder::LittleEndian, 2, TagType::SLONG     ),
        ([1,1, 0, 9, 0,0,0,2,  0, 0, 0,42, 0,0,0,0], ByteOrder::BigEndian   , 2, TagType::SLONG     ),
        ([1,1, 11,0, 2,0,0,0, 42, 0, 0, 0, 0,0,0,0], ByteOrder::LittleEndian, 2, TagType::FLOAT     ),
        ([1,1, 0,11, 0,0,0,2,  0, 0, 0,42, 0,0,0,0], ByteOrder::BigEndian   , 2, TagType::FLOAT     ),
        ([1,1, 12,0, 1,0,0,0, 42, 0, 0, 0, 0,0,0,0], ByteOrder::LittleEndian, 1, TagType::DOUBLE    ),
        ([1,1, 0,12, 0,0,0,1,  0, 0, 0,42, 0,0,0,0], ByteOrder::BigEndian   , 1, TagType::DOUBLE    ),
        ([1,1, 5, 0, 1,0,0,0, 42, 0, 0, 0, 0,0,0,0], ByteOrder::LittleEndian, 1, TagType::RATIONAL  ),
        ([1,1, 0, 5, 0,0,0,1,  0, 0, 0,42, 0,0,0,0], ByteOrder::BigEndian   , 1, TagType::RATIONAL  ),
        ([1,1, 10,0, 1,0,0,0, 42, 0, 0, 0, 0,0,0,0], ByteOrder::LittleEndian, 1, TagType::SRATIONAL ),
        ([1,1, 0,10, 0,0,0,1,  0, 0, 0,42, 0,0,0,0], ByteOrder::BigEndian   , 1, TagType::SRATIONAL ),
        // Double doesn't fit, neither 8-types and we special-case IFD
        ];
        for (buf, byte_order, count, tag_type) in cases {
            println!("Trying {buf:?}, with {byte_order:?}");
            let mut dir = Directory::new();
            dir.insert(Tag::from_u16_exhaustive(0x01_01), IfdEntry::Offset(Offset { tag_type, count, offset: 42 }));
            assert_eq!(Ifd::from_buffer(&buf, 1, byte_order, false).unwrap(), (Ifd{data: dir},0));
        }
    }

    #[test]
    #[rustfmt::skip]
    fn test_notfits_big() {
        // personal sanity checks
        assert_eq!(u16::from_le_bytes([42,0]),42);
        assert_eq!(u16::from_be_bytes([0,42]),42);

        assert_eq!(f32::from_le_bytes([0x42,0,0,0]),f32::from_bits(0x00_00_00_42));
        assert_eq!(f32::from_be_bytes([0,0,0,0x42]),f32::from_bits(0x00_00_00_42));
        let cases = [
        //tag   type       count            offset
        // //  /   \ 1 2 3 4 5 6 7 8   1  2  3  4  5  6  7  8  1 2 3 4 5 6 7 8
        ([1,1, 1, 0, 9,0,0,0,0,0,0,0, 42, 0, 0, 0, 0, 0, 0, 0, 0,0,0,0,0,0,0,0], ByteOrder::LittleEndian, 9, TagType::BYTE      ),
        ([1,1, 0, 1, 0,0,0,0,0,0,0,9,  0, 0, 0, 0, 0, 0, 0,42, 0,0,0,0,0,0,0,0], ByteOrder::BigEndian   , 9, TagType::BYTE      ),
        ([1,1, 6, 0, 9,0,0,0,0,0,0,0, 42, 0, 0, 0, 0, 0, 0, 0, 0,0,0,0,0,0,0,0], ByteOrder::LittleEndian, 9, TagType::SBYTE     ),
        ([1,1, 0, 6, 0,0,0,0,0,0,0,9,  0, 0, 0, 0, 0, 0, 0,42, 0,0,0,0,0,0,0,0], ByteOrder::BigEndian   , 9, TagType::SBYTE     ),
        ([1,1, 7, 0, 9,0,0,0,0,0,0,0, 42, 0, 0, 0, 0, 0, 0, 0, 0,0,0,0,0,0,0,0], ByteOrder::LittleEndian, 9, TagType::UNDEFINED ),
        ([1,1, 0, 7, 0,0,0,0,0,0,0,9,  0, 0, 0, 0, 0, 0, 0,42, 0,0,0,0,0,0,0,0], ByteOrder::BigEndian   , 9, TagType::UNDEFINED ),
        ([1,1, 2, 0, 9,0,0,0,0,0,0,0, 42, 0, 0, 0, 0, 0, 0, 0, 0,0,0,0,0,0,0,0], ByteOrder::LittleEndian, 9, TagType::ASCII     ),
        ([1,1, 0, 2, 0,0,0,0,0,0,0,9,  0, 0, 0, 0, 0, 0, 0,42, 0,0,0,0,0,0,0,0], ByteOrder::BigEndian   , 9, TagType::ASCII     ),
        ([1,1, 3, 0, 5,0,0,0,0,0,0,0, 42, 0, 0, 0, 0, 0, 0, 0, 0,0,0,0,0,0,0,0], ByteOrder::LittleEndian, 5, TagType::SHORT     ),
        ([1,1, 0, 3, 0,0,0,0,0,0,0,5,  0, 0, 0, 0, 0, 0, 0,42, 0,0,0,0,0,0,0,0], ByteOrder::BigEndian   , 5, TagType::SHORT     ),
        ([1,1, 8, 0, 5,0,0,0,0,0,0,0, 42, 0, 0, 0, 0, 0, 0, 0, 0,0,0,0,0,0,0,0], ByteOrder::LittleEndian, 5, TagType::SSHORT    ),
        ([1,1, 0, 8, 0,0,0,0,0,0,0,5,  0, 0, 0, 0, 0, 0, 0,42, 0,0,0,0,0,0,0,0], ByteOrder::BigEndian   , 5, TagType::SSHORT    ),
        ([1,1, 4, 0, 3,0,0,0,0,0,0,0, 42, 0, 0, 0, 0, 0, 0, 0, 0,0,0,0,0,0,0,0], ByteOrder::LittleEndian, 3, TagType::LONG      ),
        ([1,1, 0, 4, 0,0,0,0,0,0,0,3,  0, 0, 0, 0, 0, 0, 0,42, 0,0,0,0,0,0,0,0], ByteOrder::BigEndian   , 3, TagType::LONG      ),
        ([1,1, 9, 0, 3,0,0,0,0,0,0,0, 42, 0, 0, 0, 0, 0, 0, 0, 0,0,0,0,0,0,0,0], ByteOrder::LittleEndian, 3, TagType::SLONG     ),
        ([1,1, 0, 9, 0,0,0,0,0,0,0,3,  0, 0, 0, 0, 0, 0, 0,42, 0,0,0,0,0,0,0,0], ByteOrder::BigEndian   , 3, TagType::SLONG     ),
        ([1,1,13, 0, 3,0,0,0,0,0,0,0, 42, 0, 0, 0, 0, 0, 0, 0, 0,0,0,0,0,0,0,0], ByteOrder::LittleEndian, 3, TagType::IFD       ),
        ([1,1, 0,13, 0,0,0,0,0,0,0,3,  0, 0, 0, 0, 0, 0, 0,42, 0,0,0,0,0,0,0,0], ByteOrder::BigEndian   , 3, TagType::IFD       ),
        ([1,1,16, 0, 3,0,0,0,0,0,0,0, 42, 0, 0, 0, 0, 0, 0, 0, 0,0,0,0,0,0,0,0], ByteOrder::LittleEndian, 3, TagType::LONG8     ),
        ([1,1, 0,16, 0,0,0,0,0,0,0,3,  0, 0, 0, 0, 0, 0, 0,42, 0,0,0,0,0,0,0,0], ByteOrder::BigEndian   , 3, TagType::LONG8     ),
        ([1,1,17, 0, 3,0,0,0,0,0,0,0, 42, 0, 0, 0, 0, 0, 0, 0, 0,0,0,0,0,0,0,0], ByteOrder::LittleEndian, 3, TagType::SLONG8    ),
        ([1,1, 0,17, 0,0,0,0,0,0,0,3,  0, 0, 0, 0, 0, 0, 0,42, 0,0,0,0,0,0,0,0], ByteOrder::BigEndian   , 3, TagType::SLONG8    ),
        ([1,1,18, 0, 3,0,0,0,0,0,0,0, 42, 0, 0, 0, 0, 0, 0, 0, 0,0,0,0,0,0,0,0], ByteOrder::LittleEndian, 3, TagType::IFD8      ),
        ([1,1, 0,18, 0,0,0,0,0,0,0,3,  0, 0, 0, 0, 0, 0, 0,42, 0,0,0,0,0,0,0,0], ByteOrder::BigEndian   , 3, TagType::IFD8      ),
        ([1,1,11, 0, 3,0,0,0,0,0,0,0, 42, 0, 0, 0, 0, 0, 0, 0, 0,0,0,0,0,0,0,0], ByteOrder::LittleEndian, 3, TagType::FLOAT     ),
        ([1,1, 0,11, 0,0,0,0,0,0,0,3,  0, 0, 0, 0, 0, 0, 0,42, 0,0,0,0,0,0,0,0], ByteOrder::BigEndian   , 3, TagType::FLOAT     ),
        ([1,1,12, 0, 2,0,0,0,0,0,0,0, 42, 0, 0, 0, 0, 0, 0, 0, 0,0,0,0,0,0,0,0], ByteOrder::LittleEndian, 2, TagType::DOUBLE    ),
        ([1,1, 0,12, 0,0,0,0,0,0,0,2,  0, 0, 0, 0, 0, 0, 0,42, 0,0,0,0,0,0,0,0], ByteOrder::BigEndian   , 2, TagType::DOUBLE    ),
        ([1,1, 5, 0, 2,0,0,0,0,0,0,0, 42, 0, 0, 0, 0, 0, 0, 0, 0,0,0,0,0,0,0,0], ByteOrder::LittleEndian, 2, TagType::RATIONAL  ),
        ([1,1, 0, 5, 0,0,0,0,0,0,0,2,  0, 0, 0, 0, 0, 0, 0,42, 0,0,0,0,0,0,0,0], ByteOrder::BigEndian   , 2, TagType::RATIONAL  ),
        ([1,1,10, 0, 2,0,0,0,0,0,0,0, 42, 0, 0, 0, 0, 0, 0, 0, 0,0,0,0,0,0,0,0], ByteOrder::LittleEndian, 2, TagType::SRATIONAL ),
        ([1,1, 0,10, 0,0,0,0,0,0,0,2,  0, 0, 0, 0, 0, 0, 0,42, 0,0,0,0,0,0,0,0], ByteOrder::BigEndian   , 2, TagType::SRATIONAL ),
        // we special-case IFD
        ];
        for (buf, byte_order, count, tag_type) in cases {
            println!("         tag   type       count                 offset");
            println!("       |1 2 |1  2 |1  2  3  4  5  6  7  8 |1  2  3  4  5  6  7  8|");
            println!("Trying {buf:?}, with {byte_order:?}");
            let mut dir = Directory::new();
            dir.insert(Tag::from_u16_exhaustive(0x01_01), IfdEntry::Offset(Offset{ tag_type, count, offset: 42 }));
            assert_eq!(Ifd::from_buffer(&buf, 1, byte_order, true).unwrap(), (Ifd{data: dir},0));
        }
    }
}
