use async_trait::async_trait;
use bytes::Bytes;
use log::{error, info};
use std::{
    collections::BTreeMap,
    fs,
    ops::Range,
    path::{Path, PathBuf},
    time::Instant,
};
use tiff2::{
    decoder::{CogReader, Decoder},
    error::{TiffError, TiffResult},
};
use tokio::{
    fs::File,
    io::{AsyncReadExt, AsyncSeekExt},
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

    async fn get_range(&self, range: Range<u64>) -> TiffResult<Bytes> {
        let mut f = File::open(self.0.clone()).await?;
        f.seek(std::io::SeekFrom::Start(range.start)).await?;
        let len = usize::try_from(range.end - range.start)?;
        let mut buffer = vec![0u8; len];

        // Read the data into the buffer
        f.read_exact(&mut buffer)
            .await
            .map_err(|e| TiffError::TransportError(Box::new(e)))?;
        Ok(Bytes::copy_from_slice(&buffer))
    }
}

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
        // if fname.contains("JPEG") {
        //     continue;
        // }
        // if fname.contains("pred2") {
        //     continue;
        // }

        info!("testing {path:?}");
        #[allow(unused)]
        let t = fname.split_once('_').unwrap().0;
        let f = TokioFile::new(path).unwrap();
        info!("testing {:42}: type: {:?}", f.0.to_str().unwrap(), fname);

        let mut d = Decoder::new(f).await.expect("Could not make decoder");
        d.scan_ifds().await.expect("can scan ifds");
        d.read_image_ifds().await.expect("can read images");

        #[allow(unused)]
        let img_res;
        match d.decode_image(0).await {
            #[allow(unused)]
            Ok(res) => img_res = res,
            Err(e) => error!("help! {e}"),
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
        let n = Instant::now();
        m.insert(n - prev, fname.clone());
        info!("decoding {:?} cost {:?}", &m[&(n - prev)], n - prev);
        // sleep(Duration::from_millis(2000).saturating_sub(n-prev)).await;
        prev = n;
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
