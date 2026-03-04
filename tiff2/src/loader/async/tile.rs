use async_trait::async_trait;
use bytes::Bytes;
use exn::{bail, ensure, OptionExt, ResultExt};
use rayon::prelude::*;

use crate::loader::tile::{CodingResult, TileLoadErrorKind};
use crate::loader::{
    AsyncFetch, AsyncReader, ReadError, ReadErrorKind, ReadResult, TiffReader, TileLoader,
};
use crate::structs::{IfdEntry, TagData, TileCoord, TileData};

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
        ensure!(
            self.tiff.ifds.contains_key(ifd_offset),
            ReadError::fatal(format!("ifd {ifd_offset} missing"))
        );
        if let Err(e) = TileLoader::check_ifd(self.tiff.ifd(ifd_idx)) {
            if matches!(e.kind, TileLoadErrorKind::DeferredIfd) {
                for (_, entry) in self
                    .tiff
                    .ifds
                    .get_mut(&self.tiff.ifd_offsets[ifd_idx])
                    .unwrap()
                    .data
                    .iter_mut()
                    .filter(|(_, e)| matches!(e, IfdEntry::Offset(_)))
                {
                    let IfdEntry::Offset(o) = entry else {
                        unreachable!()
                    };
                    *entry = IfdEntry::Value(
                        TagData::from_buffer(
                            &self.fetch.fetch_range(o.range()).await.or_raise(|| {
                                ReadError::fetch_error(format!("Could not improve ifd {ifd_idx}"))
                            })?,
                            o.tag_type,
                            o.count.try_into().unwrap(),
                            self.tiff.byte_order,
                        )
                        .or_raise(|| ReadError::fatal(format!("invalid tag data")))?,
                    )
                }
                TileLoader::check_ifd(self.tiff.ifd(ifd_idx))
                    .or_raise(|| ReadError::fatal("ifd not an image".into()))?;
            } else {
                bail!(e.raise(ReadError::fatal("ifd not an image".into())))
            }
        }
        self.tile_loaders.insert(
            self.tiff.ifd_offsets[ifd_idx],
            TileLoader::from_ifd(
                self.tiff.ifds.remove(ifd_offset).unwrap(),
                *ifd_offset,
                self.tiff.byte_order,
            )
            .or_raise(|| ReadError::fatal("this is really bad".into()))?,
        );
        Ok(())
    }
}
