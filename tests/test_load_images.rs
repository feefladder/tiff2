use std::fs;
use std::ops::Range;
use std::path::Path;

use async_trait::async_trait;
use bytes::Bytes;
use image::{DynamicImage, ImageBuffer, Luma, Rgb, Rgba};
use tokio::fs::File;
use tokio::io::{AsyncReadExt, AsyncSeekExt};

struct TokioFile(std::path::PathBuf);

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

    async fn get_range(&self, range: Range<u64>) -> TiffResult<Bytes> {
        //create local variables so we don't use self or range in the move block
        let p = self.0.clone();
        let r = range.clone();
        let mut f = File::open(p).await?;
        f.seek(std::io::SeekFrom::Start(r.start)).await?;
        let len = usize::try_from(r.end - r.start)?;
        let mut buffer = vec![0u8; len];

        // Read the data into the buffer
        f.read_exact(&mut buffer)
            .await
            .map_err(|e| TiffError::TransportError(Box::new(e)))?;
        Ok(Bytes::copy_from_slice(&buffer))
    }
}

#[test_log::test(tokio::test)]
async fn test_rgba_8bit_deflate() {
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
        // info!("testing {path:?}");

        let f = TokioFile::new(path).unwrap();
        let t =
            f.0.file_name()
                .unwrap()
                .to_str()
                .unwrap()
                .split_once('_')
                .unwrap()
                .0
                .to_string();
        let dtype =
            f.0.file_name()
                .unwrap()
                .to_str()
                .unwrap()
                .rsplit_once('_')
                .unwrap()
                .0
                .to_string();

        println!("testing {:42}: type: {:?}", f.0.to_str().unwrap(), t);

        let mut d = Decoder::new(f).await.expect("Could not make decoder");
        d.scan_ifds().await.expect("can scan ifds");
        d.read_image_ifds().await.expect("can read images");
        // read all images
        for (_offset, img) in d.images {
            println!(
                "decoding image {:?}x{:?}, predictor: {:?}",
                img.chunk_opts.image_width, img.chunk_opts.image_height, img.chunk_opts.predictor
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
            let img_bytes = (u64::from(img.chunk_opts.image_width)
                * u64::from(img.chunk_opts.image_height)
                * u64::from(img.chunk_opts.bits_per_sample)
                * u64::try_from(img.chunk_opts.samples_per_pixel()).unwrap())
            .div_ceil(8);
            let mut img_buf = vec![0u8; usize::try_from(img_bytes).unwrap()];
            // split the buffer per chunk
            // this won't work with chunks or chunks_exact;
            let mut rem_buf = &mut img_buf[..];
            // make this into a loop based on tile_attributes and corresponditags
            for (index, data) in ranges.iter().enumerate() {
                let i_32 = u32::try_from(index).unwrap();
                let chunk_size = img
                    .chunk_opts
                    .chunk_data_dimensions(i_32)
                    .expect("chunk_dims");
                // debug!("chunk_size: {chunk_size:?}, row_stride: {:?}", img.chunk_opts.output_row_stride(i_32));
                if usize::try_from(chunk_size.0 * chunk_size.1).unwrap() > rem_buf.len() {
                    eprintln!("Chunk didn't fit in buffer");
                    break;
                }
                let (first, rb) = rem_buf.split_at_mut(
                    usize::try_from(chunk_size.1).unwrap()
                        * img.chunk_opts.output_row_stride(i_32).unwrap(),
                );
                rem_buf = rb;

                if let Err(e) = ChunkDecoder::expand_chunk(
                    data,
                    &mut first
                        .chunks_exact_mut(img.chunk_opts.output_row_stride(i_32).unwrap())
                        .collect::<Vec<&mut [u8]>>(),
                    &img.chunk_opts,
                    i_32,
                ) {
                    eprintln!("could not decode chunk: {e}");
                };
            }
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
            if dtype != "Byte" {
                continue;
            }
            let dyn_img = match t.as_str() {
                "rgba" => DynamicImage::from(
                    ImageBuffer::<Rgba<u8>, Vec<u8>>::from_raw(
                        img.chunk_opts.image_width,
                        img.chunk_opts.image_height,
                        img_buf,
                    )
                    .unwrap(),
                ),
                "rgb" => DynamicImage::from(
                    ImageBuffer::<Rgb<u8>, Vec<u8>>::from_raw(
                        img.chunk_opts.image_width,
                        img.chunk_opts.image_height,
                        img_buf,
                    )
                    .unwrap(),
                ),
                "bw" => DynamicImage::from(
                    ImageBuffer::<Luma<u8>, Vec<u8>>::from_raw(
                        img.chunk_opts.image_width,
                        img.chunk_opts.image_height,
                        img_buf,
                    )
                    .unwrap(),
                ),
                _ => continue,
            };
            viuer::print(&dyn_img, &viuer::Config::default()).expect("Could not show image");
            // sleep(Duration::from_millis(250)).await;
        }
    }
    panic!()
}
