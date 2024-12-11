use reqwest::header::RANGE;
use tiff2::{decoder::{CogReader, Decoder}, error::{TiffError, TiffResult}};
use log::{info, debug};
use std::time::{Duration, Instant};
use tokio;

#[derive(Debug, Clone)]
struct CogClient{
    client: reqwest::Client,
    url: String,
}

impl CogClient {
    fn new(url: &str) -> Self {
        CogClient { client: reqwest::Client::new(), url: url.to_string() }
    }
}

#[async_trait::async_trait]
impl CogReader for CogClient {
    const IFD_REQ_SIZE:u64 = 1024*64;

    async fn get_ranges(&self,ranges: &[std::ops::Range<u64>]) ->  TiffResult<Vec<bytes::Bytes> > {
        let mut v = Vec::with_capacity(ranges.len());
        for r in ranges {
            v.push(self.client.get(&self.url).header(RANGE, format!("bytes={}-{}",r.start,r.end-1)).send());
        }
        futures::future::join_all(
            futures::future::join_all(v)
                .await
                .into_iter()
                .collect::<Result<Vec<_>, _>>().map_err(|e| TiffError::TransportError(Box::new(e)))
                ?
                .into_iter()
                .map(|resp| resp.bytes())
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
    // let map = BTreeMap::new();
    
    // let href = "https://isdasoil.s3.amazonaws.com/soil_data/bulk_density/bulk_density.tif";//covariates/dem_30m/dem_30m.tif";
    let href = "https://zenodo.org/records/4087905/files/sol_db_od_m_30m_0..20cm_2001..2017_v0.13_wgs84.tif";
    // let href = "https://zenodo.org/records/4091154/files/sol_log.wpg2_m_30m_0..20cm_2001..2017_v0.13_wgs84.tif";

    // Step 1: Perform a HEAD request
    // let client = reqwest::Client::new();
    // let head_response = client.head(href).send().await.expect("could not head response");
    // println!("{:?}",head_response.headers());
    // Check if the server supports range requests
    // if head_response.headers().get(ACCEPT_RANGES).map(|v| v == "bytes").unwrap_or(false) {
    //     println!("Server supports range requests");
    // } else {
    //     eprintln!("Server does not support range requests");
    //     return Ok(());
    // }
    let cog_client = CogClient::new(&href);
    // Step 2: Make a range request for the first 16kB
    // let range_response = client
    //     .get(href)
    //     .header(RANGE, "bytes=5385046-5385062;")
    //     .send()
    //     .await
    //     .expect("could not range response");

    // // Check if the response is partial content (status 206)
    // if range_response.status().as_u16() != 206 && range_response.status().as_u16() != 200 {
    //     eprintln!("Failed to perform a range request. Status: {}", range_response.status());
    //     return Ok(());
    // }

    // // Get the data (first 16kB)
    // let data = range_response.bytes().await.expect("could not get bytes");
    // println!("Received first 16kB of the file: {:?} bytes", &data[..]);
    let start = Instant::now();
    let mut decoder = Decoder::new(cog_client).await?;
    // debug!("{:?}",decoder.reader.get_ranges(&[5385046..5385062]).await?);
    let t1 = start.elapsed();
    decoder.scan_ifds().await?;
    let t2 = start.elapsed();
    decoder.read_image_ifds().await?;
    let t3 = start.elapsed();
    info!("decoder: {:#?}", decoder.images);
    info!("initialization: {t1:?}, scan_ifds: {:?}, read_image_ifds: {:?}", t2-t1, t3-t2);
    // Step 3: Process the TIFF header with the `tiff2` crate
    // let mut decoder = Decoder::new(&data[..])?;
    // let header_info = decoder.read_header()?;
    // println!("TIFF Header Info: {:?}", header_info);

    Ok(())
}
