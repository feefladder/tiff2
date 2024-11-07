use std::collections::BTreeMap;

use bytes::Bytes;

use crate::{
    error::{TiffFormatError, TiffResult},
    structs::{Ifd, Image},
    ByteOrder,
};

use super::{CogReader, Limits};

// use object_store::ObjectStore;
/// Async decoder
///
#[derive(Debug, PartialEq)]
pub struct Decoder<R: CogReader> {
    reader: R,
    /// buffer holding data that can be read synchronously
    /// if implementing your own decoder, this is the place to add smart things,
    /// together with `buf_start`
    buffer: Bytes,
    /// start location of the buffer
    buf_start: u64,
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
    images: BTreeMap<u64, Image>,
    /// Ifds that are not images.
    /// These should not happen that often, but are included for completeness
    pub meta_ifds: BTreeMap<u64, Ifd>,
}

impl<R: CogReader + Sync> Decoder<R> {
    /// Create a new decoder from the source
    /// Will read an initial IFD chunk at offset zero
    pub async fn new(reader: R) -> TiffResult<Self> {
        // let buf = ;
        let buffer = reader.read_ifd(0).await?; //match buf.try_into_mut() {
                                                //     Ok(r) => r,
                                                //     Err(b) => BytesMut::from(b),
                                                // };
                                                // let mut endianness = [0u8;2];
                                                // (&buffer[..]).read_exact(&mut endianness[..])?;
        let byte_order = match <&[u8; 2]>::try_from(&buffer[0..2]).unwrap() {
            b"II" => ByteOrder::LittleEndian,
            b"MM" => ByteOrder::BigEndian,
            _ => {
                return Err(TiffFormatError::TiffSignatureNotFound.into());
            }
        };
        let bigtiff = match byte_order.u16(buffer[2..4].try_into().unwrap()) {
            42 => false,
            43 => {
                if byte_order.u16(buffer[4..6].try_into().unwrap()) != 8 {
                    return Err(TiffFormatError::TiffSignatureNotFound.into());
                }
                if byte_order.u16(buffer[6..8].try_into().unwrap()) != 0 {
                    return Err(TiffFormatError::TiffSignatureNotFound.into());
                }
                true
            }
            _ => {
                return Err(TiffFormatError::TiffSignatureInvalid.into());
            }
        };
        let ifd_offsets = vec![if bigtiff {
            byte_order.u64(buffer[8..8 + 8].try_into().unwrap())
        } else {
            u64::from(byte_order.u32(buffer[4..4 + 4].try_into().unwrap()))
        }];
        // remove the header from buffer, aka make the buffer point to the first offset
        // if ifd_offsets[0] < u64::try_from(buffer.len())? {
        //     buffer.split_to(usize::try_from(ifd_offsets[0])?);
        // }
        Ok(Self {
            buf_start: 0,
            buffer,
            reader,
            byte_order,
            bigtiff,
            limits: Default::default(),
            ifd_offsets,
            images: BTreeMap::new(),
            meta_ifds: BTreeMap::new(),
        })
    }
}

#[cfg(test)]
mod test {

    use super::Decoder;
    use crate::{decoder::CogReader, error::TiffResult, ByteOrder};
    use async_trait::async_trait;
    use bytes::Bytes;
    use std::{cmp::min, collections::BTreeMap, ops::Range, thread};

    #[async_trait]
    impl CogReader for &[u8] {
        const IFD_REQ_SIZE: u64 = 16 * 1024;
        async fn get_ranges(&self, ranges: &[Range<u64>]) -> TiffResult<Vec<Bytes>> {
            let mut res = Vec::with_capacity(ranges.len());
            for range in ranges {
                let end = min(usize::try_from(range.end)?, self.len());
                res.push(Bytes::copy_from_slice(
                    &self[usize::try_from(range.start)?..end],
                ));
            }
            Ok(res)
        }
    }

    #[tokio::test]
    async fn test_new_decoder_notbig_littleendian() {
        let data = [
            // TIFF Header (8 bytes)
            b'I', b'I', // Byte order: "II" for little-endian
            42, 0, // Magic number: 42 (0x2A00)
            8, 0, 0,
            0, // Offset to first IFD: 8

               // // First IFD (12 + 2 + 4 = 18 bytes total)
               // 1, 0,              // Number of directory entries: 1

               // // IFD Entry for ImageWidth
               // 0x00, 0x01,        // Tag for ImageWidth: 256 (0x0100)
               // 4, 0,              // Type: LONG (4 bytes per value)
               // 1, 0, 0, 0,        // Count: 1
               // 42, 0, 0, 0,       // Value: 42 (ImageWidth)

               // 0, 0, 0, 0         // Next IFD offset: 0 (no more IFDs)
        ];
        assert_eq!(
            Decoder::new(&data[..]).await.unwrap(),
            Decoder {
                reader: &data[..],
                buffer: Bytes::copy_from_slice(&data[..]),
                buf_start: 0,
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
            16, 0, 0, 0, 0, 0, 0,
            0, // Offset to first IFD (16)

               // // First IFD
               // 1, 0, 0, 0, 0, 0, 0, 0,  // Number of directory entries (8 bytes): 1

               // // IFD Entry for ImageWidth (20 bytes)
               // 0x00, 0x01, 0, 0,        // Tag for ImageWidth: 256 (0x0100)
               // 4, 0, 0, 0,              // Type: LONG (4 bytes per value)
               // 1, 0, 0, 0, 0, 0, 0, 0,  // Count: 1 (8 bytes)
               // 42, 0, 0, 0, 0, 0, 0, 0, // Value: 42 (8 bytes for BigTIFF)

               // 0, 0, 0, 0, 0, 0, 0, 0   // Next IFD offset: 0 (indicating no more IFDs)
        ];
        assert_eq!(
            Decoder::new(&data[..]).await.unwrap(),
            Decoder {
                reader: &data[..],
                buffer: Bytes::copy_from_slice(&data[..]),
                buf_start: 0,
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
            0, 0, 0,
            8, // Offset to first IFD: 8

               // // First IFD (12 + 2 + 4 = 18 bytes total)
               // 1, 0,              // Number of directory entries: 1

               // // IFD Entry for ImageWidth
               // 0x00, 0x01,        // Tag for ImageWidth: 256 (0x0100)
               // 4, 0,              // Type: LONG (4 bytes per value)
               // 1, 0, 0, 0,        // Count: 1
               // 42, 0, 0, 0,       // Value: 42 (ImageWidth)

               // 0, 0, 0, 0         // Next IFD offset: 0 (no more IFDs)
        ];
        assert_eq!(
            Decoder::new(&data[..]).await.unwrap(),
            Decoder {
                reader: &data[..],
                buffer: Bytes::copy_from_slice(&data[..]),
                buf_start: 0,
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
            0, 0, 0, 0, 0, 0, 0,
            16, // Offset to first IFD (16)

                // // First IFD
                // 1, 0, 0, 0, 0, 0, 0, 0,  // Number of directory entries (8 bytes): 1

                // // IFD Entry for ImageWidth (20 bytes)
                // 0x00, 0x01, 0, 0,        // Tag for ImageWidth: 256 (0x0100)
                // 4, 0, 0, 0,              // Type: LONG (4 bytes per value)
                // 1, 0, 0, 0, 0, 0, 0, 0,  // Count: 1 (8 bytes)
                // 42, 0, 0, 0, 0, 0, 0, 0, // Value: 42 (8 bytes for BigTIFF)

                // 0, 0, 0, 0, 0, 0, 0, 0   // Next IFD offset: 0 (indicating no more IFDs)
        ];
        assert_eq!(
            Decoder::new(&data[..]).await.unwrap(),
            Decoder {
                reader: &data[..],
                buffer: Bytes::copy_from_slice(&data[..]),
                buf_start: 0,
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
