use async_trait::async_trait;
use bytes::Bytes;
use image::{DynamicImage, ImageBuffer, Luma, Rgb, Rgba};
use log::{debug, error, info};
use rayon::prelude::*;
use std::{
    collections::BTreeMap,
    fs,
    ops::Range,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};
use tiff2::{
    decoder::{ChunkDecoder, CogReader, Decoder},
    error::{TiffError, TiffResult},
    structs::{tags::PlanarConfiguration, ChunkOpts, Tag}, ChunkType,
};
use tokio::{
    fs::File,
    io::{AsyncReadExt, AsyncSeekExt},
    task::JoinError,
    time::sleep,
};
struct TokioFile(PathBuf);

impl TokioFile {
    fn new(p: impl AsRef<Path>) -> TiffResult<Self> {
        if p.as_ref().is_file() {
            Ok(Self(p.as_ref().to_owned()))
        } else {
            Err(TiffError::IoError(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("File not found at path {:?}", p.as_ref()),
            )))
        }
    }
}

#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
impl CogReader for TokioFile {
    const IFD_REQ_SIZE: u64 = 16 * 1024;

    async fn get_ranges(&self, ranges: &[Range<u64>]) -> TiffResult<Vec<Bytes>> {
        let mut tasks = Vec::with_capacity(ranges.len());
        for range in ranges {
            //create local variables so we don't use self or range in the move block
            let p = self.0.clone();
            let r = range.clone();
            let task = async move {
                let mut f = File::open(p).await?;
                f.seek(std::io::SeekFrom::Start(r.start)).await?;
                let len = usize::try_from(r.end - r.start)?;
                let mut buffer = vec![0u8; len];

                // Read the data into the buffer
                f.read_exact(&mut buffer)
                    .await
                    .map_err(|e| TiffError::TransportError(Box::new(e)))?;
                Ok(Bytes::copy_from_slice(&buffer))
            };
            tasks.push(task);
        }
        futures::future::join_all(tasks)
            .await
            .into_iter()
            .collect::<Result<Vec<_>, _>>()
        // .map_err(|e| TiffError::TransportError(Box::new(e)))
    }
}

fn row_major_to_col_major<T>(
    data: &mut [Option<T>],
    width: usize,
    height: usize,
) -> TiffResult<Vec<T>> {
    assert_eq!(
        data.len(),
        width * height,
        "Data size must match dimensions"
    );

    let mut rearranged = Vec::with_capacity(data.len());

    for old_i in 0..data.len() {
        // for 5-wide,4-tall
        //  0  1  2  3  4
        //  5  6  7  8  9
        // 10 11 12 13 14
        // 15 16 17 18 19
        // new
        //  0  4  8 12 16
        //  1  5  9 13 17
        //  2  6 10 14 18
        //  3  7 11 15 19
        // old_i 0  1  2  3   4  5  6  7   8  9 10 11  12 13 14 15
        // new_i 0  5 10 15   1  6 11 16   2  7 12 17   3  8 13 18
        // (old_i%4)*5 + old_i/4
        // (old_i%height)*width + old_i/height
        rearranged.push(
            data[(old_i % height) * width + old_i / height]
                .take()
                .ok_or(TiffError::LimitsExceeded)?,
        )
    }

    Ok(rearranged)
}

/// split a buffer for an entire image into
///
/// ```raw
///  0     1     2     3     4
/// ----  ----  ----  ----  --..
/// ----  ----  ----  ----  --..
/// ----  ----  ----  ----  --..
/// ----  ----  ----  ----  --..
///   5     6    7      8     9
/// ----  ----  ----  ----  --..
/// ----  ----  ----  ----  --..
/// ....  ....  ....  ....  ....
/// ....  ....  ....  ....  ....
/// ```
// fn split_buffer<'a>(
//     mut buf: &'a mut [u8],
//     chopts: Arc<ChunkOpts>,
// ) -> TiffResult<Vec<Vec<&'a mut [u8]>>> {
//     let img_bytes = (u64::from(chopts.image_width)
//         * u64::from(chopts.image_height)
//         * u64::from(chopts.samples)
//         * u64::from(chopts.bits_per_sample))
//     .div_ceil(8);
//     if buf.len() < img_bytes as usize {
//         return Err(TiffError::LimitsExceeded);
//     }
//     let chdims = chopts.chunk_dimensions().unwrap();
//     let chunks_across = chopts.image_width.div_ceil(chdims.0) as u64;
//     let chunks_down = chopts.image_height.div_ceil(chdims.1) as u64;

//     let mut chunks_v: Vec<_> = (0..chunks_across * chunks_down)
//         .map(|_| Vec::new())
//         .collect();
//     for row in 0u64..u64::from(chopts.image_height) {
//         for chunk_row in 0u64..chunks_across {
//             let chunks_down = row / chdims.1 as u64;
//             let ch_index = usize::try_from(chunk_row + chunks_down * chunks_across)?;

//             let mut row_length = chdims.0;
//             if chunk_row + 1 == chunks_across {
//                 row_length = chopts.image_width % chdims.0;
//             }
//             let (row, rb) = buf.split_at_mut(row_length as usize);
//             chunks_v[ch_index].push(row)
//         }
//     }
//     Ok(chunks_v)
// }

#[tokio::main]
async fn main() -> TiffResult<()> {
    env_logger::init();
    let mut m = BTreeMap::new();
    let mut prev = Instant::now();
    for p in fs::read_dir("./tests/img/gdal/out").unwrap() {
        let path = p.unwrap().path();
        if !path.is_file() {
            continue;
        }

        if let Some(ex) = path.extension() {
            if ex != "tif" && ex != "tiff" {
                continue;
            }
        } else {
            continue;
        }

        let fname = path.file_name().unwrap().to_str().unwrap().to_owned();
        //
        // if !fname.contains("RGBA") {continue;}
        if !fname.contains("PIXEL") {
            continue;
        }
        // if !fname.contains("YES") {
        //     continue;
        // }
        // if !fname.contains("RGB_") {continue;}

        // if fname.contains("Byte") {
        //     continue;
        // }
        // if fname.contains("UI") {
        //     continue;
        // }
        // if fname.contains("Float") {
        //     continue;
        // }
        if fname.contains("JPEG") {
            continue;
        }
        // if fname.contains("pred2") {
        //     continue;
        // }

        info!("testing {path:?}");
        let t = fname.split_once('_').unwrap().0;
        let f = TokioFile::new(path).unwrap();
        info!("testing {:42}: type: {:?}", f.0.to_str().unwrap(), fname);

        let mut d = Decoder::new(f).await.expect("Could not make decoder");
        d.scan_ifds().await.expect("can scan ifds");
        d.read_image_ifds().await.expect("can read images");
        // read all images
        let img = d.images[d.ifd_offsets().last().unwrap()].clone();
        let chopts = img.chunk_opts.clone();
        debug!(
            "decoding image {:?}x{:?}, predictor: {:?}",
            chopts.image_width, chopts.image_height, chopts.predictor
        );
        let mut chunks_v = Vec::with_capacity(img.chunk_offsets.len());
        for i in 0..img.chunk_offsets.len() {
            chunks_v.push(img.chunk_offsets[i]..img.chunk_offsets[i] + img.chunk_bytes[i]);
        }
        // get the compressed data
        let ranges = d
            .reader
            .get_ranges(&chunks_v[..])
            .await
            .expect("could not get chunk ranges");
        // create a buffer for the entire image
        let img_bytes = (u64::from(chopts.image_width)
            * u64::from(chopts.image_height)
            * u64::from(chopts.samples)
            * u64::from(chopts.bits_per_sample))
        .div_ceil(8);
        let mut img_buf = vec![0u8; usize::try_from(img_bytes).unwrap()];
        // let mut split_buf = split_buffer(&mut img_buf, chopts).expect("could not split buffer");
        // split the buffer per chunk
        // this won't work with chunks or chunks_exact;
        // let mut rem_buf = &mut img_buf[..];
        // make this into a loop based on tile_attributes and
        // corresponditags
        #[allow(unused_doc_comments)]
        /// we have to do smart per-rowblock things
        /// That is:
        /// ```raw
        ///  0     1     2     3     4
        /// ----  ----  ----  ----  --.. -\
        /// ----  ----  ----  ----  --..   | this group needs to be done
        /// ----  ----  ----  ----  --..   | sequentially to keep rust happy
        /// ----  ----  ----  ----  --.. -/
        ///   5     6    7      8     9       separate rowblocks can be processed simultaneously
        /// ----  ----  ----  ----  --.. -\
        /// ----  ----  ----  ----  --..   |
        /// ....  ....  ....  ....  ....   |
        /// ....  ....  ....  ....  .... -/
        /// ```
        ///
        /// ```
        ///   0    1    2    3    4
        /// ---- ---- ---- ---- --..
        ///   5    6    7    8    9
        /// ---- ---- ---- ---- --..
        ///  10   11   12   13   14
        /// ---- ---- ---- ---- --..
        ///  15   16   17   18   19
        /// ---- ---- ---- ---- --..
        ///
        ///   0    4    8   12   16
        /// ---- ---- ---- ---- --..
        ///   1    5    9   13   17
        /// ---- ---- ---- ---- --..
        ///   2    6   10   14   18
        /// ---- ---- ---- ---- --..
        ///   3    7   11   15   19
        /// ---- ---- ---- ---- --..
        /// ```
        let chdims = chopts.chunk_dimensions()?;

        if chdims.1 == 1 {continue;}

        let img_bwidth: usize = (chopts.image_width as u64
            * chopts.samples_per_pixel() as u64
            * chopts.bits_per_sample as u64)
            .div_ceil(8)
            .try_into()?;
        let chunk_bwidth: usize =
            (chdims.0 as u64 * chopts.samples_per_pixel() as u64 * chopts.bits_per_sample as u64)
                .div_ceil(8)
                .try_into()?;
        let chunks_down: usize = chopts.image_height.div_ceil(chdims.1).try_into()?;
        let chunks_across: usize = img_bwidth.div_ceil(chunk_bwidth);
        info!("decoding {:?}x{:?} img with {:?}x{:?} chunks", chopts.image_width, chopts.image_height, chdims.0, chdims.1);
        img_buf
            .chunks_mut(img_bwidth * usize::try_from(chdims.1)?)
            .enumerate()
            .map(|(i, rg)| {
                // info!("row_group {i:?}: {:?}", rg.len());
                (i,rg)
            })
            .for_each(|(rg_i, rg)| {
                if chopts.chunk_type == ChunkType::Strip {
                    let _ = ChunkDecoder::expand_chunk(&ranges[rg_i], &mut rg.chunks_mut(img_bwidth).collect::<Vec<_>>()[..], &chopts.clone(), rg_i as u32);
                } else {

                let c_height = if rg_i + 1 == chunks_down {
                    chopts.image_height as usize % chdims.1 as usize
                } else {
                    chdims.1 as usize
                };
                let mut chunked = rg
                    .chunks_mut(img_bwidth)
                    .map(|row| row.chunks_mut(chunk_bwidth).collect::<Vec<_>>())
                    .flatten()
                    .map(|c| Some(c))
                    .collect::<Vec<_>>();
                if c_height == 0 {
                    error!("bla {rg_i}: {c_height:?}, {:?}", chunked[0].as_ref().unwrap().len());
                }
                
                row_major_to_col_major(&mut chunked, chunks_across, c_height)
                    .unwrap()
                    .par_chunks_mut(c_height)
                    .enumerate()
                    .for_each(|(i, ch)| {
                        let _ = ChunkDecoder::expand_chunk(
                            &ranges[i][..],
                            ch,
                            &chopts.clone(),
                            i.try_into().unwrap(),
                        );
                    });

                // rearranged//.chunks_mut(c_height).flatten().collect::Vec<_>()
                // rearranged.collect::<Vec<_>>().chunks_mut(c_height).collect::<Vec<_>>()
                // for (i, ch) in rearranged.chunks_mut(c_height).enumerate() {
                //
                // }
                }
            });

        // let row_groups: Vec<_> = img_buf
        //     .chunks_mut(usize::try_from(img_bwidth * chdims.1 as u64)?)
        //     .map(|rg|
        //     rg.chunks_mut(usize::try_from(img_bwidth).unwrap())
        //         .map(|row| row.chunks_mut(chunk_bwidth as usize))
        //     )
        //         .flatten()
        //         .flatten()
        //         .map(|c| Some(c))
        //         .collect();
        // // for row_group in row_groups {
        //     let mut rows: Vec<Option<_>> = row_group

        //     row_major_to_col_major(&mut rows, chunks_across, chunk_dims.1);
        // }

        // for (index, data) in ranges.iter().enumerate() {
        //     let i_32 = u32::try_from(index).unwrap();
        //     let chunk_size = img
        //         .chunk_opts
        //         .chunk_data_dimensions(i_32)
        //         .expect("chunk_dims");
        //     let n_chunk_bytes =
        //         usize::try_from(chunk_size.1).unwrap() * chopts.output_row_stride(i_32).unwrap();
        //     debug!(
        //         "chunk_size: {chunk_size:?}, row_stride: {:?}",
        //         chopts.output_row_stride(i_32)
        //     );
        //     // if n_chunk_bytes > rem_buf.len() {
        //     //     error!("{fname:42} Chunk {index:2?}/{:?} didn't fit in buffer of size {:3?}x{:3?}x{:?}x{:2?}={img_bytes:7?}, missing {n_chunk_bytes:<8?}",
        //     //         ranges.len(),
        //     //         chopts.image_width,
        //     //         chopts.image_height,
        //     //         chopts.samples,
        //     //         chopts.bits_per_sample
        //     //     );
        //     //     // error!("")
        //     //     break;
        //     // }
        //     // let (first, rb) = rem_buf.split_at_mut(n_chunk_bytes);
        //     // rem_buf = rb;

        //     if let Err(e) = ChunkDecoder::expand_chunk(
        //         data,
        //         &mut &mut img_buf.chunks_exact_mut(42).collect::<Vec<_>>()[..],
        //         &chopts,
        //         i_32,
        //     ) {
        //         error!("could not decode chunk: {e}");
        //     };
        // }
        // image ordering in case of planar configuration:
        // > The components are stored in separate “component planes.” The
        // > values in StripOffsets and StripByteCounts are then arranged as a 2-dimensional
        // > array, with SamplesPerPixel rows and StripsPerImage columns. (All of the col-
        // > umns for row 0 are stored first, followed by the columns of row 1, and so on.)
        // > PhotometricInterpretation describes the type of data stored in each component
        // > plane. For example, RGB data is stored with the Red components in one compo-
        // > nent plane, the Green in another, and the Blue in another.
        // so:
        // spp
        // ^
        // |
        // +--> chunks
        //       ___col0___________col2_______________colN______
        // row1 | Chunk1[RED]  , Chunk2[RED]  , ... ChunkN[RED]
        // row2 | Chunk1[GREEN], Chunk2[GREEN], ... ChunkN[GREEN]
        // row3 | Chunk1[BLUE] , Chunk2[BLUE] , ... ChunkN[BLUE]
        // "in memory": [Chunk1[RED],Chunk2[RED],...ChunkN[RED],Chunk1[GREEN]...]
        // give the chunks to expand_chunk
        let n = Instant::now();
        m.insert(n - prev, fname.clone());
        info!("decoding {:?} cost {:?}", &m[&(n - prev)], n - prev);
        // sleep(Duration::from_millis(2000).saturating_sub(n-prev)).await;
        prev = n;

        // if !fname.contains("ASDF") {continue;}
        let dyn_img = match t {
            "RGBA" | "rgba" => DynamicImage::from(
                ImageBuffer::<Rgba<u8>, _>::from_raw(
                    chopts.image_width,
                    chopts.image_height,
                    img_buf,
                )
                .unwrap(),
            ),
            "RGB" | "rgb" => DynamicImage::from(
                ImageBuffer::<Rgb<u8>, _>::from_raw(
                    chopts.image_width,
                    chopts.image_height,
                    img_buf,
                )
                .unwrap(),
            ),
            "BW" | "bw" => DynamicImage::from(
                ImageBuffer::<Luma<u8>, _>::from_raw(
                    chopts.image_width,
                    chopts.image_height,
                    img_buf,
                )
                .unwrap(),
            ),
            _ => continue,
        };
        // viuer::print(&dyn_img, &viuer::Config::default()).expect("Could not
        // show image");
        println!("{:?}", dyn_img.height());
    }
    

    for (k, v) in m.iter().rev().take(10) {
        println!("{v:32} took {k:?}",);
    }
    Ok(())
}

#[test]
fn test_image_grouping() {
    let width: usize = 13;
    let height: usize = 13;
    let chwidth: usize = 3;
    let chheight: usize = 3;

    let chunks_down: usize = height.div_ceil(chheight);
    let chunks_across: usize = width.div_ceil(chwidth);

    let mut data: Vec<_> = (0..width * height).collect();

    println!("OG:");
    for (i, row) in data.chunks(width).enumerate() {
        for chslice in row.chunks(chwidth) {
            print!("{chslice:2?} ");
        }
        println!("");
        if (i % chheight) == chheight - 1 {
            println!("");
        }
    }

    let row_groups: Vec<_> = data
        .chunks_mut(width * chheight)
        .enumerate()
        .map(|(rg_i, rg)| {
            let c_height = if rg_i + 1 == chunks_down {
                height % chheight
            } else {
                chheight
            };
            let mut chunked = rg
                .chunks_mut(width)
                .map(|row| row.chunks_mut(chwidth).collect::<Vec<_>>())
                .flatten()
                .map(|c| Some(c))
                .collect::<Vec<_>>();
            let mut rearranged =
                row_major_to_col_major(&mut chunked, chunks_across, c_height).unwrap();
            // rearranged//.chunks_mut(c_height).flatten().collect::Vec<_>()
            // rearranged.collect::<Vec<_>>().chunks_mut(c_height).collect::<Vec<_>>()
            for (i, ch) in rearranged.chunks_mut(c_height).enumerate() {
                let index = i + rg_i * chunks_across;
                println!("chunk {index}");
                println!("{ch:?}");
            }
        })
        // .enumerate()
        // .map(|(i, mut rg)| {
        //     let c_height = if i+1 == chunks_down { height % chheight } else {chheight};
        //     rg.chunks_mut(c_height).collect::<Vec<_>>()
        // })
        //     .flatten()
        // .flatten()
        // .map(|c| Some(c))
        .collect::<Vec<_>>();

    println!("reorganized:");
    for rg in row_groups {
        println!("{rg:?}");
    }

    panic!()
}
