use async_trait::async_trait;
use log::{debug, info};
use reqwest::header::RANGE;
use std::time::{Duration, Instant};
use tiff2::{
    decoder::{CogReader, Decoder},
    error::{TiffError, TiffResult},
};
use tokio;

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

#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
impl CogReader for CogClient {
    const IFD_REQ_SIZE: u64 = 1024 * 64;

    async fn get_ranges(&self, ranges: &[std::ops::Range<u64>]) -> TiffResult<Vec<bytes::Bytes>> {
        let mut v = Vec::with_capacity(ranges.len());
        for r in ranges {
            v.push(
                self.client
                    .get(&self.url)
                    .header(RANGE, format!("bytes={}-{}", r.start, r.end - 1))
                    .send(),
            );
        }
        futures::future::join_all(
            futures::future::join_all(v)
                .await
                .into_iter()
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| TiffError::TransportError(Box::new(e)))?
                .into_iter()
                .map(|resp| resp.bytes()),
        )
        .await
        .into_iter()
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| TiffError::TransportError(Box::new(e))) //is there such a function?
    }
}

#[tokio::main]
async fn main() -> TiffResult<()> {
    env_logger::init();

    // a selection of different COGs. The ones on Zenodo don't all have their
    // IFDs tightly stacked

    // let href = "https://isdasoil.s3.amazonaws.com/soil_data/bulk_density/bulk_density.tif";//covariates/dem_30m/dem_30m.tif";
    let href = "https://zenodo.org/records/4087905/files/sol_db_od_m_30m_0..20cm_2001..2017_v0.13_wgs84.tif";
    // let href = "https://zenodo.org/records/4091154/files/sol_log.wpg2_m_30m_0..20cm_2001..2017_v0.13_wgs84.tif";

    let cog_client = CogClient::new(&href);

    let start = Instant::now();
    let mut decoder = Decoder::new(cog_client).await?;
    let t1 = start.elapsed();
    decoder.scan_ifds().await?;
    let t2 = start.elapsed();
    decoder.read_image_ifds().await?;
    let t3 = start.elapsed();
    info!("decoder: {:#?}", decoder.images);
    info!(
        "initialization: {t1:?}, scan_ifds: {:?}, read_image_ifds: {:?}",
        t2 - t1,
        t3 - t2
    );

    Ok(())
}
