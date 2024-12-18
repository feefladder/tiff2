use async_trait::async_trait;
use bytes::Bytes;
use log::error;
use std::ops::Range;
use std::vec::Vec;
#[cfg(test)]
use tokio::{
    fs::File,
    io::{AsyncReadExt, AsyncSeekExt},
};

use crate::{
    decoder::CogReader,
    error::{TiffError, TiffResult},
};

// #[cfg(test)]
// #[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
// impl CogReader for tokio::fs::File {
//     const IFD_REQ_SIZE: u64 = 16 * 1024;  // Example buffer size, can be adjusted

//     async fn get_ranges<'a>(&'a self, ranges: &[Range<u64>]) -> TiffResult<Vec<Bytes>> {
//         let mut tasks = Vec::with_capacity(ranges.len());

//         for range in ranges {
//             // Open a new file handle for each range, allowing for parallelism
//             let file_clone = self.clone(); // File handles can be cloned

//             let task = tokio::spawn(async move {
//                 // Seek to the start position of the range
//                 let mut file = file_clone;
//                 file.seek(tokio::io::SeekFrom::Start(range.start))
//                     .await
//                     .map_err(|e| TiffError::TransportError(Box::new(e)))?;

//                 // Calculate the range's length and allocate buffer
//                 let len = (range.end - range.start) as usize;
//                 let mut buffer = vec![0u8; len];

//                 // Read the data into the buffer
//                 file.read_exact(&mut buffer)
//                     .await
//                     .map_err(|e| TiffError::TransportError(Box::new(e)))?;

//                 // Return the data as `Bytes`
//                 Ok(Bytes::copy_from_slice(&buffer))
//             });

//             tasks.push(task);
//         }

//         // Wait for all tasks to complete
//         let results = futures::future::join_all(tasks).await;

//         // Handle the results, map errors, and return the final vector of Bytes
//         results.into_iter().collect::<Result<Vec<_>, _>>()
//             .map_err(|e| TiffError::TransportError(Box::new(e)))?
//             .into_iter().collect::<Result<Vec<Bytes>, _>>()
//     }
// }
#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
impl CogReader for &[u8] {
    const IFD_REQ_SIZE: u64 = 16 * 1024;
    async fn get_ranges(&self, ranges: &[Range<u64>]) -> TiffResult<Vec<Bytes>> {
        let mut res = Vec::with_capacity(ranges.len());
        for range in ranges {
            if range.end >= self.len().try_into().unwrap() {
                error!("tried to get range {range:?} from {self:?}");
                return Err(TiffError::LimitsExceeded);
            }
            let end = std::cmp::min(usize::try_from(range.end)?, self.len());
            res.push(Bytes::copy_from_slice(
                &self[usize::try_from(range.start)?..end],
            ));
        }
        Ok(res)
    }
}
