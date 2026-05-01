use async_trait::async_trait;
use bytes::Bytes;
use exn::{bail, OptionExt, ResultExt};
use rayon::prelude::*;

use crate::loader::tile::{CodingResult, TileLoadErrorKind};
use crate::loader::{
    AsyncFetch, AsyncIfdReader, AsyncReader, IfdLoader, IfdReader, ReadError, ReadErrorKind,
    ReadResult, TiffIfdReader, TiffReader, TileLoader,
};
use crate::structs::{TileCoord, TileData};

#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
impl<Fetch: AsyncFetch> AsyncReader for TiffReader<Fetch> {
    /// Get tiles for the corresponding coordinates
    ///
    /// will filter out invalid coordinates. Possible decoding errors are forwarded.
    ///
    /// # Errors
    ///
    /// ## Recoverable:
    ///
    async fn get_tiles(
        &self,
        ifd_idx: usize,
        coords: &[TileCoord],
    ) -> ReadResult<Vec<(TileCoord, CodingResult<TileData>)>> {
        let fatal = |message: String| {
            move || ReadError {
                kind: ReadErrorKind::Fatal,
                message,
            }
        };
        let ifd_offset = self
            .tiff
            .ifd_offsets
            .get(ifd_idx)
            .ok_or_raise(fatal(format!(
                "no ifd {ifd_idx}, tiff has {} public ifds",
                self.tiff.ifd_offsets.len()
            )))?;
        let no_loader = |ifd_offset: u64| {
            move || ReadError {
                message: format!("tile loader for ifd {ifd_offset} not loaded"),
                kind: ReadErrorKind::NoTileLoader { ifd_offset },
            }
        };
        let tile_loader = self
            .tile_loaders
            .get(ifd_offset)
            .ok_or_raise(no_loader(*ifd_offset))?;
        let mut out_coords = Vec::with_capacity(coords.len());
        let mut ranges = Vec::with_capacity(coords.len());
        for (coord, range) in tile_loader
            .tiles_ranges(coords.iter())
            .filter(|(_, r)| r.is_ok())
            .map(|(tc, r)| (tc, r.unwrap()))
        {
            out_coords.push(coord);
            ranges.push(range);
        }
        let compressed: Vec<Bytes> = self
            .fetch
            .fetch_ranges(&ranges)
            .await
            .or_raise(fatal(format!("could not fetch {ranges:?}")))?;
        let mut res = Vec::with_capacity(compressed.len());
        tile_loader
            .get_tiles(
                out_coords.into_par_iter().zip(compressed),
                &self.decoder_registry,
            )
            .collect_into_vec(&mut res);
        Ok(res)
    }

    async fn prep_ifd(&mut self, ifd_idx: usize) -> ReadResult<()> {
        let ifd_offset = self
            .tiff
            .ifd_offsets
            .get(ifd_idx)
            .ok_or_raise(|| ReadError::fatal(format!("no ifd {ifd_idx}")))?;
        let mut ifd = self
            .tiff
            .ifds
            .remove(ifd_offset)
            .ok_or_raise(|| ReadError::fatal(format!("ifd {ifd_offset} missing")))?;

        // try twice: 1st time may de deferred
        for n in 0..2 {
            if let Err(e) = TileLoader::check_ifd(&ifd) {
                if matches!(e.kind, TileLoadErrorKind::DeferredIfd) {
                    let mut ifd_reader = TiffIfdReader::wrap(
                        &self.fetch,
                        IfdLoader::wrap(ifd, self.tiff.bigtiff, self.tiff.byte_order, None),
                    );
                    if let Err(e) = ifd_reader.fill_deferred().await {
                        self.tiff.ifds.insert(*ifd_offset, ifd_reader.finish());
                        bail!(e.raise(ReadError::fatal(
                            "ifd prep failed: could not fill its values".into()
                        )))
                    }
                    ifd = ifd_reader.finish();
                } else {
                    self.tiff.ifds.insert(*ifd_offset, ifd);
                    bail!(e.raise(ReadError::fatal(format!("ifd not an image on {n}th try"))))
                }
            } else {
                break;
            }
        }
        self.tile_loaders.insert(
            *ifd_offset,
            TileLoader::from_ifd(ifd, *ifd_offset, self.tiff.byte_order).or_raise(|| {
                ReadError::fatal(
                    "Could not create TileLoader from ifd. This is a bug. please open an issue"
                        .into(),
                )
            })?,
        );
        Ok(())
    }
}
