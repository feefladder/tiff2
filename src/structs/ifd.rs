use crate::{
    decoder::{CogReader, EndianReader},
    error::{TiffError, TiffFormatError, TiffResult, UsageError},
    structs::entry::{BufferedEntry, IfdEntry},
    tags::Tag,
    ByteOrder,
};

use std::{collections::BTreeMap, io};
pub type Directory = BTreeMap<Tag, IfdEntry>;

#[derive(Debug, PartialEq, Default)]
pub struct Ifd {
    sub_ifds: Vec<Ifd>,
    data: Directory,
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
        let mut r = EndianReader::wrap(io::Cursor::new(buf), byte_order);
        let num_entries: u64 = if bigtiff {
            r.read_u64()?
        } else {
            r.read_u16()?.into()
        };
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

    /// Get a tag, returning error if not present or loaded
    pub fn require_tag_value(&self, tag: &Tag) -> TiffResult<&BufferedEntry> {
        match self.require_tag(&tag)? {
            IfdEntry::Offset {
                tag_type,
                count,
                offset,
            } => Err(UsageError::RequiredTagNotLoaded(*tag, *tag_type, *count, *offset).into()),
            IfdEntry::Value(be) => Ok(be),
        }
    }

    /// get a tag, returning error if not loaded, Ok(None) if not present
    pub fn get_tag_value(&self, tag: &Tag) -> TiffResult<Option<&BufferedEntry>> {
        if let Some(be) = self.get_tag(tag) {
            match be {
                IfdEntry::Offset {
                    tag_type,
                    count,
                    offset,
                } => Err(UsageError::RequiredTagNotLoaded(*tag, *tag_type, *count, *offset).into()),
                IfdEntry::Value(be) => Ok(Some(be)),
            }
        } else {
            Ok(None)
        }
    }

    pub fn contains_key(&self, tag: &Tag) -> bool {
        self.data.contains_key(tag)
    }
    /// Put the data corresponding to tag in self
    ///
    /// Can be used like:
    /// ```
    /// # let ifd = Ifd::default();
    /// # ifd.data.insert(Tag::TileOffsets, IfdEntry::Offset(TagType::LONG8, 1, 42));
    /// let tag = Tag::TileOffsets;
    /// if let IfdEntry::Offset(tag_type, count, offset) = ifd.get(Tag::TileOffsets) {
    ///     let mut buf = BufferedEntry::new(tag_type, count);
    ///     reader.read_tag_data(offset, &mut buf).await?;
    ///     fix_endianness(&mut buf, byte_order);
    ///     ifd.insert_tag_data_from_buffer(tag, buf);
    /// }
    /// ```
    ///
    /// # returns
    /// The old value if it was present. If this was a BufferedEntry, this is
    /// probably an error.
    pub fn insert_tag_data_from_buffer(
        &mut self,
        tag: &Tag,
        data: BufferedEntry,
    ) -> Option<IfdEntry> {
        self.data.insert(*tag, IfdEntry::Value(data))
    }
}

#[allow(unused_imports)]
mod test_ifd {
    use super::*;
    use crate::{tags::TagType, value::Value};

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
        ([1,0, 1,1, 1, 0, 1,0,0,0, 42, 0, 0, 0], ByteOrder::LittleEndian, Value::Byte      (42)                ),
        ([0,1, 1,1, 0, 1, 0,0,0,1, 42, 0, 0, 0], ByteOrder::BigEndian,    Value::Byte      (42)                ),
        ([1,0, 1,1, 6, 0, 1,0,0,0, 42, 0, 0, 0], ByteOrder::LittleEndian, Value::SignedByte(42)                ),
        ([0,1, 1,1, 0, 6, 0,0,0,1, 42, 0, 0, 0], ByteOrder::BigEndian,    Value::SignedByte(42)                ),
        ([1,0, 1,1, 7, 0, 1,0,0,0, 42, 0, 0, 0], ByteOrder::LittleEndian, Value::Undefined (42)                ),
        ([0,1, 1,1, 0, 7, 0,0,0,1, 42, 0, 0, 0], ByteOrder::BigEndian,    Value::Undefined (42)                ),
        ([1,0, 1,1, 2, 0, 1,0,0,0,  0, 0, 0, 0], ByteOrder::LittleEndian, Value::Ascii     ("".into())         ),
        ([0,1, 1,1, 0, 2, 0,0,0,1,  0, 0, 0, 0], ByteOrder::BigEndian,    Value::Ascii     ("".into())         ),
        ([1,0, 1,1, 3, 0, 1,0,0,0, 42, 0, 0, 0], ByteOrder::LittleEndian, Value::Short     (42)                ),
        ([0,1, 1,1, 0, 3, 0,0,0,1,  0,42, 0, 0], ByteOrder::BigEndian,    Value::Short     (42)                ),
        ([1,0, 1,1, 8, 0, 1,0,0,0, 42, 0, 0, 0], ByteOrder::LittleEndian, Value::SShort    (42)                ),
        ([0,1, 1,1, 0, 8, 0,0,0,1,  0,42, 0, 0], ByteOrder::BigEndian,    Value::SShort    (42)                ),
        ([1,0, 1,1, 4, 0, 1,0,0,0, 42, 0, 0, 0], ByteOrder::LittleEndian, Value::Long      (42)                ),
        ([0,1, 1,1, 0, 4, 0,0,0,1,  0, 0, 0,42], ByteOrder::BigEndian,    Value::Long      (42)                ),
        ([1,0, 1,1, 9, 0, 1,0,0,0, 42, 0, 0, 0], ByteOrder::LittleEndian, Value::SLong     (42)                ),
        ([0,1, 1,1, 0, 9, 0,0,0,1,  0, 0, 0,42], ByteOrder::BigEndian,    Value::SLong     (42)                ),
        ([1,0, 1,1, 11,0, 1,0,0,0, 42, 0, 0, 0], ByteOrder::LittleEndian, Value::Float     (f32::from_bits(42))),
        ([0,1, 1,1, 0,11, 0,0,0,1,  0, 0, 0,42], ByteOrder::BigEndian,    Value::Float     (f32::from_bits(42))),

        ([0,1, 1,1, 0, 2, 0,0,0,4, b'A',b'B',b'C',0], ByteOrder::BigEndian, Value::Ascii("ABC".into())),
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
        ([1,0,0,0,0,0,0,0, 1,1, 1, 0, 1,0,0,0,0,0,0,0, 42, 0, 0, 0, 0, 0, 0, 0], ByteOrder::LittleEndian, Value::Byte      (42)                ),
        ([0,0,0,0,0,0,0,1, 1,1, 0, 1, 0,0,0,0,0,0,0,1, 42, 0, 0, 0, 0, 0, 0, 0], ByteOrder::BigEndian,    Value::Byte      (42)                ),
        ([1,0,0,0,0,0,0,0, 1,1, 6, 0, 1,0,0,0,0,0,0,0, 42, 0, 0, 0, 0, 0, 0, 0], ByteOrder::LittleEndian, Value::SignedByte(42)                ),
        ([0,0,0,0,0,0,0,1, 1,1, 0, 6, 0,0,0,0,0,0,0,1, 42, 0, 0, 0, 0, 0, 0, 0], ByteOrder::BigEndian,    Value::SignedByte(42)                ),
        ([1,0,0,0,0,0,0,0, 1,1, 7, 0, 1,0,0,0,0,0,0,0, 42, 0, 0, 0, 0, 0, 0, 0], ByteOrder::LittleEndian, Value::Undefined (42)                ),
        ([0,0,0,0,0,0,0,1, 1,1, 0, 7, 0,0,0,0,0,0,0,1, 42, 0, 0, 0, 0, 0, 0, 0], ByteOrder::BigEndian,    Value::Undefined (42)                ),
        ([1,0,0,0,0,0,0,0, 1,1, 2, 0, 1,0,0,0,0,0,0,0,  0, 0, 0, 0, 0, 0, 0, 0], ByteOrder::LittleEndian, Value::Ascii     ("".into())         ),
        ([0,0,0,0,0,0,0,1, 1,1, 0, 2, 0,0,0,0,0,0,0,1,  0, 0, 0, 0, 0, 0, 0, 0], ByteOrder::BigEndian,    Value::Ascii     ("".into())         ),
        ([1,0,0,0,0,0,0,0, 1,1, 3, 0, 1,0,0,0,0,0,0,0, 42, 0, 0, 0, 0, 0, 0, 0], ByteOrder::LittleEndian, Value::Short     (42)                ),
        ([0,0,0,0,0,0,0,1, 1,1, 0, 3, 0,0,0,0,0,0,0,1,  0,42, 0, 0, 0, 0, 0, 0], ByteOrder::BigEndian,    Value::Short     (42)                ),
        ([1,0,0,0,0,0,0,0, 1,1, 8, 0, 1,0,0,0,0,0,0,0, 42, 0, 0, 0, 0, 0, 0, 0], ByteOrder::LittleEndian, Value::SShort    (42)                ),
        ([0,0,0,0,0,0,0,1, 1,1, 0, 8, 0,0,0,0,0,0,0,1,  0,42, 0, 0, 0, 0, 0, 0], ByteOrder::BigEndian,    Value::SShort    (42)                ),
        ([1,0,0,0,0,0,0,0, 1,1, 4, 0, 1,0,0,0,0,0,0,0, 42, 0, 0, 0, 0, 0, 0, 0], ByteOrder::LittleEndian, Value::Long      (42)                ),
        ([0,0,0,0,0,0,0,1, 1,1, 0, 4, 0,0,0,0,0,0,0,1,  0, 0, 0,42, 0, 0, 0, 0], ByteOrder::BigEndian,    Value::Long      (42)                ),
        ([1,0,0,0,0,0,0,0, 1,1, 9, 0, 1,0,0,0,0,0,0,0, 42, 0, 0, 0, 0, 0, 0, 0], ByteOrder::LittleEndian, Value::SLong     (42)                ),
        ([0,0,0,0,0,0,0,1, 1,1, 0, 9, 0,0,0,0,0,0,0,1,  0, 0, 0,42, 0, 0, 0, 0], ByteOrder::BigEndian,    Value::SLong     (42)                ),
        ([1,0,0,0,0,0,0,0, 1,1, 11,0, 1,0,0,0,0,0,0,0, 42, 0, 0, 0, 0, 0, 0, 0], ByteOrder::LittleEndian, Value::Float     (f32::from_bits(42))),
        ([0,0,0,0,0,0,0,1, 1,1, 0,11, 0,0,0,0,0,0,0,1,  0, 0, 0,42, 0, 0, 0, 0], ByteOrder::BigEndian,    Value::Float     (f32::from_bits(42))),
        ([1,0,0,0,0,0,0,0, 1,1, 12,0, 1,0,0,0,0,0,0,0, 42, 0, 0, 0, 0, 0, 0, 0], ByteOrder::LittleEndian, Value::Double    (f64::from_bits(42))),
        ([0,0,0,0,0,0,0,1, 1,1, 0,12, 0,0,0,0,0,0,0,1,  0, 0, 0, 0, 0, 0, 0,42], ByteOrder::BigEndian,    Value::Double    (f64::from_bits(42))),
        ([1,0,0,0,0,0,0,0, 1,1, 5, 0, 1,0,0,0,0,0,0,0,  42,0, 0, 0,13, 0, 0, 0], ByteOrder::LittleEndian, Value::Rational  (42, 13)            ),
        ([0,0,0,0,0,0,0,1, 1,1, 0, 5, 0,0,0,0,0,0,0,1,  0, 0, 0,42, 0, 0, 0,13], ByteOrder::BigEndian,    Value::Rational  (42, 13)            ),
        ([1,0,0,0,0,0,0,0, 1,1, 10,0, 1,0,0,0,0,0,0,0, 42, 0, 0, 0,13, 0, 0, 0], ByteOrder::LittleEndian, Value::SRational (42, 13)            ),
        ([0,0,0,0,0,0,0,1, 1,1, 0,10, 0,0,0,0,0,0,0,1,  0, 0, 0,42, 0, 0, 0,13], ByteOrder::BigEndian,    Value::SRational (42, 13)            ),
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
        ([1,0, 1,1, 1, 0, 4,0,0,0, 42,42,42,42], ByteOrder::LittleEndian, Value::List(vec![Value::Byte      (42); 4])     ),
        ([0,1, 1,1, 0, 1, 0,0,0,4, 42,42,42,42], ByteOrder::BigEndian,    Value::List(vec![Value::Byte      (42); 4])     ),
        ([1,0, 1,1, 6, 0, 4,0,0,0, 42,42,42,42], ByteOrder::LittleEndian, Value::List(vec![Value::SignedByte(42); 4])     ),
        ([0,1, 1,1, 0, 6, 0,0,0,4, 42,42,42,42], ByteOrder::BigEndian,    Value::List(vec![Value::SignedByte(42); 4])     ),
        ([1,0, 1,1, 7, 0, 4,0,0,0, 42,42,42,42], ByteOrder::LittleEndian, Value::List(vec![Value::Undefined (42); 4])     ),
        ([0,1, 1,1, 0, 7, 0,0,0,4, 42,42,42,42], ByteOrder::BigEndian,    Value::List(vec![Value::Undefined (42); 4])     ),
        ([1,0, 1,1, 2, 0, 4,0,0,0, 42,42,42, 0], ByteOrder::LittleEndian, Value::Ascii                      ("***".into())),
        ([0,1, 1,1, 0, 2, 0,0,0,4, 42,42,42, 0], ByteOrder::BigEndian,    Value::Ascii                      ("***".into())),
        ([1,0, 1,1, 3, 0, 2,0,0,0, 42, 0,42, 0], ByteOrder::LittleEndian, Value::List(vec![Value::Short     (42); 2])     ),
        ([0,1, 1,1, 0, 3, 0,0,0,2,  0,42, 0,42], ByteOrder::BigEndian,    Value::List(vec![Value::Short     (42); 2])     ),
        ([1,0, 1,1, 8, 0, 2,0,0,0, 42, 0,42, 0], ByteOrder::LittleEndian, Value::List(vec![Value::SShort    (42); 2])     ),
        ([0,1, 1,1, 0, 8, 0,0,0,2,  0,42, 0,42], ByteOrder::BigEndian,    Value::List(vec![Value::SShort    (42); 2])     ),

        ([0,1, 1,1, 0, 2, 0,0,0,4, b'A',b'B',b'C',0], ByteOrder::BigEndian, Value::Ascii("ABC".into())),
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
        ([1,0,0,0,0,0,0,0, 1,1, 1, 0, 8,0,0,0,0,0,0,0, 42,42,42,42,42,42,42,42], ByteOrder::LittleEndian, Value::List(vec![Value::Byte      (42)                ; 8])),
        ([0,0,0,0,0,0,0,1, 1,1, 0, 1, 0,0,0,0,0,0,0,8, 42,42,42,42,42,42,42,42], ByteOrder::BigEndian,    Value::List(vec![Value::Byte      (42)                ; 8])),
        ([1,0,0,0,0,0,0,0, 1,1, 6, 0, 8,0,0,0,0,0,0,0, 42,42,42,42,42,42,42,42], ByteOrder::LittleEndian, Value::List(vec![Value::SignedByte(42)                ; 8])),
        ([0,0,0,0,0,0,0,1, 1,1, 0, 6, 0,0,0,0,0,0,0,8, 42,42,42,42,42,42,42,42], ByteOrder::BigEndian,    Value::List(vec![Value::SignedByte(42)                ; 8])),
        ([1,0,0,0,0,0,0,0, 1,1, 7, 0, 8,0,0,0,0,0,0,0, 42,42,42,42,42,42,42,42], ByteOrder::LittleEndian, Value::List(vec![Value::Undefined (42)                ; 8])),
        ([0,0,0,0,0,0,0,1, 1,1, 0, 7, 0,0,0,0,0,0,0,8, 42,42,42,42,42,42,42,42], ByteOrder::BigEndian,    Value::List(vec![Value::Undefined (42)                ; 8])),
        ([1,0,0,0,0,0,0,0, 1,1, 2, 0, 8,0,0,0,0,0,0,0, 42,42,42,42,42,42,42, 0], ByteOrder::LittleEndian, Value::Ascii                      ("*******".into())       ),
        ([0,0,0,0,0,0,0,1, 1,1, 0, 2, 0,0,0,0,0,0,0,8, 42,42,42,42,42,42,42, 0], ByteOrder::BigEndian,    Value::Ascii                      ("*******".into())       ),
        ([1,0,0,0,0,0,0,0, 1,1, 3, 0, 4,0,0,0,0,0,0,0, 42, 0,42, 0,42, 0,42, 0], ByteOrder::LittleEndian, Value::List(vec![Value::Short     (42)                ; 4])),
        ([0,0,0,0,0,0,0,1, 1,1, 0, 3, 0,0,0,0,0,0,0,4,  0,42, 0,42, 0,42, 0,42], ByteOrder::BigEndian,    Value::List(vec![Value::Short     (42)                ; 4])),
        ([1,0,0,0,0,0,0,0, 1,1, 8, 0, 4,0,0,0,0,0,0,0, 42, 0,42, 0,42, 0,42, 0], ByteOrder::LittleEndian, Value::List(vec![Value::SShort    (42)                ; 4])),
        ([0,0,0,0,0,0,0,1, 1,1, 0, 8, 0,0,0,0,0,0,0,4,  0,42, 0,42, 0,42, 0,42], ByteOrder::BigEndian,    Value::List(vec![Value::SShort    (42)                ; 4])),
        ([1,0,0,0,0,0,0,0, 1,1, 4, 0, 2,0,0,0,0,0,0,0, 42, 0, 0, 0,42, 0, 0, 0], ByteOrder::LittleEndian, Value::List(vec![Value::Long      (42)                ; 2])),
        ([0,0,0,0,0,0,0,1, 1,1, 0, 4, 0,0,0,0,0,0,0,2,  0, 0, 0,42, 0, 0, 0,42], ByteOrder::BigEndian,    Value::List(vec![Value::Long      (42)                ; 2])),
        ([1,0,0,0,0,0,0,0, 1,1, 9, 0, 2,0,0,0,0,0,0,0, 42, 0, 0, 0,42, 0, 0, 0], ByteOrder::LittleEndian, Value::List(vec![Value::SLong     (42)                ; 2])),
        ([0,0,0,0,0,0,0,1, 1,1, 0, 9, 0,0,0,0,0,0,0,2,  0, 0, 0,42, 0, 0, 0,42], ByteOrder::BigEndian,    Value::List(vec![Value::SLong     (42)                ; 2])),
        ([1,0,0,0,0,0,0,0, 1,1, 11,0, 2,0,0,0,0,0,0,0, 42, 0, 0, 0,42, 0, 0, 0], ByteOrder::LittleEndian, Value::List(vec![Value::Float     (f32::from_bits(42)); 2])),
        ([0,0,0,0,0,0,0,1, 1,1, 0,11, 0,0,0,0,0,0,0,2,  0, 0, 0,42, 0, 0, 0,42], ByteOrder::BigEndian,    Value::List(vec![Value::Float     (f32::from_bits(42)); 2])),
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
        ([1,0, 1,1, 9, 0, 2,0,0,0, 42, 0, 0, 0], ByteOrder::LittleEndian, 2, TagType::SLONG     ),
        ([0,1, 1,1, 0, 9, 0,0,0,2,  0, 0, 0,42], ByteOrder::BigEndian   , 2, TagType::SLONG     ),
        ([1,0, 1,1, 11,0, 2,0,0,0, 42, 0, 0, 0], ByteOrder::LittleEndian, 2, TagType::FLOAT     ),
        ([0,1, 1,1, 0,11, 0,0,0,2,  0, 0, 0,42], ByteOrder::BigEndian   , 2, TagType::FLOAT     ),
        ([1,0, 1,1, 12,0, 1,0,0,0, 42, 0, 0, 0], ByteOrder::LittleEndian, 1, TagType::DOUBLE    ),
        ([0,1, 1,1, 0,12, 0,0,0,1,  0, 0, 0,42], ByteOrder::BigEndian   , 1, TagType::DOUBLE    ),
        ([1,0, 1,1, 5, 0, 1,0,0,0, 42, 0, 0, 0], ByteOrder::LittleEndian, 1, TagType::RATIONAL  ),
        ([0,1, 1,1, 0, 5, 0,0,0,1,  0, 0, 0,42], ByteOrder::BigEndian   , 1, TagType::RATIONAL  ),
        ([1,0, 1,1, 10,0, 1,0,0,0, 42, 0, 0, 0], ByteOrder::LittleEndian, 1, TagType::SRATIONAL  ),
        ([0,1, 1,1, 0,10, 0,0,0,1,  0, 0, 0,42], ByteOrder::BigEndian   , 1, TagType::SRATIONAL  ),
        // Double doesn't fit, neither 8-types and we special-case IFD
        ];
        for (buf, byte_order, count, tag_type) in cases {
            println!("Trying {buf:?}, with {byte_order:?}");
            let mut dir = Directory::new();
            dir.insert(Tag::from_u16_exhaustive(0x01_01), IfdEntry::Offset { tag_type, count, offset: 42 });
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
        ([1,0,0,0,0,0,0,0, 1,1, 11,0, 3,0,0,0,0,0,0,0, 42, 0, 0, 0, 0, 0, 0, 0], ByteOrder::LittleEndian, 3, TagType::FLOAT     ),
        ([0,0,0,0,0,0,0,1, 1,1, 0,11, 0,0,0,0,0,0,0,3,  0, 0, 0, 0, 0, 0, 0,42], ByteOrder::BigEndian   , 3, TagType::FLOAT     ),
        ([1,0,0,0,0,0,0,0, 1,1, 12,0, 2,0,0,0,0,0,0,0, 42, 0, 0, 0, 0, 0, 0, 0], ByteOrder::LittleEndian, 2, TagType::DOUBLE    ),
        ([0,0,0,0,0,0,0,1, 1,1, 0,12, 0,0,0,0,0,0,0,2,  0, 0, 0, 0, 0, 0, 0,42], ByteOrder::BigEndian   , 2, TagType::DOUBLE    ),
        ([1,0,0,0,0,0,0,0, 1,1, 5, 0, 2,0,0,0,0,0,0,0, 42, 0, 0, 0, 0, 0, 0, 0], ByteOrder::LittleEndian, 2, TagType::RATIONAL  ),
        ([0,0,0,0,0,0,0,1, 1,1, 0, 5, 0,0,0,0,0,0,0,2,  0, 0, 0, 0, 0, 0, 0,42], ByteOrder::BigEndian   , 2, TagType::RATIONAL  ),
        ([1,0,0,0,0,0,0,0, 1,1, 10,0, 2,0,0,0,0,0,0,0, 42, 0, 0, 0, 0, 0, 0, 0], ByteOrder::LittleEndian, 2, TagType::SRATIONAL ),
        ([0,0,0,0,0,0,0,1, 1,1, 0,10, 0,0,0,0,0,0,0,2,  0, 0, 0, 0, 0, 0, 0,42], ByteOrder::BigEndian   , 2, TagType::SRATIONAL ),
        // we special-case IFD
        ];
        for (buf, byte_order, count, tag_type) in cases {
            println!("         tag   type       count                 offset");
            println!("       |1 2 |1  2 |1  2  3  4  5  6  7  8 |1  2  3  4  5  6  7  8|");
            println!("Trying {buf:?}, with {byte_order:?}");
            let mut dir = Directory::new();
            dir.insert(Tag::from_u16_exhaustive(0x01_01), IfdEntry::Offset { tag_type, count, offset: 42 });
            assert_eq!(Ifd::from_buffer(&buf, byte_order, true).unwrap(), Ifd{
                sub_ifds: Vec::new(),
                data: dir
            });
        }
    }
}
