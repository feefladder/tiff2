// mod test {
//     use crate::{decoder::CogReader, structs::Image};

//     use std::{collections::HashMap, sync::Arc};
//     type OverviewLevel = u8;
//     struct CogDecoder {
//         /// OverviewLevel->Image map (could be a vec)
//         images: HashMap<OverviewLevel, Arc<Image>>,
//         // geo_data: Idk,
//         reader: Arc<dyn CogReader>,
//     }

//     impl CogDecoder {
//         /// requiring mutable access to self is suboptimal
//         fn get_chunk(
//             &mut self,
//             i_chunk: u64,
//             zoom_level: OverviewLevel,
//         ) -> impl Future<Output = DecodingResult> {
//             match self.images.get(&zoom_level) {
//                 None => panic!(), // in this piece of code, we'd have to await IFD retrieval+decoding
//                 Some(img) => img.clone().decode_chunk(i_chunk), // since this returns a future that doesn't reference self, we are happy
//             }
//         }
//     }

//     impl Image {
//         // better move this to decoder, only make image return the offset and length
//         fn decode_chunk<R>(&self, reader: R, i_chunk: u64) -> impl Future<Output = DecodingResult> {
//             let chunk_offset = self.chunk_offsets[i_chunk];
//             let chunk_bytes = self.chunk_bytes[i_chunk];
//             ChunkDecoder::decode(r, chunk_offset, chunk_bytes, self.chunk_opts.clone())
//         }
//     }

//     #[tokio::test]
//     fn test_concurrency() {
//         let decoder = CogDecoder::from_url("https://enourmous-cog.com")
//             .await
//             .expect("Decoder should build");
//         decoder
//             .read_overviews(vec![0, 5])
//             .await
//             .expect("Decoder should read ifds");
//         // get a chunk from the highest resolution image
//         let chunk_1 = decoder.get_chunk(42, 0);
//         // get a chunk from a lower resolution image
//         let chunk_2 = decoder.get_chunk(42, 5);
//         let data = (chunk_1.await, chunk_2.await);
//     }

//     #[tokio::test]
//     fn test_concurrency_fail() {
//         let decoder = CogDecoder::from_url("https://enourmous-cog.com")
//             .await
//             .expect("Decoder should build");
//         decoder
//             .read_overviews(vec![0])
//             .await
//             .expect("decoder should read ifds");
//         // get a chunk from the highest resolution image
//         let chunk_1 = decoder.get_chunk(42, 0);
//         // get a chunk from a lower resolution image
//         let chunk_2 = decoder.get_chunk(42, 5); //panic!
//         let data = (chunk_1.await, chunk_2.await);
//     }

//     // how HeroicKatana would do it if I understand correctly:
//     #[tokio::test]
//     fn test_concurrency_recover() {
//         let decoder = CogDecoder::from_url("https://enourmous-cog.com")
//             .await
//             .expect("Decoder should build");
//         decoder
//             .read_overviews(vec![0])
//             .await
//             .expect("decoder should read ifds");
//         // get a chunk from the highest resolution image
//         let chunk_1 = decoder.get_chunk(42, 0).unwrap();
//         // get a chunk from a lower resolution image
//         if let OverviewNotLoadedError(chunk_err) = decoder.get_chunk(42, 5).unwrap_err() {
//             // read_overviews changes state of the decoder to LoadingIfds
//             decoder.read_overviews(chunk_err).await;
//         }
//         let chunk_2 = decoder.get_chunk(42, 5);
//         let data = (chunk_1.await, chunk_2.await);
//     }
// }
