/* global GeoTIFF:false, plotty:false */
// const { Pool, fromUrl } = GeoTIFF;
use log::{error, info};
use std::{collections::BTreeMap, fs, path::Path, time::Instant};
use tiff2::{decoder::Decoder, error::TiffResult};

mod common;
use common::{img_print, TokioFile};

// const imageWindow = [0, 0, 500, 500];
const TIFFS: &[&str] = &[
    // "stripped.tiff",
    "tiled.tiff",
    "interleave.tiff",
    "tiledplanar.tiff",
    "float32.tiff",
    "uint32.tiff",
    "int32.tiff",
    "float64.tiff",
    "lzw.tiff", // <-- this errors on index out-of-bounds
    "tiledplanarlzw.tiff",
    "float64lzw.tiff",
    "lzw_predictor.tiff",
    "deflate.tiff",
    "deflate_predictor.tiff",
    "deflate_predictor_tiled.tiff",
    "lerc.tiff",
    "lerc_interleave.tiff",
    "lerc_deflate.tiff",
    "float32lerc.tiff",
    "float32lerc_interleave.tiff",
    "float32lerc_deflate.tiff",
    "n_bit_tiled_10.tiff",
    "n_bit_11.tiff",
    "n_bit_12.tiff",
    "n_bit_13.tiff",
    // "n_bit_14.tiff",
    // "n_bit_15.tiff",
    // "n_bit_interleave_10.tiff",
    // "n_bit_interleave_12.tiff",
    // "n_bit_interleave_14.tiff",
    // "n_bit_interleave_15.tiff",
    // "float_n_bit_16.tiff",
    // "float_n_bit_tiled_16.tiff",
    // "float_n_bit_interleave_16.tiff",
];

const RGBTIFFS: &[&str] = &[
    "stripped.tiff",
    "rgb.tiff",
    "BigTIFF.tif",
    "rgb_paletted.tiff",
    "cmyk.tif",
    "ycbcr.tif",
    "cielab.tif",
    "5ae862e00b093000130affda.tif",
    "jpeg.tiff",
    "jpeg_ycbcr.tiff",
];

// const pool = new Pool();
/// dir pointing to geotiff repo
const GEOTIFF_DIR: &str = "/home/user/git/geotiff/";

#[tokio::main]
async fn main() -> TiffResult<()> {
    env_logger::init();
    let mut m = BTreeMap::new();
    let mut prev = Instant::now();
    // adjust to use a dir on you
    for p in fs::read_dir(GEOTIFF_DIR.to_string() + "resources/data").unwrap() {
        let path = p.unwrap().path();
        let fname = path.file_name().unwrap().to_str().unwrap().to_owned();
        if !fname.ends_with("tiff") && !fname.ends_with("tif") {
            continue;
        }
        if fname == "nt_20201024_f18_nrt_s.tif" {
            continue;
        }
        println!("decoding {:64}", fname);

        let reader = TokioFile::new(path).expect("could not open file");
        let mut dc = Decoder::new(reader)
            .await
            .expect("could not create decoder");
        dc.scan_ifds().await?;
        dc.read_image_ifds().await?;
        match dc.decode_image(0).await {
            Ok(r) => img_print(r, &dc.get_overview(0)?.chunk_opts),
            Err(e) => error!("help! {e}"),
        }

        let n = Instant::now();
        m.insert(n - prev, fname.clone());
        info!("decoding {:?} cost {:?}", &m[&(n - prev)], n - prev);
        prev = n;
    }

    for (k, v) in m.iter().rev().take(10) {
        println!("{v:32} took {k:?}");
    }
    Ok(())
}

#[test]
fn test_tiffs() {
    for t in RGBTIFFS {
        println!("\n\n\n\n{}\n\n\n\n", t);
        match GeoTiff::read(
            File::open(&("resources/data/".to_owned() + t)).expect("could not parse path"),
        ) {
            Ok(x) => {
                println!("great success! {:?}", x);
            }
            Err(e) => {
                println!("Fail! {:?}", e);
                panic!("Failed at {:?} with: {:?}", t, e)
            }
        };
    }
    // assert!(false)
}
