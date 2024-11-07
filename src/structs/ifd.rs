use crate::{
    decoder::EndianReader,
    error::{TiffError, TiffFormatError, TiffResult, UsageError},
    structs::{IfdEntry, Tag, TagData},
    ByteOrder,
};

use std::{collections::BTreeMap, io};
pub type Directory = BTreeMap<Tag, IfdEntry>;

#[derive(Debug, PartialEq, Default, Clone)]
pub struct Ifd {
    pub(crate) sub_ifds: Vec<Ifd>,
    pub(crate) data: Directory,
}

/// Base IFD struct without any special-cased metadata
impl Ifd {
    /// Creates this ifd from a buffer.
    ///
    /// Tags that fit in the offset field are directly added as an
    /// `IfdEntry::Value`, otherwise it will be a `type, count, offset` struct
    pub fn from_buffer(
        buf: &[u8],
        // num_entries: u64,
        byte_order: ByteOrder,
        bigtiff: bool,
    ) -> TiffResult<Self> {
        // let n_offset_bytes =
        let mut ifd = Ifd::default();

        // maybe make this a parameter (num_entries), since we'd need to read
        // that in order for us to get the correct range of the IFD
        let mut r = EndianReader::wrap(io::Cursor::new(buf), byte_order);
        let num_entries: u64 = if bigtiff {
            r.read_u64()?
        } else {
            r.read_u16()?.into()
        };

        // Then this can be over a chunks_exact(buf, if bigtiff{})
        for _ in 0..num_entries {
            let tag = Tag::from_u16_exhaustive(r.read_u16()?);
            ifd.data
                .insert(tag, IfdEntry::from_reader(&mut r, bigtiff)?);
        }
        Ok(ifd)
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
            .remove(&tag)
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
        match self.require_tag(&tag)? {
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
        Self {
            sub_ifds: Vec::new(),
            data,
        }
    }
}

#[allow(unused_imports)]
mod test_ifd {
    use super::*;
    use crate::structs::{value::Value, Offset, TagData, TagType};

    /// test reading multiple tags, esp. whether we skip over the offset properly
    #[test]
    #[rustfmt::skip]
    fn test_multitag() {
        let cases = [
            //n_tags tag type  count    offset
            // //    // /  \  /     \   /     \
            ([2,0, 1,1, 1,0, 1,0,0,0, 42, 0, 0, 0,
                   0,1, 1,0, 1,0,0,0, 43, 0, 0, 0], TagData::Byte(vec![42]), TagData::Byte(vec![43])),
            ([2,0, 1,1, 4,0, 1,0,0,0, 42, 0, 0, 0,
                   0,1, 4,0, 1,0,0,0, 43, 0, 0, 0], TagData::Long(vec![42]), TagData::Long(vec![43])),
            ([2,0, 1,1, 9,0, 1,0,0,0, 42, 0, 0, 0,
                   0,1, 9,0, 1,0,0,0, 43, 0, 0, 0], TagData::SLong(vec![42]), TagData::SLong(vec![43])),
            ([2,0, 1,1, 1,0, 4,0,0,0, 42,42,42,42,
                   0,1, 1,0, 4,0,0,0, 43,43,43,43], TagData::Byte(vec![42;4]), TagData::Byte(vec![43;4])),
            ([2,0, 1,1, 1,0, 3,0,0,0, 42,42,42, 0,
                   0,1, 9,0, 1,0,0,0, 42, 0, 0, 0], TagData::Byte(vec![42;3]), TagData::SLong(vec![42])),
        ];
        for (buf, res1, res2) in cases {
            let t1 = Tag::from_u16_exhaustive(0x0101); // image_length
            let t2 = Tag::from_u16_exhaustive(0x0100); // image_width
            let mut dir = Directory::new();
            dir.insert(t1, IfdEntry::Value(res1));
            dir.insert(t2, IfdEntry::Value(res2));
            assert_eq!(Ifd::from_buffer(&buf[..], ByteOrder::LittleEndian, false).unwrap(), Ifd{
                sub_ifds: Vec::new(),
                data: dir
            });
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
        //n_tags tag type  count    offset
        // //    // /  \  /     \   /     \
        ([1,0, 1,1, 1, 0, 1,0,0,0, 42, 0, 0, 0], ByteOrder::LittleEndian, TagData::Byte      (vec![42])                ),
        ([0,1, 1,1, 0, 1, 0,0,0,1, 42, 0, 0, 0], ByteOrder::BigEndian,    TagData::Byte      (vec![42])                ),
        ([1,0, 1,1, 6, 0, 1,0,0,0, 42, 0, 0, 0], ByteOrder::LittleEndian, TagData::SByte     (vec![42])                ),
        ([0,1, 1,1, 0, 6, 0,0,0,1, 42, 0, 0, 0], ByteOrder::BigEndian,    TagData::SByte     (vec![42])                ),
        ([1,0, 1,1, 7, 0, 1,0,0,0, 42, 0, 0, 0], ByteOrder::LittleEndian, TagData::Undefined (vec![42])                ),
        ([0,1, 1,1, 0, 7, 0,0,0,1, 42, 0, 0, 0], ByteOrder::BigEndian,    TagData::Undefined (vec![42])                ),
        ([1,0, 1,1, 2, 0, 1,0,0,0,  0, 0, 0, 0], ByteOrder::LittleEndian, TagData::Ascii     (vec![0 ])                ),
        ([0,1, 1,1, 0, 2, 0,0,0,1,  0, 0, 0, 0], ByteOrder::BigEndian,    TagData::Ascii     (vec![0 ])                ),
        ([1,0, 1,1, 3, 0, 1,0,0,0, 42, 0, 0, 0], ByteOrder::LittleEndian, TagData::Short     (vec![42])                ),
        ([0,1, 1,1, 0, 3, 0,0,0,1,  0,42, 0, 0], ByteOrder::BigEndian,    TagData::Short     (vec![42])                ),
        ([1,0, 1,1, 8, 0, 1,0,0,0, 42, 0, 0, 0], ByteOrder::LittleEndian, TagData::SShort    (vec![42])                ),
        ([0,1, 1,1, 0, 8, 0,0,0,1,  0,42, 0, 0], ByteOrder::BigEndian,    TagData::SShort    (vec![42])                ),
        ([1,0, 1,1, 4, 0, 1,0,0,0, 42, 0, 0, 0], ByteOrder::LittleEndian, TagData::Long      (vec![42])                ),
        ([0,1, 1,1, 0, 4, 0,0,0,1,  0, 0, 0,42], ByteOrder::BigEndian,    TagData::Long      (vec![42])                ),
        ([1,0, 1,1, 9, 0, 1,0,0,0, 42, 0, 0, 0], ByteOrder::LittleEndian, TagData::SLong     (vec![42])                ),
        ([0,1, 1,1, 0, 9, 0,0,0,1,  0, 0, 0,42], ByteOrder::BigEndian,    TagData::SLong     (vec![42])                ),
        ([1,0, 1,1,13, 0, 1,0,0,0, 42, 0, 0, 0], ByteOrder::LittleEndian, TagData::Ifd       (vec![42])                ),
        ([0,1, 1,1, 0,13, 0,0,0,1,  0, 0, 0,42], ByteOrder::BigEndian,    TagData::Ifd       (vec![42])                ),
        ([1,0, 1,1,11, 0, 1,0,0,0, 42, 0, 0, 0], ByteOrder::LittleEndian, TagData::Float     (vec![f32::from_bits(42)])),
        ([0,1, 1,1, 0,11, 0,0,0,1,  0, 0, 0,42], ByteOrder::BigEndian,    TagData::Float     (vec![f32::from_bits(42)])),
        // Double doesn't fit, neither 8-types and we special-case IFD
        ];
        for (buf, byte_order, res) in cases {
            println!("Trying {buf:?}, with {byte_order:?} should become {res:?}");
            let mut dir = Directory::new();
            dir.insert(Tag::from_u16_exhaustive(0x01_01), IfdEntry::Value(res.try_into().unwrap()));
            assert_eq!(Ifd::from_buffer(&buf, byte_order, false).unwrap(), Ifd{
                sub_ifds: Vec::new(),
                data: dir
            });
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
        //     n_tags      tag   type       count            offset
        // /            \  /  \ /   \ 1 2 3 4 5 6 7 8   1  2  3  4  5  6  7  8
        ([1,0,0,0,0,0,0,0, 1,1, 1, 0, 1,0,0,0,0,0,0,0, 42, 0, 0, 0, 0, 0, 0, 0], ByteOrder::LittleEndian, TagData::Byte      (vec![42])                ),
        ([0,0,0,0,0,0,0,1, 1,1, 0, 1, 0,0,0,0,0,0,0,1, 42, 0, 0, 0, 0, 0, 0, 0], ByteOrder::BigEndian,    TagData::Byte      (vec![42])                ),
        ([1,0,0,0,0,0,0,0, 1,1, 6, 0, 1,0,0,0,0,0,0,0, 42, 0, 0, 0, 0, 0, 0, 0], ByteOrder::LittleEndian, TagData::SByte     (vec![42])                ),
        ([0,0,0,0,0,0,0,1, 1,1, 0, 6, 0,0,0,0,0,0,0,1, 42, 0, 0, 0, 0, 0, 0, 0], ByteOrder::BigEndian,    TagData::SByte     (vec![42])                ),
        ([1,0,0,0,0,0,0,0, 1,1, 7, 0, 1,0,0,0,0,0,0,0, 42, 0, 0, 0, 0, 0, 0, 0], ByteOrder::LittleEndian, TagData::Undefined (vec![42])                ),
        ([0,0,0,0,0,0,0,1, 1,1, 0, 7, 0,0,0,0,0,0,0,1, 42, 0, 0, 0, 0, 0, 0, 0], ByteOrder::BigEndian,    TagData::Undefined (vec![42])                ),
        ([1,0,0,0,0,0,0,0, 1,1, 2, 0, 1,0,0,0,0,0,0,0,  0, 0, 0, 0, 0, 0, 0, 0], ByteOrder::LittleEndian, TagData::Ascii     (vec![0 ])               ),
        ([0,0,0,0,0,0,0,1, 1,1, 0, 2, 0,0,0,0,0,0,0,1,  0, 0, 0, 0, 0, 0, 0, 0], ByteOrder::BigEndian,    TagData::Ascii     (vec![0 ])               ),
        ([1,0,0,0,0,0,0,0, 1,1, 3, 0, 1,0,0,0,0,0,0,0, 42, 0, 0, 0, 0, 0, 0, 0], ByteOrder::LittleEndian, TagData::Short     (vec![42])                ),
        ([0,0,0,0,0,0,0,1, 1,1, 0, 3, 0,0,0,0,0,0,0,1,  0,42, 0, 0, 0, 0, 0, 0], ByteOrder::BigEndian,    TagData::Short     (vec![42])                ),
        ([1,0,0,0,0,0,0,0, 1,1, 8, 0, 1,0,0,0,0,0,0,0, 42, 0, 0, 0, 0, 0, 0, 0], ByteOrder::LittleEndian, TagData::SShort    (vec![42])                ),
        ([0,0,0,0,0,0,0,1, 1,1, 0, 8, 0,0,0,0,0,0,0,1,  0,42, 0, 0, 0, 0, 0, 0], ByteOrder::BigEndian,    TagData::SShort    (vec![42])                ),
        ([1,0,0,0,0,0,0,0, 1,1, 4, 0, 1,0,0,0,0,0,0,0, 42, 0, 0, 0, 0, 0, 0, 0], ByteOrder::LittleEndian, TagData::Long      (vec![42])                ),
        ([0,0,0,0,0,0,0,1, 1,1, 0, 4, 0,0,0,0,0,0,0,1,  0, 0, 0,42, 0, 0, 0, 0], ByteOrder::BigEndian,    TagData::Long      (vec![42])                ),
        ([1,0,0,0,0,0,0,0, 1,1, 9, 0, 1,0,0,0,0,0,0,0, 42, 0, 0, 0, 0, 0, 0, 0], ByteOrder::LittleEndian, TagData::SLong     (vec![42])                ),
        ([0,0,0,0,0,0,0,1, 1,1, 0, 9, 0,0,0,0,0,0,0,1,  0, 0, 0,42, 0, 0, 0, 0], ByteOrder::BigEndian,    TagData::SLong     (vec![42])                ),
        ([1,0,0,0,0,0,0,0, 1,1,13, 0, 1,0,0,0,0,0,0,0, 42, 0, 0, 0, 0, 0, 0, 0], ByteOrder::LittleEndian, TagData::Ifd       (vec![42])                ),
        ([0,0,0,0,0,0,0,1, 1,1, 0,13, 0,0,0,0,0,0,0,1,  0, 0, 0,42, 0, 0, 0, 0], ByteOrder::BigEndian,    TagData::Ifd       (vec![42])                ),
        ([1,0,0,0,0,0,0,0, 1,1,16, 0, 1,0,0,0,0,0,0,0, 42, 0, 0, 0, 0, 0, 0, 0], ByteOrder::LittleEndian, TagData::Long8     (vec![42])                ),
        ([0,0,0,0,0,0,0,1, 1,1, 0,16, 0,0,0,0,0,0,0,1,  0, 0, 0, 0, 0, 0, 0,42], ByteOrder::BigEndian,    TagData::Long8     (vec![42])                ),
        ([1,0,0,0,0,0,0,0, 1,1,17, 0, 1,0,0,0,0,0,0,0, 42, 0, 0, 0, 0, 0, 0, 0], ByteOrder::LittleEndian, TagData::SLong8    (vec![42])                ),
        ([0,0,0,0,0,0,0,1, 1,1, 0,17, 0,0,0,0,0,0,0,1,  0, 0, 0, 0, 0, 0, 0,42], ByteOrder::BigEndian,    TagData::SLong8    (vec![42])                ),
        ([1,0,0,0,0,0,0,0, 1,1,18, 0, 1,0,0,0,0,0,0,0, 42, 0, 0, 0, 0, 0, 0, 0], ByteOrder::LittleEndian, TagData::Ifd8      (vec![42])                ),
        ([0,0,0,0,0,0,0,1, 1,1, 0,18, 0,0,0,0,0,0,0,1,  0, 0, 0, 0, 0, 0, 0,42], ByteOrder::BigEndian,    TagData::Ifd8      (vec![42])                ),
        ([1,0,0,0,0,0,0,0, 1,1,11, 0, 1,0,0,0,0,0,0,0, 42, 0, 0, 0, 0, 0, 0, 0], ByteOrder::LittleEndian, TagData::Float     (vec![f32::from_bits(42)])),
        ([0,0,0,0,0,0,0,1, 1,1, 0,11, 0,0,0,0,0,0,0,1,  0, 0, 0,42, 0, 0, 0, 0], ByteOrder::BigEndian,    TagData::Float     (vec![f32::from_bits(42)])),
        ([1,0,0,0,0,0,0,0, 1,1,12, 0, 1,0,0,0,0,0,0,0, 42, 0, 0, 0, 0, 0, 0, 0], ByteOrder::LittleEndian, TagData::Double    (vec![f64::from_bits(42)])),
        ([0,0,0,0,0,0,0,1, 1,1, 0,12, 0,0,0,0,0,0,0,1,  0, 0, 0, 0, 0, 0, 0,42], ByteOrder::BigEndian,    TagData::Double    (vec![f64::from_bits(42)])),
        ([1,0,0,0,0,0,0,0, 1,1, 5, 0, 1,0,0,0,0,0,0,0,  42,0, 0, 0,43, 0, 0, 0], ByteOrder::LittleEndian, TagData::Rational  (vec![42, 43])            ),
        ([0,0,0,0,0,0,0,1, 1,1, 0, 5, 0,0,0,0,0,0,0,1,  0, 0, 0,42, 0, 0, 0,43], ByteOrder::BigEndian,    TagData::Rational  (vec![42, 43])            ),
        ([1,0,0,0,0,0,0,0, 1,1, 10,0, 1,0,0,0,0,0,0,0, 42, 0, 0, 0,43, 0, 0, 0], ByteOrder::LittleEndian, TagData::SRational (vec![42, 43])            ),
        ([0,0,0,0,0,0,0,1, 1,1, 0,10, 0,0,0,0,0,0,0,1,  0, 0, 0,42, 0, 0, 0,43], ByteOrder::BigEndian,    TagData::SRational (vec![42, 43])            ),
        // we special-case IFD
        ];
        for (buf, byte_order, res) in cases {
            println!("         tag   type       count                 offset");
            println!("       |1 2 |1  2 |1  2  3  4  5  6  7  8 |1  2  3  4  5  6  7  8|");
            println!("Trying {buf:?}, with {byte_order:?} should become {res:?}");
            let mut dir = Directory::new();
            dir.insert(Tag::from_u16_exhaustive(0x01_01), IfdEntry::Value(res.try_into().unwrap()));
            assert_eq!(Ifd::from_buffer(&buf, byte_order, true).unwrap(), Ifd{
                sub_ifds: Vec::new(),
                data: dir
            });
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
        //n_tags tag type  count    offset
        // //    // /  \  /     \   /     \
        ([1,0, 1,1, 1, 0, 4,0,0,0, 42,42,42,42], ByteOrder::LittleEndian, TagData::Byte      (vec![42; 4]) ),
        ([0,1, 1,1, 0, 1, 0,0,0,4, 42,42,42,42], ByteOrder::BigEndian,    TagData::Byte      (vec![42; 4]) ),
        ([1,0, 1,1, 6, 0, 4,0,0,0, 42,42,42,42], ByteOrder::LittleEndian, TagData::SByte     (vec![42; 4]) ),
        ([0,1, 1,1, 0, 6, 0,0,0,4, 42,42,42,42], ByteOrder::BigEndian,    TagData::SByte     (vec![42; 4]) ),
        ([1,0, 1,1, 7, 0, 4,0,0,0, 42,42,42,42], ByteOrder::LittleEndian, TagData::Undefined (vec![42; 4]) ),
        ([0,1, 1,1, 0, 7, 0,0,0,4, 42,42,42,42], ByteOrder::BigEndian,    TagData::Undefined (vec![42; 4]) ),
        ([1,0, 1,1, 2, 0, 4,0,0,0, 42,42,42, 0], ByteOrder::LittleEndian, TagData::Ascii     ("***\0".into())),
        ([0,1, 1,1, 0, 2, 0,0,0,4, 42,42,42, 0], ByteOrder::BigEndian,    TagData::Ascii     ("***\0".into())),
        ([1,0, 1,1, 3, 0, 2,0,0,0, 42, 0,42, 0], ByteOrder::LittleEndian, TagData::Short     (vec![42; 2]) ),
        ([0,1, 1,1, 0, 3, 0,0,0,2,  0,42, 0,42], ByteOrder::BigEndian,    TagData::Short     (vec![42; 2]) ),
        ([1,0, 1,1, 8, 0, 2,0,0,0, 42, 0,42, 0], ByteOrder::LittleEndian, TagData::SShort    (vec![42; 2]) ),
        ([0,1, 1,1, 0, 8, 0,0,0,2,  0,42, 0,42], ByteOrder::BigEndian,    TagData::SShort    (vec![42; 2]) ),
        ([0,1, 1,1, 0, 2, 0,0,0,4, b'A',b'B',b'C',0], ByteOrder::BigEndian, TagData::Ascii("ABC\0".into())),
        // others don't fit, neither 8-types and we special-case IFD
        ];
        for (buf, byte_order, res) in cases {
            println!("Trying {buf:?}, with {byte_order:?} should become {res:?}");
            let mut dir = Directory::new();
            dir.insert(Tag::from_u16_exhaustive(0x01_01), IfdEntry::Value(res.try_into().unwrap()));
            assert_eq!(Ifd::from_buffer(&buf, byte_order, false).unwrap(), Ifd{
                sub_ifds: Vec::new(),
                data: dir
            });
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
        //     n_tags      tag   type       count            offset
        // /            \  /  \ /   \ 1 2 3 4 5 6 7 8   1  2  3  4  5  6  7  8
        ([1,0,0,0,0,0,0,0, 1,1, 1, 0, 8,0,0,0,0,0,0,0, 42,42,42,42,42,42,42,42], ByteOrder::LittleEndian, TagData::Byte      (vec![42                ; 8])),
        ([0,0,0,0,0,0,0,1, 1,1, 0, 1, 0,0,0,0,0,0,0,8, 42,42,42,42,42,42,42,42], ByteOrder::BigEndian,    TagData::Byte      (vec![42                ; 8])),
        ([1,0,0,0,0,0,0,0, 1,1, 6, 0, 8,0,0,0,0,0,0,0, 42,42,42,42,42,42,42,42], ByteOrder::LittleEndian, TagData::SByte     (vec![42                ; 8])),
        ([0,0,0,0,0,0,0,1, 1,1, 0, 6, 0,0,0,0,0,0,0,8, 42,42,42,42,42,42,42,42], ByteOrder::BigEndian,    TagData::SByte     (vec![42                ; 8])),
        ([1,0,0,0,0,0,0,0, 1,1, 7, 0, 8,0,0,0,0,0,0,0, 42,42,42,42,42,42,42,42], ByteOrder::LittleEndian, TagData::Undefined (vec![42                ; 8])),
        ([0,0,0,0,0,0,0,1, 1,1, 0, 7, 0,0,0,0,0,0,0,8, 42,42,42,42,42,42,42,42], ByteOrder::BigEndian,    TagData::Undefined (vec![42                ; 8])),
        ([1,0,0,0,0,0,0,0, 1,1, 2, 0, 8,0,0,0,0,0,0,0, 42,42,42,42,42,42,42, 0], ByteOrder::LittleEndian, TagData::Ascii     ("*******\0".into()           )),
        ([0,0,0,0,0,0,0,1, 1,1, 0, 2, 0,0,0,0,0,0,0,8, 42,42,42,42,42,42,42, 0], ByteOrder::BigEndian,    TagData::Ascii     ("*******\0".into()           )),
        ([1,0,0,0,0,0,0,0, 1,1, 3, 0, 4,0,0,0,0,0,0,0, 42, 0,42, 0,42, 0,42, 0], ByteOrder::LittleEndian, TagData::Short     (vec![42                ; 4])),
        ([0,0,0,0,0,0,0,1, 1,1, 0, 3, 0,0,0,0,0,0,0,4,  0,42, 0,42, 0,42, 0,42], ByteOrder::BigEndian,    TagData::Short     (vec![42                ; 4])),
        ([1,0,0,0,0,0,0,0, 1,1, 8, 0, 4,0,0,0,0,0,0,0, 42, 0,42, 0,42, 0,42, 0], ByteOrder::LittleEndian, TagData::SShort    (vec![42                ; 4])),
        ([0,0,0,0,0,0,0,1, 1,1, 0, 8, 0,0,0,0,0,0,0,4,  0,42, 0,42, 0,42, 0,42], ByteOrder::BigEndian,    TagData::SShort    (vec![42                ; 4])),
        ([1,0,0,0,0,0,0,0, 1,1, 4, 0, 2,0,0,0,0,0,0,0, 42, 0, 0, 0,42, 0, 0, 0], ByteOrder::LittleEndian, TagData::Long      (vec![42                ; 2])),
        ([0,0,0,0,0,0,0,1, 1,1, 0, 4, 0,0,0,0,0,0,0,2,  0, 0, 0,42, 0, 0, 0,42], ByteOrder::BigEndian,    TagData::Long      (vec![42                ; 2])),
        ([1,0,0,0,0,0,0,0, 1,1, 9, 0, 2,0,0,0,0,0,0,0, 42, 0, 0, 0,42, 0, 0, 0], ByteOrder::LittleEndian, TagData::SLong     (vec![42                ; 2])),
        ([0,0,0,0,0,0,0,1, 1,1, 0, 9, 0,0,0,0,0,0,0,2,  0, 0, 0,42, 0, 0, 0,42], ByteOrder::BigEndian,    TagData::SLong     (vec![42                ; 2])),
        ([1,0,0,0,0,0,0,0, 1,1,13, 0, 2,0,0,0,0,0,0,0, 42, 0, 0, 0,42, 0, 0, 0], ByteOrder::LittleEndian, TagData::Ifd       (vec![42                ; 2])),
        ([0,0,0,0,0,0,0,1, 1,1, 0,13, 0,0,0,0,0,0,0,2,  0, 0, 0,42, 0, 0, 0,42], ByteOrder::BigEndian,    TagData::Ifd       (vec![42                ; 2])),
        ([1,0,0,0,0,0,0,0, 1,1,11, 0, 2,0,0,0,0,0,0,0, 42, 0, 0, 0,42, 0, 0, 0], ByteOrder::LittleEndian, TagData::Float     (vec![f32::from_bits(42); 2])),
        ([0,0,0,0,0,0,0,1, 1,1, 0,11, 0,0,0,0,0,0,0,2,  0, 0, 0,42, 0, 0, 0,42], ByteOrder::BigEndian,    TagData::Float     (vec![f32::from_bits(42); 2])),
        // we special-case IFD
        ];
        for (buf, byte_order, res) in cases {
            println!("         tag   type       count                 offset");
            println!("       |1 2 |1  2 |1  2  3  4  5  6  7  8 |1  2  3  4  5  6  7  8|");
            println!("Trying {buf:?}, with {byte_order:?} should become {res:?}");
            let mut dir = Directory::new();
            dir.insert(Tag::from_u16_exhaustive(0x01_01), IfdEntry::Value(res.try_into().unwrap()));
            assert_eq!(Ifd::from_buffer(&buf, byte_order, true).unwrap(), Ifd{
                sub_ifds: Vec::new(),
                data: dir
            });
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
        //n_tags tag type  count    offset
        // //    // /  \  /     \   /     \
        ([1,0, 1,1, 1, 0, 5,0,0,0, 42, 0, 0, 0], ByteOrder::LittleEndian, 5, TagType::BYTE      ),
        ([0,1, 1,1, 0, 1, 0,0,0,5,  0, 0, 0,42], ByteOrder::BigEndian   , 5, TagType::BYTE      ),
        ([1,0, 1,1, 6, 0, 5,0,0,0, 42, 0, 0, 0], ByteOrder::LittleEndian, 5, TagType::SBYTE     ),
        ([0,1, 1,1, 0, 6, 0,0,0,5,  0, 0, 0,42], ByteOrder::BigEndian   , 5, TagType::SBYTE     ),
        ([1,0, 1,1, 7, 0, 5,0,0,0, 42, 0, 0, 0], ByteOrder::LittleEndian, 5, TagType::UNDEFINED ),
        ([0,1, 1,1, 0, 7, 0,0,0,5,  0, 0, 0,42], ByteOrder::BigEndian   , 5, TagType::UNDEFINED ),
        ([1,0, 1,1, 2, 0, 5,0,0,0, 42, 0, 0, 0], ByteOrder::LittleEndian, 5, TagType::ASCII     ),
        ([0,1, 1,1, 0, 2, 0,0,0,5,  0, 0, 0,42], ByteOrder::BigEndian   , 5, TagType::ASCII     ),
        ([1,0, 1,1, 3, 0, 3,0,0,0, 42, 0, 0, 0], ByteOrder::LittleEndian, 3, TagType::SHORT     ),
        ([0,1, 1,1, 0, 3, 0,0,0,3,  0, 0, 0,42], ByteOrder::BigEndian   , 3, TagType::SHORT     ),
        ([1,0, 1,1, 8, 0, 3,0,0,0, 42, 0, 0, 0], ByteOrder::LittleEndian, 3, TagType::SSHORT    ),
        ([0,1, 1,1, 0, 8, 0,0,0,3,  0, 0, 0,42], ByteOrder::BigEndian   , 3, TagType::SSHORT    ),
        ([1,0, 1,1, 4, 0, 2,0,0,0, 42, 0, 0, 0], ByteOrder::LittleEndian, 2, TagType::LONG      ),
        ([0,1, 1,1, 0, 4, 0,0,0,2,  0, 0, 0,42], ByteOrder::BigEndian   , 2, TagType::LONG      ),
        ([1,0, 1,1,13, 0, 2,0,0,0, 42, 0, 0, 0], ByteOrder::LittleEndian, 2, TagType::IFD       ),
        ([0,1, 1,1, 0,13, 0,0,0,2,  0, 0, 0,42], ByteOrder::BigEndian   , 2, TagType::IFD       ),
        ([1,0, 1,1, 9, 0, 2,0,0,0, 42, 0, 0, 0], ByteOrder::LittleEndian, 2, TagType::SLONG     ),
        ([0,1, 1,1, 0, 9, 0,0,0,2,  0, 0, 0,42], ByteOrder::BigEndian   , 2, TagType::SLONG     ),
        ([1,0, 1,1, 11,0, 2,0,0,0, 42, 0, 0, 0], ByteOrder::LittleEndian, 2, TagType::FLOAT     ),
        ([0,1, 1,1, 0,11, 0,0,0,2,  0, 0, 0,42], ByteOrder::BigEndian   , 2, TagType::FLOAT     ),
        ([1,0, 1,1, 12,0, 1,0,0,0, 42, 0, 0, 0], ByteOrder::LittleEndian, 1, TagType::DOUBLE    ),
        ([0,1, 1,1, 0,12, 0,0,0,1,  0, 0, 0,42], ByteOrder::BigEndian   , 1, TagType::DOUBLE    ),
        ([1,0, 1,1, 5, 0, 1,0,0,0, 42, 0, 0, 0], ByteOrder::LittleEndian, 1, TagType::RATIONAL  ),
        ([0,1, 1,1, 0, 5, 0,0,0,1,  0, 0, 0,42], ByteOrder::BigEndian   , 1, TagType::RATIONAL  ),
        ([1,0, 1,1, 10,0, 1,0,0,0, 42, 0, 0, 0], ByteOrder::LittleEndian, 1, TagType::SRATIONAL ),
        ([0,1, 1,1, 0,10, 0,0,0,1,  0, 0, 0,42], ByteOrder::BigEndian   , 1, TagType::SRATIONAL ),
        // Double doesn't fit, neither 8-types and we special-case IFD
        ];
        for (buf, byte_order, count, tag_type) in cases {
            println!("Trying {buf:?}, with {byte_order:?}");
            let mut dir = Directory::new();
            dir.insert(Tag::from_u16_exhaustive(0x01_01), IfdEntry::Offset(Offset { tag_type, count, offset: 42 }));
            assert_eq!(Ifd::from_buffer(&buf, byte_order, false).unwrap(), Ifd{
                sub_ifds: Vec::new(),
                data: dir
            });
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
        //     n_tags      tag   type       count            offset
        // /            \  /  \ /   \ 1 2 3 4 5 6 7 8   1  2  3  4  5  6  7  8
        ([1,0,0,0,0,0,0,0, 1,1, 1, 0, 9,0,0,0,0,0,0,0, 42, 0, 0, 0, 0, 0, 0, 0], ByteOrder::LittleEndian, 9, TagType::BYTE      ),
        ([0,0,0,0,0,0,0,1, 1,1, 0, 1, 0,0,0,0,0,0,0,9,  0, 0, 0, 0, 0, 0, 0,42], ByteOrder::BigEndian   , 9, TagType::BYTE      ),
        ([1,0,0,0,0,0,0,0, 1,1, 6, 0, 9,0,0,0,0,0,0,0, 42, 0, 0, 0, 0, 0, 0, 0], ByteOrder::LittleEndian, 9, TagType::SBYTE     ),
        ([0,0,0,0,0,0,0,1, 1,1, 0, 6, 0,0,0,0,0,0,0,9,  0, 0, 0, 0, 0, 0, 0,42], ByteOrder::BigEndian   , 9, TagType::SBYTE     ),
        ([1,0,0,0,0,0,0,0, 1,1, 7, 0, 9,0,0,0,0,0,0,0, 42, 0, 0, 0, 0, 0, 0, 0], ByteOrder::LittleEndian, 9, TagType::UNDEFINED ),
        ([0,0,0,0,0,0,0,1, 1,1, 0, 7, 0,0,0,0,0,0,0,9,  0, 0, 0, 0, 0, 0, 0,42], ByteOrder::BigEndian   , 9, TagType::UNDEFINED ),
        ([1,0,0,0,0,0,0,0, 1,1, 2, 0, 9,0,0,0,0,0,0,0, 42, 0, 0, 0, 0, 0, 0, 0], ByteOrder::LittleEndian, 9, TagType::ASCII     ),
        ([0,0,0,0,0,0,0,1, 1,1, 0, 2, 0,0,0,0,0,0,0,9,  0, 0, 0, 0, 0, 0, 0,42], ByteOrder::BigEndian   , 9, TagType::ASCII     ),
        ([1,0,0,0,0,0,0,0, 1,1, 3, 0, 5,0,0,0,0,0,0,0, 42, 0, 0, 0, 0, 0, 0, 0], ByteOrder::LittleEndian, 5, TagType::SHORT     ),
        ([0,0,0,0,0,0,0,1, 1,1, 0, 3, 0,0,0,0,0,0,0,5,  0, 0, 0, 0, 0, 0, 0,42], ByteOrder::BigEndian   , 5, TagType::SHORT     ),
        ([1,0,0,0,0,0,0,0, 1,1, 8, 0, 5,0,0,0,0,0,0,0, 42, 0, 0, 0, 0, 0, 0, 0], ByteOrder::LittleEndian, 5, TagType::SSHORT    ),
        ([0,0,0,0,0,0,0,1, 1,1, 0, 8, 0,0,0,0,0,0,0,5,  0, 0, 0, 0, 0, 0, 0,42], ByteOrder::BigEndian   , 5, TagType::SSHORT    ),
        ([1,0,0,0,0,0,0,0, 1,1, 4, 0, 3,0,0,0,0,0,0,0, 42, 0, 0, 0, 0, 0, 0, 0], ByteOrder::LittleEndian, 3, TagType::LONG      ),
        ([0,0,0,0,0,0,0,1, 1,1, 0, 4, 0,0,0,0,0,0,0,3,  0, 0, 0, 0, 0, 0, 0,42], ByteOrder::BigEndian   , 3, TagType::LONG      ),
        ([1,0,0,0,0,0,0,0, 1,1, 9, 0, 3,0,0,0,0,0,0,0, 42, 0, 0, 0, 0, 0, 0, 0], ByteOrder::LittleEndian, 3, TagType::SLONG     ),
        ([0,0,0,0,0,0,0,1, 1,1, 0, 9, 0,0,0,0,0,0,0,3,  0, 0, 0, 0, 0, 0, 0,42], ByteOrder::BigEndian   , 3, TagType::SLONG     ),
        ([1,0,0,0,0,0,0,0, 1,1,13, 0, 3,0,0,0,0,0,0,0, 42, 0, 0, 0, 0, 0, 0, 0], ByteOrder::LittleEndian, 3, TagType::IFD       ),
        ([0,0,0,0,0,0,0,1, 1,1, 0,13, 0,0,0,0,0,0,0,3,  0, 0, 0, 0, 0, 0, 0,42], ByteOrder::BigEndian   , 3, TagType::IFD       ),
        ([1,0,0,0,0,0,0,0, 1,1,16, 0, 3,0,0,0,0,0,0,0, 42, 0, 0, 0, 0, 0, 0, 0], ByteOrder::LittleEndian, 3, TagType::LONG8     ),
        ([0,0,0,0,0,0,0,1, 1,1, 0,16, 0,0,0,0,0,0,0,3,  0, 0, 0, 0, 0, 0, 0,42], ByteOrder::BigEndian   , 3, TagType::LONG8     ),
        ([1,0,0,0,0,0,0,0, 1,1,17, 0, 3,0,0,0,0,0,0,0, 42, 0, 0, 0, 0, 0, 0, 0], ByteOrder::LittleEndian, 3, TagType::SLONG8    ),
        ([0,0,0,0,0,0,0,1, 1,1, 0,17, 0,0,0,0,0,0,0,3,  0, 0, 0, 0, 0, 0, 0,42], ByteOrder::BigEndian   , 3, TagType::SLONG8    ),
        ([1,0,0,0,0,0,0,0, 1,1,18, 0, 3,0,0,0,0,0,0,0, 42, 0, 0, 0, 0, 0, 0, 0], ByteOrder::LittleEndian, 3, TagType::IFD8      ),
        ([0,0,0,0,0,0,0,1, 1,1, 0,18, 0,0,0,0,0,0,0,3,  0, 0, 0, 0, 0, 0, 0,42], ByteOrder::BigEndian   , 3, TagType::IFD8      ),
        ([1,0,0,0,0,0,0,0, 1,1,11, 0, 3,0,0,0,0,0,0,0, 42, 0, 0, 0, 0, 0, 0, 0], ByteOrder::LittleEndian, 3, TagType::FLOAT     ),
        ([0,0,0,0,0,0,0,1, 1,1, 0,11, 0,0,0,0,0,0,0,3,  0, 0, 0, 0, 0, 0, 0,42], ByteOrder::BigEndian   , 3, TagType::FLOAT     ),
        ([1,0,0,0,0,0,0,0, 1,1,12, 0, 2,0,0,0,0,0,0,0, 42, 0, 0, 0, 0, 0, 0, 0], ByteOrder::LittleEndian, 2, TagType::DOUBLE    ),
        ([0,0,0,0,0,0,0,1, 1,1, 0,12, 0,0,0,0,0,0,0,2,  0, 0, 0, 0, 0, 0, 0,42], ByteOrder::BigEndian   , 2, TagType::DOUBLE    ),
        ([1,0,0,0,0,0,0,0, 1,1, 5, 0, 2,0,0,0,0,0,0,0, 42, 0, 0, 0, 0, 0, 0, 0], ByteOrder::LittleEndian, 2, TagType::RATIONAL  ),
        ([0,0,0,0,0,0,0,1, 1,1, 0, 5, 0,0,0,0,0,0,0,2,  0, 0, 0, 0, 0, 0, 0,42], ByteOrder::BigEndian   , 2, TagType::RATIONAL  ),
        ([1,0,0,0,0,0,0,0, 1,1,10, 0, 2,0,0,0,0,0,0,0, 42, 0, 0, 0, 0, 0, 0, 0], ByteOrder::LittleEndian, 2, TagType::SRATIONAL ),
        ([0,0,0,0,0,0,0,1, 1,1, 0,10, 0,0,0,0,0,0,0,2,  0, 0, 0, 0, 0, 0, 0,42], ByteOrder::BigEndian   , 2, TagType::SRATIONAL ),
        // we special-case IFD
        ];
        for (buf, byte_order, count, tag_type) in cases {
            println!("         tag   type       count                 offset");
            println!("       |1 2 |1  2 |1  2  3  4  5  6  7  8 |1  2  3  4  5  6  7  8|");
            println!("Trying {buf:?}, with {byte_order:?}");
            let mut dir = Directory::new();
            dir.insert(Tag::from_u16_exhaustive(0x01_01), IfdEntry::Offset(Offset{ tag_type, count, offset: 42 }));
            assert_eq!(Ifd::from_buffer(&buf, byte_order, true).unwrap(), Ifd{
                sub_ifds: Vec::new(),
                data: dir
            });
        }
    }
}
