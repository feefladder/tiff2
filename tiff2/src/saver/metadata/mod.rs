use exn::ResultExt;
use smallvec::smallvec;

use crate::saver::metadata::error::SaverError;
use crate::saver::metadata::ifd::IfdSaver;
use crate::structs::{TagData, Tiff};
use crate::ByteOrder;

pub mod error;
mod ifd;

pub type SaverResult<T> = exn::Result<T, error::SaverError>;

impl Tiff {
    fn write_header(&self, buf: &mut [u8], first_ifd_offset: u64) -> SaverResult<usize> {
        // TODO: check buffer length
        buf[0..2].copy_from_slice(match self.byte_order {
            ByteOrder::LittleEndian => b"II",
            ByteOrder::BigEndian => b"MM",
        });
        let mut offset = 2;
        offset += TagData::Short(if self.bigtiff {
            smallvec![43, 8, 0]
        } else {
            smallvec![42]
        })
        .to_buffer(&mut buf[offset..], self.byte_order);
        offset += if self.bigtiff {
            TagData::Long8(smallvec![first_ifd_offset])
        } else {
            TagData::Long(smallvec![u32::try_from(first_ifd_offset).or_raise(
                || {
                    SaverError::need_bigtiff(format!(
                        "first ifd offset {first_ifd_offset} overflows u32"
                    ))
                }
            )?])
        }
        .to_buffer(&mut buf[offset..], self.byte_order);
        Ok(offset)
    }

    fn ifd_saver(&self, ifd_offset: u64) -> IfdSaver {
        IfdSaver::from_ifd(
            self.ifds[&ifd_offset].clone(),
            ifd_offset,
            self.bigtiff,
            self.byte_order,
        )
    }
}

#[cfg(test)]
mod test {
    use std::collections::BTreeMap;

    use bytes::Bytes;

    use super::*;
    use crate::loader::metadata::IfdLoadResponse;
    use crate::loader::TiffLoader;
    use crate::structs::metadata::tags::{CompressionMethod, PhotometricInterpretation};
    use crate::structs::tiff::header_size;
    use crate::structs::{entry_size, num_entries_size, offset_size, Ifd, IfdEntry, Offset, Tag};

    #[test]
    fn test_save_header_small_le() {
        let tiff = Tiff {
            ifds: BTreeMap::new(),
            ifd_offsets: vec![header_size(false)],
            bigtiff: false,
            byte_order: ByteOrder::LittleEndian,
        };
        let mut buf = vec![0; header_size(true) as usize];
        tiff.write_header(&mut buf, header_size(false)).unwrap();
        assert_eq!(
            &buf[..header_size(false) as usize],
            [b'I', b'I', 42, 0, header_size(false) as u8, 0, 0, 0,]
        );
        assert_eq!(
            Tiff::from_header(Bytes::from_owner(buf)).unwrap().unwrap(),
            tiff
        );
    }

    #[test]
    fn test_save_header_small_be() {
        let tiff = Tiff {
            ifds: BTreeMap::new(),
            ifd_offsets: vec![header_size(false)],
            bigtiff: false,
            byte_order: ByteOrder::BigEndian,
        };
        let mut buf = vec![0; header_size(true) as usize];
        tiff.write_header(&mut buf, header_size(false)).unwrap();
        assert_eq!(
            &buf[..header_size(false) as usize],
            &[b'M', b'M', 0, 42, 0, 0, 0, header_size(false) as u8]
        );
        assert_eq!(
            Tiff::from_header(Bytes::from_owner(buf)).unwrap().unwrap(),
            tiff
        );
    }

    #[test]
    #[rustfmt::skip]
    fn test_save_header_big_le() {
        let tiff = Tiff {
            ifds: BTreeMap::new(),
            ifd_offsets: vec![header_size(true)],
            bigtiff: true,
            byte_order: ByteOrder::LittleEndian,
        };
        let mut buf = vec![0; header_size(true) as usize];
        tiff.write_header(&mut buf, header_size(true)).unwrap();
        assert_eq!(
            &buf[..],
            &[
                b'I',b'I',
                43,0,
                8,0,
                0,0,
                header_size(true) as u8,0,0,0,0,0,0,0,
            ]
        );
        assert_eq!(Tiff::from_header(Bytes::from_owner(buf)).unwrap().unwrap(), tiff);
    }

    #[test]
    #[rustfmt::skip]
    fn test_save_header_big_be() {
        let tiff = Tiff {
            ifds: BTreeMap::new(),
            ifd_offsets: vec![header_size(true)],
            bigtiff: true,
            byte_order: ByteOrder::BigEndian,
        };
        let mut buf = vec![0; header_size(true) as usize];
        tiff.write_header(&mut buf, header_size(true)).unwrap();
        assert_eq!(&buf[..], &[
            b'M',b'M',
            0,43,
            0,8,
            0,0,
            0,0,0,0,0,0,0,header_size(true) as u8
        ]);
        assert_eq!(Tiff::from_header(Bytes::from_owner(buf)).unwrap().unwrap(), tiff);
    }

    #[test]
    fn test_save_tiff_roundtrip_single_tile_hack() {
        // this is a round-trip test, mainly to get a feel for the more low-level apis
        // the hack is that in stead of having multiple tiles, all offsets refer to the same tile
        // Also tile ranges are incorrect for edge tiles
        let tile_data = [42u8; 8 * 8];
        let tiff = Tiff {
            ifds: BTreeMap::from([(
                header_size(false),
                Ifd {
                    data: BTreeMap::from([
                        (
                            Tag::ImageWidth,
                            IfdEntry::Value(TagData::Short(smallvec![42])),
                        ),
                        (
                            Tag::ImageLength,
                            IfdEntry::Value(TagData::Short(smallvec![42])),
                        ),
                        (
                            Tag::PhotometricInterpretation,
                            IfdEntry::Value(TagData::Short(smallvec![
                                PhotometricInterpretation::BlackIsZero.to_u16()
                            ])),
                        ),
                        (
                            Tag::SamplesPerPixel,
                            IfdEntry::Value(TagData::Short(smallvec![1])),
                        ),
                        (
                            Tag::Compression,
                            IfdEntry::Value(TagData::Short(smallvec![
                                CompressionMethod::None.to_u16()
                            ])),
                        ),
                        (
                            Tag::TileWidth,
                            IfdEntry::Value(TagData::Short(smallvec![8])),
                        ),
                        (
                            Tag::TileLength,
                            IfdEntry::Value(TagData::SByte(smallvec![8])),
                        ),
                        (
                            Tag::TileByteCounts,
                            IfdEntry::Value(TagData::Short(smallvec![8*8;6*6])),
                        ), // So at this point everything breaks down a bit...
                        // Ideally, there'd be some sort of TiffBuilder struct or something
                        // Anyways, this here is not really the way to do stuff?
                        // I mean, what should TileOffsets be? At this point we don't really know that...
                        // and like...
                        // TiffSaver::new(width,height).tiled(tile_size)
                        // should actually be the right way to make something.
                        // I mean, still it should of course also be possible to create a saver from a Tiff
                        (
                            Tag::TileOffsets,
                            // all tiles point to the same in-file location
                            IfdEntry::Value(TagData::Long(
                                smallvec![u32::try_from(header_size(false) + num_entries_size(false) + entry_size(false) * 9 + offset_size(false) + 6*6*(2+4)).unwrap();6*6],
                            )),
                        ),
                    ]),
                },
            )]),
            ifd_offsets: vec![header_size(false), 0],
            bigtiff: false,
            byte_order: ByteOrder::LittleEndian,
        };
        let mut out = vec![
            0;
            header_size(false) as usize
                + num_entries_size(false) as usize
                + entry_size(false) as usize * 9
                + offset_size(false) as usize
                + 6 * 6 * (2 + 4)
                + 8 * 8
        ];
        let mut offset = 0;
        offset += tiff.write_header(&mut out, header_size(false)).unwrap();
        println!("{out:?}");
        let mut saver = tiff.ifd_saver(header_size(false));
        // hmm, the kind of logical thing to do now.... ah well, I think it'll work like that
        // I already inserted TileByteCounts, even though maybe that was only possible because we don't compress anything...
        // so well, imagine in future this will also have a tile_bytecounts somewhere
        //
        // So now we just do the really ugly thing of making all tiles point to the same place in the file...
        offset += saver.required_len() as usize;
        for (tag, tag_data) in saver.to_do_mut() {
            println!("{tag_data:?}");
            let o;
            {
                let IfdEntry::Value(v) = tag_data else {
                    unreachable!()
                };
                o = Offset {
                    tag_type: v.tag_type(),
                    count: v.len() as u64,
                    offset: offset as u64,
                };
                offset += v.to_buffer(&mut out[offset..], tiff.byte_order);
            }
            println!("{o:?}:{out:?}");
            *tag_data = IfdEntry::Offset(o);
        }
        saver
            .write(&mut out[header_size(false) as usize..], 0)
            .unwrap();
        out[offset as usize..].copy_from_slice(&tile_data);
        // assert_eq!(&out, &[]);
        let f = Bytes::from_owner(out);
        let mut read = Tiff::from_header(f.clone()).unwrap().unwrap();
        let IfdLoadResponse::Partial {
            ifd_loader: mut read_ifd,
            next_ifd_offset: zero,
            needed_data,
        } = read
            .ifd_loader(
                f.slice(read.next_ifd_offset().unwrap() as usize..).clone(),
                read.next_ifd_offset().unwrap(),
            )
            .unwrap()
        else {
            panic!("bare tiff should return partial for ifdloadresponse with deferred tags")
        };
        assert_eq!(zero, 0);
        assert!(needed_data.is_empty());
        for (_t, o) in read_ifd.deferred_values_mut() {
            let v;
            {
                let IfdEntry::Offset(o) = o else {
                    unreachable!()
                };
                let r = o.range();
                v = TagData::from_buffer(
                    &f[r.start as _..r.end as _],
                    o.tag_type,
                    o.count as _,
                    read.byte_order,
                )
                .unwrap();
            }
            *o = IfdEntry::Value(v);
        }
        read.insert_ifd(read.next_ifd_offset().unwrap(), read_ifd.finish(), 0);
        assert_eq!(read, tiff);
        let IfdEntry::Value(tbyte_counts) = &read.ifd(0).data[&Tag::TileByteCounts] else {
            unreachable!()
        };
        let IfdEntry::Value(toffsets) = &read.ifd(0).data[&Tag::TileOffsets] else {
            unreachable!()
        };
        for tile_idx in 0..6 * 6 {
            let l = <&[u16]>::try_from(tbyte_counts).unwrap()[tile_idx];
            let r = <&[u32]>::try_from(toffsets).unwrap()[tile_idx];
            println!("{tile_idx}->{r}..{l},");
            assert_eq!(&f[r as _..r as usize + l as usize], &tile_data)
        }
    }
}
