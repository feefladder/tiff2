use std::ops::Range;

use async_trait::async_trait;
use bytes::Bytes;
use log::error;
#[cfg(test)]
use tokio::io::{AsyncReadExt, AsyncSeekExt};

use crate::decoder::CogReader;
use crate::error::TiffResult;

#[cfg(test)]
#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
impl CogReader for tokio::fs::File {
    const IFD_REQ_SIZE: u64 = 16 * 1024; // Example buffer size, can be adjusted

    async fn get_range<'a>(&'a self, range: Range<u64>) -> TiffResult<Bytes> {
        use crate::error::TiffError;
        // Seek to the start position of the range
        let mut file: tokio::fs::File = (*self).try_clone().await?;
        file.seek(tokio::io::SeekFrom::Start(range.start))
            .await
            .map_err(|e| TiffError::TransportError(Box::new(e)))?;

        // Calculate the range's length and allocate buffer
        let len = (range.end - range.start) as usize;
        let mut buffer = vec![0u8; len];

        // Read the data into the buffer
        file.read_exact(&mut buffer)
            .await
            .map_err(|e| TiffError::TransportError(Box::new(e)))?;

        // Return the data as `Bytes`
        Ok(Bytes::copy_from_slice(&buffer))
    }
}

#[cfg(not(feature = "object_store"))]
#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
impl CogReader for &[u8] {
    const IFD_REQ_SIZE: u64 = 16 * 1024;
    async fn get_range(&self, range: Range<u64>) -> TiffResult<Bytes> {
        if range.end >= self.len().try_into().unwrap() {
            error!("tried to get range {range:?} from {self:?}");
            // return Err(TiffError::LimitsExceeded);
        }
        let end = std::cmp::min(usize::try_from(range.end)?, self.len());
        Ok(Bytes::copy_from_slice(
            &self[usize::try_from(range.start)?..end],
        ))
    }
}

#[cfg(feature = "object_store")]
#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
impl<T: object_store::ObjectStore> CogReader for T {
    const IFD_REQ_SIZE: u64 = 16 * 1024;

    async fn get_range(&self, range: Range<u64>) -> TiffResult<Bytes> {
        self.get_range(
            &object_store::path::Path::from(""),
            range.start as usize..range.end as usize,
        )
        .await
        .map_err(|e| TiffError::TransportError(Box::new(e)))
    }
}
