use std::{
    error::Error,
    ops::Range,
    time::{Duration, Instant},
};

use async_trait::async_trait;
use bytes::Bytes;
use derive_more::Display;
use exn::{bail, ensure, ErrorExt, OptionExt, ResultExt};
use reqwest::header::RANGE;
use tiff2::loader::{
    cache::CogCache, AsyncFetch, AsyncMetaReader, AsyncReader, DecoderRegistry, FetchError,
    FetchResult, TiffMetaReader,
};
use tokio::time::sleep;

#[derive(Debug, Clone)]
struct ReqwestFetch {
    client: reqwest::Client,
    url: String,
}

impl ReqwestFetch {
    fn new(url: &str) -> Self {
        ReqwestFetch {
            client: reqwest::Client::new(),
            url: url.to_string(),
        }
    }
}

async fn fetch_with_retry(
    client: &reqwest::Client,
    url: &str,
    range: &std::ops::Range<u64>,
    max_retries: usize,
) -> FetchResult<bytes::Bytes> {
    let mut attempts = 0;

    loop {
        let response = client
            .get(url)
            .header(RANGE, format!("bytes={}-{}", range.start, range.end - 1))
            .send()
            .await;

        match response {
            Ok(resp) if resp.status().is_success() => {
                return resp
                    .bytes()
                    .await
                    .or_raise(|| FetchError("Invalid response".into()))
            }
            Ok(resp) if resp.status() == reqwest::StatusCode::TOO_MANY_REQUESTS => {
                ensure!(
                    attempts > max_retries,
                    FetchError(format!("request limit {max_retries} exceeded"))
                );
                // Exponential backoff
                let retry_after = resp
                    .headers()
                    .get("Retry-After")
                    .and_then(|h| h.to_str().ok())
                    .and_then(|s| s.parse::<u64>().ok())
                    .map(Duration::from_secs)
                    .unwrap_or_else(|| Duration::from_secs(1 << attempts));

                attempts += 1;
                sleep(retry_after).await;
            }
            Err(e) => bail!(e.raise().raise(FetchError("Transport error".into()))),
            Ok(r) => {
                // Handle other HTTP errors
                bail!(r
                    .error_for_status()
                    .unwrap_err()
                    .raise()
                    .raise(FetchError(format!("Http error on attempt {attempts}"))))
            }
        }
    }
}

#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
impl AsyncFetch for ReqwestFetch {
    async fn fetch_range(&self, range: Range<u64>) -> FetchResult<Bytes> {
        println!("fetching {range:?}");
        fetch_with_retry(&self.client, &self.url, &range, 10).await
    }
}

#[derive(Debug, Display, Clone, PartialEq)]
#[display("app failed")]
struct AppError;
impl Error for AppError {}

#[tokio::main]
async fn main() -> Result<(), exn::Exn<AppError>> {
    // a selection of different COGs. The ones on Zenodo don't all have their
    // IFDs tightly stacked

    let href = "https://isdasoil.s3.amazonaws.com/soil_data/bulk_density/bulk_density.tif";
    // let href = "https://isdasoil.s3.amazonaws.com/covariates/dem_30m/dem_30m.tif";
    // let href = "https://zenodo.org/records/4087905/files/sol_db_od_m_30m_0..20cm_2001..2017_v0.13_wgs84.tif";
    // let href = "https://zenodo.org/records/4091154/files/sol_log.wpg2_m_30m_0..20cm_2001..2017_v0.13_wgs84.tif";
    // let href = "https://service.pdok.nl/rws/ahn/atom/downloads/dtm_05m/M_01GN2.tif";
    // let href = "https://service.pdok.nl/rws/ahn/atom/downloads/dtm_05m/M_02DZ1.tif";
    // let href = "https://sentinel-cogs.s3.us-west-2.amazonaws.com/sentinel-s2-l2a-cogs/16/T/CR/2025/3/S2A_16TCR_20250322_0_L2A/B02.tif";
    // let href = "https://ssh.datastations.nl/api/access/datafile/273106";
    // let cog_client = HttpBuilder::new().with_url(href).build().map_err(|e|TiffError::TransportError(Box::new(e)))?;
    let cog_client = ReqwestFetch::new(href);

    let start = Instant::now();
    let mut meta_reader: TiffMetaReader<ReqwestFetch, CogCache> =
        AsyncMetaReader::open(cog_client, 16 * 1024)
            .await
            .or_raise(|| AppError)?;
    meta_reader.next().await;
    let t1 = start.elapsed();
    while meta_reader.next().await.unwrap().is_some() {}
    // meta_reader.skip(42).await.or_raise(|| AppError)?;
    // while meta_reader.next().await.or_raise(|| AppError)?.is_some() {}
    let t2 = start.elapsed();
    let mut reader = meta_reader.finish(DecoderRegistry::default());
    let ifd_idx = reader.tiff().len() - 2;
    reader.prep_ifd(ifd_idx).await.or_raise(|| AppError)?;
    let t3 = start.elapsed();
    // info!("decoder: {:#?}", decoder.images);
    println!(
        "initialization: {t1:?}, scan_ifds: {:?}, read_image_ifds: {:?}",
        t2 - t1,
        t3 - t2
    );

    let topts = reader.tile_opts(ifd_idx).ok_or_raise(|| AppError)?;
    let max_x = topts.chunks_across();
    let max_y = topts.chunks_down();
    let mut tile_coords = Vec::with_capacity(max_x as usize * max_y as usize);
    for x in 0..max_x {
        for y in 0..max_y {
            tile_coords.push((x, y).into());
        }
    }
    let _tiles = reader.get_tiles(ifd_idx, &tile_coords).await;
    // let img = &decoder.images[&decoder.ifd_offsets()[ifd_idx]];
    // let img_buf = reader
    //     .decode_image(ifd_idx)
    //     .await
    //     .expect("could not decode image");
    // let chopts = &decoder.images[&decoder.ifd_offsets()[ifd_idx]].tile_opts;
    // let _ = match img_buf {
    //     TileData::F32(v) => viuer::print(
    //         &DynamicImage::from(
    //             ImageBuffer::<Luma<_>, _>::from_raw(chopts.image_width, chopts.image_height, v)
    //                 .expect("could not create image"),
    //         ),
    //         &viuer::Config::default(),
    //     ),
    //     // TileData::F64(v) => viuer::print(&DynamicImage::from(ImageBuffer::<Luma<_>,_>::from_raw(chopts.image_width, chopts.image_height, v).expect("could not create image")), &viuer::Config::default()),
    //     // TileData::I8 (v)  => viuer::print(&DynamicImage::from(ImageBuffer::<Luma<_>,_>::from_raw(chopts.image_width, chopts.image_height, v).expect("could not create image")), &viuer::Config::default()),
    //     // TileData::I16(v) => viuer::print(&DynamicImage::from(ImageBuffer::<Luma<_>,_>::from_raw(chopts.image_width, chopts.image_height, v).expect("could not create image")), &viuer::Config::default()),
    //     // TileData::I32(v) => viuer::print(&DynamicImage::from(ImageBuffer::<Luma<_>,_>::from_raw(chopts.image_width, chopts.image_height, v).expect("could not create image")), &viuer::Config::default()),
    //     // TileData::I64(v) => viuer::print(&DynamicImage::from(ImageBuffer::<Luma<_>,_>::from_raw(chopts.image_width, chopts.image_height, v).expect("could not create image")), &viuer::Config::default()),
    //     TileData::U8(v) => viuer::print(
    //         &DynamicImage::from(
    //             ImageBuffer::<Luma<_>, _>::from_raw(chopts.image_width, chopts.image_height, v)
    //                 .expect("could not create image"),
    //         ),
    //         &viuer::Config::default(),
    //     ),
    //     TileData::U16(v) => viuer::print(
    //         &DynamicImage::from(
    //             ImageBuffer::<Luma<_>, _>::from_raw(chopts.image_width, chopts.image_height, v)
    //                 .expect("could not create image"),
    //         ),
    //         &viuer::Config::default(),
    //     ),
    //     // TileData::U32(v) => viuer::print(&DynamicImage::from(ImageBuffer::<Luma<_>,_>::from_raw(chopts.image_width, chopts.image_height, v).expect("could not create image")), &viuer::Config::default()),
    //     // TileData::U64(v) => viuer::print(&DynamicImage::from(ImageBuffer::<Luma<_>,_>::from_raw(chopts.image_width, chopts.image_height, v).expect("could not create image")), &viuer::Config::default()),
    //     _ => Err(ViuError::KittyNotSupported),
    // };

    // println!(
    //     "{:?}, {:?}, {:?}",
    //     chopts.photometric_interpretation, chopts.sample_format, chopts.bits_per_sample
    // );
    Ok(())
}
