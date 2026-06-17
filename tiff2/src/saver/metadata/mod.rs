use std::ops::Range;
use std::sync::Arc;

use exn::{ensure, ResultExt};
use smallvec::smallvec;

use crate::structs::error::BUF_CHECK;
use crate::structs::tiff::header_size;
use crate::structs::{TagData, Tiff};
use crate::ByteOrder;

pub mod error;
use error::TiffSaveError;
mod extension;
pub use extension::TiffExtSaverRegistry;
mod ifd;
pub use ifd::IfdSaver;
mod cog_planner;

pub type TiffSaveResult<T> = exn::Result<T, error::TiffSaveError>;

#[derive(Debug, Clone, PartialEq)]
pub enum TiffSaveResponse {
    /// The tiff does not contains any IFDs, so it can't write the header yet
    NeedIfd,
    /// Header of n bytes written
    Done(usize),
}

impl TiffSaveResponse {
    fn unwrap(self) -> usize {
        match self {
            Self::Done(v) => v,
            other => panic!("Called Unwrap on a not-Done TiffSaveResponse {other:?}"),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum IfdSaveResponse {
    NeedBuffer(Range<u64>),
    Partial {
        ifd_saver: IfdSaver,
        needed_buffers: Vec<usize>,
    },
    Complete,
}

/// Trait for all Saver types to implement
///
/// This is a lower-lever API where special planners can be implemented (such as COG).
/// For extension tags or adding metadata, creating an [`ExtensionSaver`] may be the better option.
pub trait TiffSaver: Sized + Send + Sync {
    /// Write the header of the tiff file to the provided buffer
    ///
    /// It will point the ifd to the first ifd in the tiff
    fn save_header(&mut self, buf: &mut [u8]) -> TiffSaveResult<TiffSaveResponse>;

    /// Get a mutable reference to the underlying tiff
    fn tiff_mut(&mut self) -> &mut Tiff;

    /// Get an immutable reference to the underlying tiff
    fn tiff(&self) -> &Tiff;

    /// Get the ifd saver
    // shenanigans wrt wtf this actually means...I mean: we can have a saver
    // without having saved any tiles right? the loader gets it from an offset,
    // so maybe we want to get it from an offset here as well? The tiff should
    // know ifd offsets right? I sure think so? or since it implements
    // TiffSaver, it should be this -at-least-capable-of-if-inefficient-at- TiffSaver
    // But that would also mean...
    // idk what that means, something with it being nice that Ifd can also store offset values
    // and that's I guess the right place for a KISS implementation
    //
    // So the thing is: Work on Tiff until the locations of ifds is known.
    // Then create IfdSavers and write them. It may also be needed to change the ifd location
    // I guess at that point... Somethingsomething planner something
    //
    // or not? like howto ghost area and such?
    // actually, it's perfectly fine to re-create IfdSavers every now and then
    //
    /// Takes a `&mut self` to allow planners to change state. This should not
    /// change the tiff. In other words: Creating an IfdSaver does not save
    /// anything yet or remove the Ifd.
    fn ifd_saver(
        &mut self,
        ifd_offset: u64,
        extension_registry: Arc<TiffExtSaverRegistry>,
    ) -> TiffSaveResult<IfdSaveResponse>;

    /// TODO: I don't know what this will do yet, it's mainly there for symmetry with the loading side now
    fn resume_saver(
        &mut self,
        ranges: Vec<Range<u64>>,
        buffers: Vec<&mut [u8]>,
        saver: IfdSaver,
    ) -> TiffSaveResult<IfdSaveResponse>;
}

impl TiffSaver for Tiff {
    fn tiff(&self) -> &Tiff {
        self
    }
    fn tiff_mut(&mut self) -> &mut Tiff {
        self
    }
    fn save_header(&mut self, buf: &mut [u8]) -> TiffSaveResult<TiffSaveResponse> {
        // Please pass in correct buffers to this function
        // You should kind of be knowing what you're doing?
        ensure!(
            buf.len() >= header_size(self.bigtiff) as usize,
            TiffSaveError::invalid_buffer(
                header_size(self.bigtiff),
                "Could not write header".to_string()
            )
        );
        // That we do not have an IFD is somewhat expected if we are building an
        let Some(first_ifd_offset) = self.ifd_offsets.first() else {
            return Ok(TiffSaveResponse::NeedIfd);
        };
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
        .to_buffer(&mut buf[offset..], self.byte_order)
        .expect(BUF_CHECK);
        offset += if self.bigtiff {
            TagData::Long8(smallvec![*first_ifd_offset])
        } else {
            TagData::Long(smallvec![u32::try_from(*first_ifd_offset).or_raise(
                || {
                    TiffSaveError::need_bigtiff(format!(
                        "first ifd offset {first_ifd_offset} overflows u32"
                    ))
                }
            )?])
        }
        .to_buffer(&mut buf[offset..], self.byte_order)
        .expect(BUF_CHECK);
        Ok(TiffSaveResponse::Done(offset))
    }
    fn ifd_saver(
        &mut self,
        ifd_offset: u64,
        _extension_registry: Arc<TiffExtSaverRegistry>,
    ) -> TiffSaveResult<IfdSaveResponse> {
        eprintln!("extension saving WIP, not working");
        Ok(IfdSaver::from_ifd(
            self.ifds[&ifd_offset].clone(),
            ifd_offset,
            self.bigtiff,
            self.byte_order,
        )
        .to_response())
    }
    /// Write the saver's data to the provided buffers
    ///
    /// This assumes a one-to-one correspondence between buffers and ranges
    /// e.g.
    /// ```
    /// # let saver;
    /// let ifd_saver = saver.ifd_saver(42, Arc::new(Vec::new().into()))
    /// let bufs = ifd_saver.to_save().map(|(_tag, byte_len)| vec![0;byte_len]);
    ///
    /// ```
    fn resume_saver(
        &mut self,
        ranges: Vec<Range<u64>>,
        mut buffers: Vec<&mut [u8]>,
        mut saver: IfdSaver,
    ) -> TiffSaveResult<IfdSaveResponse> {
        let to_do = saver.to_save().enumerate().collect::<Vec<_>>();
        ensure!(
            to_do.len() == buffers.len() && to_do.len() == ranges.len(),
            TiffSaveError::permanent(
                "invalid buffers encountered for writing from Tiff".to_string()
            )
        );
        // directly try to write ifd to_write tags to the provided buffers
        for (idx, (tag, byte_length)) in to_do {
            ensure!(
                byte_length < buffers[idx].len(),
                TiffSaveError::permanent("TODO".to_string())
            );
            saver
                .save_tag_data(buffers[idx], tag, ranges[idx].start)
                .expect(BUF_CHECK);
        }
        Ok(saver.to_response())
    }
}

#[cfg(test)]
mod test {
    use std::collections::{BTreeMap, HashMap};
    use std::default::Default;
    use std::sync::Arc;

    use bytes::Bytes;

    use super::*;
    use crate::loader::metadata::IfdLoadResponse;
    use crate::loader::TiffLoader;
    use crate::structs::metadata::tags::{CompressionMethod, PhotometricInterpretation};
    use crate::structs::tiff::header_size;
    use crate::structs::{entry_size, num_entries_size, offset_size, Ifd, Tag};

    #[test]
    fn test_save_header_small_le() {
        let mut tiff = Tiff {
            ifds: BTreeMap::new(),
            ifd_offsets: vec![header_size(false)],
            extensions: HashMap::new(),
            bigtiff: false,
            byte_order: ByteOrder::LittleEndian,
        };
        let mut buf = vec![0; header_size(true) as usize];
        tiff.save_header(&mut buf).unwrap();
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
        let mut tiff = Tiff {
            ifds: BTreeMap::new(),
            ifd_offsets: vec![header_size(false)],
            extensions: HashMap::new(),
            bigtiff: false,
            byte_order: ByteOrder::BigEndian,
        };
        let mut buf = vec![0; header_size(true) as usize];
        tiff.save_header(&mut buf).unwrap();
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
        let mut tiff = Tiff {
            ifds: BTreeMap::new(),
            ifd_offsets: vec![header_size(true)],
            extensions: HashMap::new(),
            bigtiff: true,
            byte_order: ByteOrder::LittleEndian,
        };
        let mut buf = vec![0; header_size(true) as usize];
        tiff.save_header(&mut buf).unwrap();
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
        let mut tiff = Tiff {
            ifds: BTreeMap::new(),
            ifd_offsets: vec![header_size(true)],
            extensions: HashMap::new(),
            bigtiff: true,
            byte_order: ByteOrder::BigEndian,
        };
        let mut buf = vec![0; header_size(true) as usize];
        tiff.save_header(&mut buf).unwrap();
        assert_eq!(&buf[..], &[
            b'M',b'M',
            0,43,
            0,8,
            0,0,
            0,0,0,0,0,0,0,header_size(true) as u8
        ]);
        assert_eq!(Tiff::from_header(Bytes::from_owner(buf), ).unwrap().unwrap(), tiff);
    }

    #[test]
    fn test_save_tiff_roundtrip_single_tile_hack() {
        // this is a round-trip test, mainly to get a feel for the more low-level apis
        // the hack is that in stead of having multiple tiles, all offsets refer to the same tile
        // Also tile ranges are incorrect for edge tiles
        let tile_data = [42u8; 8 * 8];
        let mut tiff = Tiff {
            ifds: BTreeMap::from([(
                header_size(false),
                Ifd {
                    tags: BTreeMap::from([
                        (Tag::ImageWidth, TagData::Short(smallvec![42])),
                        (Tag::ImageLength, TagData::Short(smallvec![42])),
                        (
                            Tag::PhotometricInterpretation,
                            TagData::Short(smallvec![
                                PhotometricInterpretation::BlackIsZero.to_u16()
                            ]),
                        ),
                        (Tag::SamplesPerPixel, TagData::Short(smallvec![1])),
                        (
                            Tag::Compression,
                            TagData::Short(smallvec![CompressionMethod::None.to_u16()]),
                        ),
                        (Tag::TileWidth, TagData::Short(smallvec![8])),
                        (Tag::TileLength, TagData::SByte(smallvec![8])),
                        (Tag::TileByteCounts, TagData::Short(smallvec![8*8;6*6])), // So at this point everything breaks down a bit...
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
                            TagData::Long(
                                smallvec![u32::try_from(header_size(false) + num_entries_size(false) + entry_size(false) * 9 + offset_size(false) + 6*6*(2+4)).unwrap();6*6],
                            ),
                        ),
                    ]),
                    ..Default::default()
                },
            )]),
            ifd_offsets: vec![header_size(false), 0],
            extensions: HashMap::new(),
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
        offset += tiff.save_header(&mut out).unwrap().unwrap();
        println!("{out:?}");
        let IfdSaveResponse::Partial {
            mut ifd_saver,
            needed_buffers,
        } = tiff
            .ifd_saver(header_size(false), Arc::new(Vec::new().into()))
            .unwrap()
        else {
            panic!()
        };
        // hmm, the kind of logical thing to do now.... ah well, I think it'll work like that
        // I already inserted TileByteCounts, even though maybe that was only possible because we don't compress anything...
        // so well, imagine in future this will also have a tile_bytecounts somewhere
        //
        // So now we just do the really ugly thing of making all tiles point to the same place in the file...
        offset += ifd_saver.required_len() as usize;
        for (tag, tag_data) in ifd_saver.to_save().collect::<Vec<_>>() {
            println!("{tag_data:?}");
            offset += ifd_saver
                .save_tag_data(&mut out[offset..], tag, u64::try_from(offset).unwrap())
                .unwrap();
        }
        ifd_saver
            .write(&mut out[header_size(false) as usize..], 0)
            .unwrap();
        out[offset as usize..].copy_from_slice(&tile_data);
        // assert_eq!(&out, &[]);
        let f = Bytes::from_owner(out);
        let mut read = Tiff::from_header(f.clone()).unwrap().unwrap();
        let IfdLoadResponse::Partial {
            ifd_loader: mut read_ifd,
            needed_data,
        } = read
            .ifd_loader(
                f.slice(read.next_ifd_offset().unwrap() as usize..).clone(),
                read.next_ifd_offset().unwrap(),
                Arc::new(Vec::new().into()),
            )
            .unwrap()
        else {
            panic!("bare tiff should return partial for ifdloadresponse with deferred tags")
        };
        assert!(needed_data.is_empty());
        for (t, r) in read_ifd.to_load().collect::<Vec<_>>() {
            read_ifd
                .load_tag_data(&f[r.start as _..r.end as _], t)
                .unwrap();
        }
        read.insert_ifd(
            read.next_ifd_offset().unwrap(),
            read_ifd.next_ifd_offset.unwrap(),
            read_ifd.finish(),
        );
        assert_eq!(read, tiff);
        let tbyte_counts = &read.ifd(0).tags[&Tag::TileByteCounts];
        let toffsets = &read.ifd(0).tags[&Tag::TileOffsets];
        for tile_idx in 0..6 * 6 {
            let l = <&[u16]>::try_from(tbyte_counts).unwrap()[tile_idx];
            let r = <&[u32]>::try_from(toffsets).unwrap()[tile_idx];
            println!("{tile_idx}->{r}..{l},");
            assert_eq!(&f[r as _..r as usize + l as usize], &tile_data)
        }
    }
}
