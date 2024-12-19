use std::{collections::BTreeMap, fmt::Debug, ops::Range, sync::Arc};

use async_trait::async_trait;
use bytes::{Buf, Bytes};
use log::{debug, error};

use crate::{
    decoder::ImageDecoder,
    error::{TiffError, TiffFormatError, TiffResult, UsageError},
    structs::{Ifd, Image},
    ByteOrder,
};

use super::{CogReader, EndianReader, Limits};

const HEADER_SIZE_SMALLTIFF: usize = 6;
const HEADER_SIZE_BIGTIFF: usize = 16;
const ENTRY_SIZE_SMALLTIFF: u64 = 12;
const ENTRY_SIZE_BIGTIFF: u64 = 20;
const NUM_ENTRIES_SIZE_SMALLTIFF: u64 = 2;
const NUM_ENTRIES_SIZE_BIGTIFF: u64 = 8;

/// cache for reading ifds, since their size is not known from the header. Also
/// allows for creating a reader at a specific location
#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
pub trait IfdCache: Send {
    /// check if a given range is contained in the cache and give it if it exists
    fn try_get_range(&self, range: &Range<u64>) -> Option<Bytes>; // or Result
    /// add the range to the cache and return it
    /// SHOULD return an error if
    async fn fetch_range<R: CogReader>(
        &mut self,
        range: &Range<u64>,
        reader: &R,
    ) -> TiffResult<Bytes>;
    /// get the desired range, fetch if needed
    #[inline]
    async fn get_range<R: CogReader>(
        &mut self,
        range: Range<u64>,
        reader: &R,
    ) -> TiffResult<Bytes> {
        if let Some(res) = self.try_get_range(&range) {
            Ok(res)
        } else {
            self.fetch_range(&range, reader).await
        }
    }
}

#[derive(Default, PartialEq)]
pub struct IfdBuffer {
    /// buffer holding data that can be read synchronously
    /// if implementing your own decoder, this is the place to add smart things,
    /// such as a rangemap. However, for now I think this is simple and
    /// efficient enough.
    buffer: Bytes,
    /// start location of the buffer
    buf_start: u64,
}

impl Debug for IfdBuffer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IfdBuffer")
            .field("buf_start", &self.buf_start)
            .field(
                "buffer",
                &&self.buffer[..if self.buffer.len() < 32 {
                    self.buffer.len()
                } else {
                    32
                }],
            )
            .finish()
    }
}

#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
impl IfdCache for IfdBuffer {
    fn try_get_range(&self, range: &Range<u64>) -> Option<Bytes> {
        // Ensure the range is within the buffer
        if self.buf_start > range.start || range.end > self.buf_start + self.buffer.len() as u64 {
            return None; // Range is not in the buffer
        }

        // Calculate the start and end positions within the buffer
        let offset_in_buf = (range.start - self.buf_start) as usize;
        let len_in_buf = (range.end - range.start) as usize;

        // Return the slice of the buffer corresponding to the range
        Some(self.buffer.slice(offset_in_buf..offset_in_buf + len_in_buf))
    }

    async fn fetch_range<R: CogReader>(
        &mut self,
        range: &Range<u64>,
        reader: &R,
    ) -> TiffResult<Bytes> {
        // Fetch and store the new range in the buffer
        self.buf_start = range.start;
        self.buffer = reader.get_ranges(&[range.clone()]).await?[0].clone();
        Ok(self.buffer.clone())
    }
}

// use object_store::ObjectStore;
/// Async decoder
///
#[derive(Debug, PartialEq)]
#[non_exhaustive]
pub struct Decoder<R: CogReader, C: IfdCache = IfdBuffer> {
    /// Reader, implements CogReader
    pub reader: R,
    /// cache for only the IFDs, not their data
    ifd_cache: C,
    /// byte_order of the tiff file
    byte_order: ByteOrder,
    /// whether we are bigtiff.
    ///
    /// Influences layout of IFDs
    bigtiff: bool,
    /// Memory limits
    ///
    /// not currently used
    limits: Limits,
    /// ifd offsets. Points to the nr_of_ifds field/tag
    ifd_offsets: Vec<u64>,
    /// Images, sorted by offset location
    pub images: BTreeMap<u64, Image>,
    /// not-loaded image IFDs or IFDs that are not images.
    pub meta_ifds: BTreeMap<u64, Ifd>,
}

impl<R: CogReader + Sync> Decoder<R, IfdBuffer> {
    pub async fn new(reader: R) -> TiffResult<Decoder<R, IfdBuffer>> {
        Decoder::new_generic(reader, IfdBuffer::default()).await
    }
}

impl<R: CogReader + Sync, C: IfdCache> Decoder<R, C> {
    pub fn ifd_offsets(&self) -> &[u64] {
        &self.ifd_offsets
    }
    /// Create a new decoder from the source
    /// Will read an initial IFD chunk at offset zero
    pub async fn new_generic(reader: R, mut ifd_cache: C) -> TiffResult<Self> {
        // let buf = ;
        // let buffer = ifd_cache.get_range(0..R::IFD_REQ_SIZE, &reader).await?;
        debug!(
            "reading file header: {:?}",
            &ifd_cache.get_range(0..16, &reader).await?[..]
        );
        let byte_order =
            match <&[u8; 2]>::try_from(&ifd_cache.get_range(0..2, &reader).await?[..]).unwrap() {
                b"II" => ByteOrder::LittleEndian,
                b"MM" => ByteOrder::BigEndian,
                _ => {
                    return Err(TiffFormatError::TiffSignatureNotFound.into());
                }
            };
        debug!("byte order: {byte_order:?}");
        let buf = ifd_cache.get_range(2..32, &reader).await?;
        debug!("buf: {:?}", &buf[..]);
        let mut r = EndianReader::wrap(std::io::Cursor::new(buf), byte_order);
        let bigtiff = match r.read_u16()? {
            42 => false,
            43 => {
                if r.read_u16()? != 8 {
                    return Err(TiffFormatError::TiffSignatureNotFound.into());
                }
                if r.read_u16()? != 0 {
                    return Err(TiffFormatError::TiffSignatureNotFound.into());
                }
                true
            }
            v => {
                error!(
                    "Tiff signature {v:?} invalid: {:?}",
                    &ifd_cache.get_range(2..32, &reader).await?[..]
                );
                return Err(TiffFormatError::TiffSignatureInvalid.into());
            }
        };
        let ifd_offsets = vec![if bigtiff {
            r.read_u64()?
        } else {
            u64::from(r.read_u32()?)
        }];
        Ok(Decoder {
            ifd_cache,
            reader,
            byte_order,
            bigtiff,
            limits: Default::default(),
            ifd_offsets,
            images: BTreeMap::new(),
            meta_ifds: BTreeMap::new(),
        })
    }

    /// get the range of the buffer
    ///
    /// # panics
    ///
    /// if buffer.len doesn't fit in u64
    // pub fn buf_range(&self) -> Range<u64> {
    //     self.buf_start..self.buf_start + u64::try_from(self.buffer.len()).unwrap()
    // }

    /// checks if the ifd at offset is contained in the buffer by reading the
    /// number of tags from the first few bytes.
    ///
    /// # panics
    ///
    /// if `offset - self.buf_start` doesn't fit in `usize`
    // #[inline]
    // fn ifd_is_in_buf(&self, offset: u64) -> bool {
    //     if offset < self.buf_start
    //         || offset + 8 > self.buf_start + u64::try_from(self.buffer.len()).unwrap()
    //     {
    //         return false;
    //     }

    //     let n_entries = if self.bigtiff {
    //         self.byte_order.u64(
    //             self.buffer[offset_in_buf..offset_in_buf + 8] // size of n_entries field
    //                 .try_into()
    //                 .unwrap(),
    //         )
    //     } else {
    //         u64::from(
    //             self.byte_order.u16(
    //                 self.buffer[offset_in_buf..offset_in_buf + 2]
    //                     .try_into()
    //                     .unwrap(),
    //             ),
    //         )
    //     };
    //     let ifd_len = if self.bigtiff {
    //         offset_in_buf
    //             + usize::try_from(NUM_ENTRIES_SIZE_BIGTIFF + n_entries * ENTRY_SIZE_BIGTIFF)
    //                 .unwrap()
    //     } else {
    //         offset_in_buf
    //             + usize::try_from(NUM_ENTRIES_SIZE_SMALLTIFF + n_entries * ENTRY_SIZE_SMALLTIFF)
    //                 .unwrap()
    //     };
    //     debug!("checking if ifd [{offset}..; {ifd_len}] fits in buffer {:?}", self.buffer.len());
    //     self.buffer.len()
    //         >= ifd_len
    // }

    /// Scan and read IFDs
    ///
    /// Goes through the linked list of IFDs, starting from the last one in ifd_offsets.
    ///  reading them in if they are in the buffer.
    pub async fn scan_ifds(&mut self) -> TiffResult<()> {
        loop {
            // start with the last ifd in the list (assuming that all previous
            // ones have been low)
            let offset = *self
                .ifd_offsets
                .last()
                .ok_or(TiffError::from(TiffFormatError::ImageFileDirectoryNotFound))?;
            // IFD num_entries field is 2 bytes if small and 8 bytes if bigtiff
            let end;
            if self.bigtiff {
                let n_tags_bytes = self
                    .ifd_cache
                    .get_range(offset..offset + NUM_ENTRIES_SIZE_BIGTIFF, &self.reader)
                    .await?;
                let n_tags = self.byte_order.u64(n_tags_bytes[..].try_into().unwrap()); // 8 BYTES
                end = offset + NUM_ENTRIES_SIZE_BIGTIFF + n_tags * ENTRY_SIZE_BIGTIFF + 8;
            } else {
                let n_tags_bytes = self
                    .ifd_cache
                    .get_range(offset..offset + NUM_ENTRIES_SIZE_SMALLTIFF, &self.reader)
                    .await?;
                let n_tags = u64::from(self.byte_order.u16(n_tags_bytes[..].try_into().unwrap())); // 2 BYTES
                end = offset + NUM_ENTRIES_SIZE_SMALLTIFF + n_tags * ENTRY_SIZE_SMALLTIFF + 4;
            }

            let (ifd, pos) = Ifd::from_buffer(
                &self.ifd_cache.get_range(offset..end, &self.reader).await?,
                self.byte_order,
                self.bigtiff,
            )?;
            if Image::check_ifd(&ifd).is_ok_and(|m| m.is_empty()) {
                self.images
                    .insert(offset, Image::from_ifd(ifd, self.byte_order)?);
            } else {
                self.meta_ifds.insert(offset, ifd);
            }
            if pos == 0 {
                break;
            } else if self.ifd_offsets.contains(&pos) {
                return Err(TiffFormatError::CycleInOffsets.into());
            } else {
                self.ifd_offsets.push(pos);
            }
        }
        Ok(())
    }

    pub async fn read_image_ifds(&mut self) -> TiffResult<()> {
        let mut imgs = BTreeMap::new();
        {
            for (offset, ifd) in self.meta_ifds.iter() {
                if let Ok(retr_tags) = Image::check_ifd(&ifd) {
                    imgs.insert(
                        offset.clone(),
                        self.reader.get_tags(retr_tags, self.byte_order),
                    );
                }
            }
        }
        for (offset, img_fut) in imgs {
            // we checked that it was present before
            let mut ifd = self.meta_ifds.remove(&offset).unwrap();
            let tags = img_fut.await?;
            ifd.insert_tag_data(tags)?;
            self.images
                .insert(offset, Image::from_ifd(ifd, self.byte_order)?);
        }
        Ok(())
    }

    pub fn get_overview(&self, index: usize) -> TiffResult<Image> {
        let ifd_offset = self
            .ifd_offsets
            .get(index)
            .ok_or(UsageError::OverviewNotLoaded(index))?;
        self.images
            .get(ifd_offset)
            .ok_or(UsageError::NotAnImage(*ifd_offset).into()) //?
            .map(|im| im.clone())
        //     .map(|im| Ok(ImageDecoder {
        //         image: im.clone(),
        //         reader: &self.reader
        // }))
    }
}

#[cfg(test)]
mod test {

    use super::*;
    use crate::ByteOrder;
    use bytes::Bytes;
    use std::{collections::BTreeMap, thread};

    #[test]
    fn test_rangyness() {
        let a = 0..42;
        let b = 1..42;
        assert_eq!(a.contains(&b.start), a.contains(&(b.end - 1)));
    }

    #[tokio::test]
    async fn test_new_decoder_notbig_littleendian() {
        let data = [
            // TIFF Header (8 bytes)
            b'I', b'I', // Byte order: "II" for little-endian
            42, 0, // Magic number: 42 (0x2A00)
            8, 0, 0, 0, // Offset to first IFD: 8
            // First IFD (12 + 2 + 4 = 18 bytes total)
            1, 0, // Number of directory entries: 1
            // IFD Entry for ImageWidth
            0x00, 0x01, // Tag for ImageWidth: 256 (0x0100)
            4, 0, // Type: LONG (4 bytes per value)
            1, 0, 0, 0, // Count: 1
            42, 0, 0, 0, // Value: 42 (ImageWidth)
            0, 0, 0, 0, // Next IFD offset: 0 (no more IFDs)
        ];
        assert_eq!(
            Decoder::new(&data[..]).await.unwrap(),
            Decoder {
                reader: &data[..],
                ifd_cache: IfdBuffer {
                    buffer: Bytes::copy_from_slice(&data[2..HEADER_SIZE_BIGTIFF]),
                    buf_start: 2,
                },
                byte_order: ByteOrder::LittleEndian,
                bigtiff: false,
                limits: Default::default(),
                ifd_offsets: vec![8],
                images: BTreeMap::new(),
                meta_ifds: BTreeMap::new(),
            }
        );
    }

    #[tokio::test]
    async fn new_decoder_big_littleendian() {
        let data = [
            // BigTIFF Header (16 bytes)
            b'I', b'I', // Byte order: "II" for little-endian
            43, 0, // Magic number for BigTIFF: 43 (0x2B00)
            8, 0, // Offset size (8 bytes) and count size (8 bytes)
            0, 0, // Reserved bytes (2 bytes, set to 0)
            16, 0, 0, 0, 0, 0, 0, 0, // Offset to first IFD (16)
            // First IFD
            1, 0, 0, 0, 0, 0, 0, 0, // Number of directory entries (8 bytes): 1
            // IFD Entry for ImageWidth (20 bytes)
            0x00, 0x01, 0, 0, // Tag for ImageWidth: 256 (0x0100)
            4, 0, 0, 0, // Type: LONG (4 bytes per value)
            1, 0, 0, 0, 0, 0, 0, 0, // Count: 1 (8 bytes)
            42, 0, 0, 0, 0, 0, 0, 0, // Value: 42 (8 bytes for BigTIFF)
            0, 0, 0, 0, 0, 0, 0, 0, // Next IFD offset: 0 (indicating no more IFDs)
        ];
        assert_eq!(
            Decoder::new(&data[..]).await.unwrap(),
            Decoder {
                reader: &data[..],
                ifd_cache: IfdBuffer {
                    buffer: Bytes::copy_from_slice(&data[2..HEADER_SIZE_BIGTIFF]),
                    buf_start: 2,
                },
                byte_order: ByteOrder::LittleEndian,
                bigtiff: true,
                limits: Default::default(),
                ifd_offsets: vec![16],
                images: BTreeMap::new(),
                meta_ifds: BTreeMap::new(),
            }
        )
    }

    #[tokio::test]
    async fn test_new_decoder_notbig_bigendian() {
        let data = [
            // TIFF Header (8 bytes)
            b'M', b'M', // Byte order: "II" for little-endian
            0, 42, // Magic number: 42 (0x2A00)
            0, 0, 0, 8, // Offset to first IFD: 8
            // First IFD (12 + 2 + 4 = 18 bytes total)
            1, 0, // Number of directory entries: 1
            // IFD Entry for ImageWidth
            0x00, 0x01, // Tag for ImageWidth: 256 (0x0100)
            4, 0, // Type: LONG (4 bytes per value)
            1, 0, 0, 0, // Count: 1
            42, 0, 0, 0, // Value: 42 (ImageWidth)
            0, 0, 0, 0, // Next IFD offset: 0 (no more IFDs)
        ];
        assert_eq!(
            Decoder::new(&data[..]).await.unwrap(),
            Decoder {
                reader: &data[..],
                ifd_cache: IfdBuffer {
                    buffer: Bytes::copy_from_slice(&data[2..HEADER_SIZE_BIGTIFF]),
                    buf_start: 2,
                },
                bigtiff: false,
                byte_order: ByteOrder::BigEndian,
                limits: Default::default(),
                ifd_offsets: vec![8],
                images: BTreeMap::new(),
                meta_ifds: BTreeMap::new(),
            }
        );
    }

    #[tokio::test]
    async fn new_decoder_big_bigendian() {
        let data = [
            // BigTIFF Header (16 bytes)
            b'M', b'M', // Byte order: "II" for little-endian
            0, 43, // Magic number for BigTIFF: 43 (0x2B00)
            0, 8, // Offset size (8 bytes) and count size (8 bytes)
            0, 0, // Reserved bytes (2 bytes, set to 0)
            0, 0, 0, 0, 0, 0, 0, 16, // Offset to first IFD (16)
            // First IFD
            1, 0, 0, 0, 0, 0, 0, 0, // Number of directory entries (8 bytes): 1
            // IFD Entry for ImageWidth (20 bytes)
            0x00, 0x01, 0, 0, // Tag for ImageWidth: 256 (0x0100)
            4, 0, 0, 0, // Type: LONG (4 bytes per value)
            1, 0, 0, 0, 0, 0, 0, 0, // Count: 1 (8 bytes)
            42, 0, 0, 0, 0, 0, 0, 0, // Value: 42 (8 bytes for BigTIFF)
            0, 0, 0, 0, 0, 0, 0, 0, // Next IFD offset: 0 (indicating no more IFDs)
        ];
        assert_eq!(
            Decoder::new(&data[..]).await.unwrap(),
            Decoder {
                reader: &data[..],
                ifd_cache: IfdBuffer {
                    buffer: Bytes::copy_from_slice(&data[2..HEADER_SIZE_BIGTIFF]),
                    buf_start: 2,
                },
                byte_order: ByteOrder::BigEndian,
                bigtiff: true,
                limits: Default::default(),
                ifd_offsets: vec![16],
                images: BTreeMap::new(),
                meta_ifds: BTreeMap::new(),
            }
        )
    }
    // use crate::{
    //     error::{TiffResult},
    //     decoder::CogReader,
    //     structs::Image
    // };

    // use std::{collections::HashMap, future::Future, sync::Arc};
    // type OverviewLevel = u8;

    // struct CogDecoder {
    //     /// OverviewLevel->Image map (could be a vec)
    //     images: HashMap<OverviewLevel, Arc<Image>>,
    //     // geo_data: Idk,
    //     reader: Arc<dyn CogReader>,
    // }

    // impl CogDecoder {
    //     /// Retrieve a single chunk
    //     fn get_single_chunk(
    //         &self,
    //         i_chunk: usize,
    //         zoom_level: OverviewLevel,
    //     ) -> TiffResult<impl Future<Output = DecodingResult> /* + Send */> {
    //         match self.images.get(&zoom_level) {
    //             None => panic!(), // in this piece of code, we'd have to await IFD retrieval+decoding
    //             Some(img) => {
    //                 let img_cloned = img.clone();
    //                 let reader = self.reader.clone();
    //                 Ok(async move {
    //                     // don't mention self in this scope
    //                     let n_bytes = &img_cloned.chunk_bytes(i_chunk);
    //                     let out_buf = vec![0u8; n_bytes];
    //                     ChunkDecoder::spawn(
    //                         reader,
    //                         img_cloned.chunk_offset(i_chunk),
    //                         img_cloned.chunk_bytes(i_chunk),
    //                         img.chunk_opts(),
    //                     )
    //                     .await
    //                 })
    //             } // since this returns a future that doesn't reference self, we are happy
    //         }
    //     }

    //     fn from_url() {}
    // }

    // // impl Image {
    // //     // better move this to decoder, only make image return the offset and length
    // //     async fn decode_chunk<R>(
    // //         &self,
    // //         reader: R,
    // //         i_chunk: u64,
    // //     ) -> impl Future<Output = DecodingResult> {
    // //         ChunkDecoder::decode(
    // //             r,
    // //             self.chunk_offsets.get_u64(i_chunk),
    // //             self.chunk_bytes.get_u64(i_chunk.try_into()?)?,
    // //             self.chunk_opts.clone(),
    // //         )
    // //     }
    // // }

    // #[tokio::test]
    // async fn test_concurrency() {
    //     let decoder = CogDecoder::from_url("https://enourmous-cog.com")
    //         .await
    //         .expect("Decoder should build");
    //     decoder
    //         .read_overviews(vec![0, 5])
    //         .await
    //         .expect("Decoder should read ifds");
    //     // get a chunk from the highest resolution image
    //     let chunk_1 = decoder.get_chunk(42, 0);
    //     // get a chunk from a lower resolution image
    //     let chunk_2 = decoder.get_chunk(42, 5);
    //     let data = (chunk_1.await, chunk_2.await);
    // }

    // #[tokio::test]
    // async fn test_concurrency_fail() {
    //     let decoder = CogDecoder::from_url("https://enourmous-cog.com")
    //         .await
    //         .expect("Decoder should build");
    //     decoder
    //         .read_overviews(vec![0])
    //         .await
    //         .expect("decoder should read ifds");
    //     // get a chunk from the highest resolution image
    //     let chunk_1 = decoder.get_chunk(42, 0);
    //     // get a chunk from a lower resolution image
    //     let chunk_2 = decoder.get_chunk(42, 5); //panic!
    //     let data = (chunk_1.await, chunk_2.await);
    // }

    // how HeroicKatana would do it if I understand correctly:
    // #[tokio::test]
    // async fn test_concurrency_recover() {
    //     let decoder = CogDecoder::from_url("https://enourmous-cog.com")
    //         .await
    //         .expect("Decoder should build");
    //     decoder
    //         .read_overviews(vec![0])
    //         .await
    //         .expect("decoder should read ifds");
    //     // get a chunk from the highest resolution image
    //     let chunk_1 = decoder.get_chunk(42, 0).unwrap();
    //     // get a chunk from a lower resolution image
    //     if let OverviewNotLoadedError(chunk_err) = decoder.get_chunk(42, 5).unwrap_err() {
    //         // read_overviews changes state of the decoder to LoadingIfds
    //         decoder.read_overviews(chunk_err).await;
    //     }
    //     let chunk_2 = decoder.get_chunk(42, 5);
    //     let data = (chunk_1.await, chunk_2.await);
    // }

    #[tokio::test]
    #[cfg(feature = "object_store")]
    async fn test_object_store_bytes_mut() {
        use object_store::{path::Path, ObjectStore};
        let obj_store = object_store::http::HttpBuilder::new()
            .with_url("https://isdasoil.s3.amazonaws.com/covariates/dem_30m/dem_30m.tif")
            .build()
            .unwrap();
        let ranges = obj_store
            .get_ranges(&Path::default(), &[0..48, 64..128])
            .await
            .expect("request didn't resolve successfully");
        for range in ranges {
            // this fails
            match range.try_into_mut() {
                Ok(mut range_mut) => {
                    println!("success with {range_mut:?}");
                    range_mut.chunks_exact_mut(8).for_each(|v| {
                        v.copy_from_slice(
                            &u64::from_le_bytes((*v).try_into().unwrap()).to_ne_bytes(),
                        )
                    });
                }
                Err(range) => {
                    eprintln!("Could not get mut on {:?}", range);
                }
            }
            // range_mut.chunks_exact_mut(8).for_each(|v| {
            //     v.copy_from_slice(&u64::from_le_bytes((*v).try_into().unwrap()).to_ne_bytes())
            // })
        }
        let mut range_mut = obj_store
            .get_range(&Path::default(), 256..512)
            .await
            .unwrap()
            .try_into_mut()
            .expect("Could not get single mut");
        range_mut.chunks_exact_mut(8).for_each(|v| {
            v.copy_from_slice(&u64::from_le_bytes((*v).try_into().unwrap()).to_ne_bytes())
        });
        panic!("plz show test {range_mut:?}");
    }

    #[tokio::test]
    /// main idea for concurrently decoding chunks.
    /// Note that we may need to first fetch everything and then decode, since
    /// fetch is io-bound and decode is CPU-bound.
    /// [This article](https://ryhl.io/blog/async-what-is-blocking/) recommends
    /// rayon, but we'll see. Note: Bevy has its own executor
    /// [blog on threads](https://blog.logrocket.com/using-rust-scoped-threads-improve-efficiency-safety/)
    async fn test_split_concurrent() {
        // let input_ranges: Vec<std::ops::Range<u64>> = (0..42)
        //     .into_iter()
        //     // img.chunk_range(i)
        //     .map(|i| i..i + 42)
        //     .collect();
        // let compressed_chunks = obj_store.get_ranges(input_ranges).await.unwrap();

        // vec![0; img.total_data_size()] // or a sub-view/bbox in COGland
        let mut data: Vec<u8> = vec![42; 42]; // 42 bytes
        let slice = &mut data;
        let mut req = slice.clone();
        for (i, c) in req.chunks_exact_mut(CHUNK_SIZE).enumerate() {
            for v in c {
                *v += i as u8;
            }
        }
        // Split the Vec into mut chunks. Will have to roll our own because
        // padding
        // e.g. tiles at the right and bottom will have other sizes, so we can't
        // do chunks_exact_mut. Let's see with split_at_mut (see impl)
        // Actually, we cannot use a whole slice, but need to get a different
        // slice per row, like:
        //    1     2
        // +-----+-----+ <- 0 [u8; 4]
        // |     |-----| <- 1 [u8; 4]
        // |     |-----| <- 2 [u8; 4]
        // +-----+-----+ <- 3 [u8; 4]
        // So put it in a ?vec? of slices/buffers or something and then
        // read_exact those into the buffer.
        // In "memory", that would look like:
        // [11112222111122221111222211112222]
        //      |--|    |--|    |--|    |--|
        //        0       1       2       3
        // with incomplete chunk:
        // +-----+---00+ <- 0 [u8; 3]
        // |     |---00| <- 1 [u8; 3]
        // |     |---00| <- 2 [u8; 3]
        // +-----+-----+<- 0 [u8; 4]
        // |-----|     |<- 1 [u8; 4]
        // |     |     |
        // +-----+-----+
        // [1111222111122211112221111222]
        //      |-|    |-|    |-|    |-|
        //       0      1      2      3
        // but padding (in float predictor) is mixed into the data, so we cannot
        // directly decode into the output buffer...
        //
        // maybe float prediction messes the last bit up though, because it
        // shuffles padding bytes into the mix and then we'd need a full-sized
        // temporary or output buffer.
        const CHUNK_SIZE: usize = 7;
        let chunks = slice.chunks_exact_mut(CHUNK_SIZE);
        thread::scope(|scope| {
            // Example: processing both halves concurrently
            let mut handles = Vec::new();
            for (i, c) in chunks.enumerate() {
                handles.push(scope.spawn(move || {
                    for v in c {
                        *v += i as u8;
                    }
                }));
            }

            // Wait for all threads
            for handle in handles {
                handle.join().unwrap();
            }
        });
        // Check the modified data in the original vector
        println!("Modified data: {:?}", slice);
        assert_eq!(req, *slice);
    }
}
