use async_trait::async_trait;
use image::{DynamicImage, ImageBuffer, Luma};
use log::{error, info};
use reqwest::header::RANGE;
use std::time::{Duration, Instant};
use tiff2::{
    decoder::{CogReader, Decoder, TileData},
    error::{TiffError, TiffResult},
};
use tokio::time::sleep;
use viuer::ViuError;

#[derive(Debug, Clone)]
struct CogClient {
    client: reqwest::Client,
    url: String,
}

impl CogClient {
    fn new(url: &str) -> Self {
        CogClient {
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
) -> TiffResult<bytes::Bytes> {
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
                    .map_err(|e| TiffError::TransportError(Box::new(e)))
            }
            Ok(resp) if resp.status() == reqwest::StatusCode::TOO_MANY_REQUESTS => {
                if attempts >= max_retries {
                    error!("max retries reached");
                    return Err(TiffError::LimitsExceeded);
                }
                info!("retrying, waiting {:?}", &resp.headers().get("Retry-After"));
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
            Err(e) => return Err(TiffError::TransportError(Box::new(e))),
            _ => {
                // Handle other HTTP errors
                return Err(TiffError::LimitsExceeded);
            }
        }
    }
}

#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
impl CogReader for CogClient {
    const IFD_REQ_SIZE: u64 = 1024 * 64;

    async fn get_range(&self, range: std::ops::Range<u64>) -> TiffResult<bytes::Bytes> {
        fetch_with_retry(&self.client, &self.url, &range, 15).await
    }
}

#[tokio::main]
async fn main() -> TiffResult<()> {
    env_logger::init();

    // a selection of different COGs. The ones on Zenodo don't all have their
    // IFDs tightly stacked

    // let href = "https://isdasoil.s3.amazonaws.com/soil_data/bulk_density/bulk_density.tif";
    // let href = "https://isdasoil.s3.amazonaws.com/covariates/dem_30m/dem_30m.tif";
    // let href = "https://zenodo.org/records/4087905/files/sol_db_od_m_30m_0..20cm_2001..2017_v0.13_wgs84.tif";
    // let href = "https://zenodo.org/records/4091154/files/sol_log.wpg2_m_30m_0..20cm_2001..2017_v0.13_wgs84.tif";
    // let href = "https://service.pdok.nl/rws/ahn/atom/downloads/dtm_05m/M_01GN2.tif";
    // let href = "https://service.pdok.nl/rws/ahn/atom/downloads/dtm_05m/M_02DZ1.tif";
    // let href = "https://sentinel-cogs.s3.us-west-2.amazonaws.com/sentinel-s2-l2a-cogs/16/T/CR/2025/3/S2A_16TCR_20250322_0_L2A/B02.tif";
    let href = "https://ssh.datastations.nl/api/access/datafile/273106";
    // let cog_client = HttpBuilder::new().with_url(href).build().map_err(|e|TiffError::TransportError(Box::new(e)))?;
    let cog_client = CogClient::new(href);

    let start = Instant::now();
    let mut decoder = Decoder::new(cog_client).await?;
    let t1 = start.elapsed();
    decoder.scan_ifds().await?;
    let t2 = start.elapsed();
    decoder.read_image_ifds().await?;
    let t3 = start.elapsed();
    // info!("decoder: {:#?}", decoder.images);
    info!(
        "initialization: {t1:?}, scan_ifds: {:?}, read_image_ifds: {:?}",
        t2 - t1,
        t3 - t2
    );
    let ifd_idx = decoder.ifd_offsets().len() - 4;
    // let img = &decoder.images[&decoder.ifd_offsets()[ifd_idx]];
    let img_buf = decoder
        .decode_image(ifd_idx)
        .await
        .expect("could not decode image");
    let chopts = &decoder.images[&decoder.ifd_offsets()[ifd_idx]].chunk_opts;
    let _ = match img_buf {
        TileData::F32(v) => viuer::print(
            &DynamicImage::from(
                ImageBuffer::<Luma<_>, _>::from_raw(chopts.image_width, chopts.image_height, v)
                    .expect("could not create image"),
            ),
            &viuer::Config::default(),
        ),
        // TileData::F64(v) => viuer::print(&DynamicImage::from(ImageBuffer::<Luma<_>,_>::from_raw(chopts.image_width, chopts.image_height, v).expect("could not create image")), &viuer::Config::default()),
        // TileData::I8 (v)  => viuer::print(&DynamicImage::from(ImageBuffer::<Luma<_>,_>::from_raw(chopts.image_width, chopts.image_height, v).expect("could not create image")), &viuer::Config::default()),
        // TileData::I16(v) => viuer::print(&DynamicImage::from(ImageBuffer::<Luma<_>,_>::from_raw(chopts.image_width, chopts.image_height, v).expect("could not create image")), &viuer::Config::default()),
        // TileData::I32(v) => viuer::print(&DynamicImage::from(ImageBuffer::<Luma<_>,_>::from_raw(chopts.image_width, chopts.image_height, v).expect("could not create image")), &viuer::Config::default()),
        // TileData::I64(v) => viuer::print(&DynamicImage::from(ImageBuffer::<Luma<_>,_>::from_raw(chopts.image_width, chopts.image_height, v).expect("could not create image")), &viuer::Config::default()),
        TileData::U8(v) => viuer::print(
            &DynamicImage::from(
                ImageBuffer::<Luma<_>, _>::from_raw(chopts.image_width, chopts.image_height, v)
                    .expect("could not create image"),
            ),
            &viuer::Config::default(),
        ),
        TileData::U16(v) => viuer::print(
            &DynamicImage::from(
                ImageBuffer::<Luma<_>, _>::from_raw(chopts.image_width, chopts.image_height, v)
                    .expect("could not create image"),
            ),
            &viuer::Config::default(),
        ),
        // TileData::U32(v) => viuer::print(&DynamicImage::from(ImageBuffer::<Luma<_>,_>::from_raw(chopts.image_width, chopts.image_height, v).expect("could not create image")), &viuer::Config::default()),
        // TileData::U64(v) => viuer::print(&DynamicImage::from(ImageBuffer::<Luma<_>,_>::from_raw(chopts.image_width, chopts.image_height, v).expect("could not create image")), &viuer::Config::default()),
        _ => Err(ViuError::KittyNotSupported),
    };

    info!(
        "{:?}, {:?}, {:?}",
        chopts.photometric_interpretation, chopts.sample_format, chopts.bits_per_sample
    );
    Ok(())
}
