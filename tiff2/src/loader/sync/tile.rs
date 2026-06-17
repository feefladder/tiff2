use std::sync::Arc;

use bytes::Bytes;
use exn::{bail, OptionExt, ResultExt};
use rayon::prelude::*;

use crate::loader::tile::{CodingResult, TileLoadErrorKind};
use crate::loader::{
    IfdLoader, IfdReader, ReadError, ReadErrorKind, ReadResult, SyncFetch, SyncIfdReader,
    SyncReader, TiffIfdReader, TiffLoader, TiffReader, TileLoader,
};
use crate::structs::{TileCoord, TileData};

impl<Fetch: SyncFetch, Loader: TiffLoader> SyncReader for TiffReader<Fetch, Loader> {
    /// Get tiles for the corresponding coordinates
    ///
    /// will filter out invalid coordinates. Possible decoding errors are forwarded.
    ///
    /// # Errors
    ///
    /// ## Recoverable:
    ///
    fn get_tiles(
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
            .loader
            .tiff()
            .ifd_offsets
            .get(ifd_idx)
            .ok_or_raise(fatal(format!(
                "no ifd {ifd_idx}, tiff has {} public ifds",
                self.loader.tiff().ifd_offsets.len()
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

    fn prep_ifd(&mut self, ifd_idx: usize) -> ReadResult<()> {
        let ifd_offset = self
            .loader
            .tiff()
            .ifd_offsets
            .get(ifd_idx)
            .ok_or_raise(|| ReadError::fatal(format!("no ifd {ifd_idx}")))?;
        let mut ifd = self
            .loader
            .tiff()
            .ifds
            .remove(ifd_offset)
            .ok_or_raise(|| ReadError::fatal(format!("ifd {ifd_offset} missing")))?;

        // try twice: 1st time may de deferred
        for n in 0..2 {
            if let Err(e) = TileLoader::check_ifd(&ifd) {
                if matches!(e.kind, TileLoadErrorKind::DeferredIfd) {
                    let mut ifd_reader = TiffIfdReader::wrap(
                        &self.fetch,
                        IfdLoader::wrap(
                            ifd,
                            self.loader.tiff().bigtiff,
                            self.loader.tiff().byte_order,
                            None,
                            // We're recovering from an error here, if you have
                            // unloaded extensions at this point, that's kind of
                            // your problem.
                            // TODO: should we have the registry in TiffReader?
                            Arc::new(Vec::new().into()),
                        ),
                    );
                    if let Err(e) = ifd_reader.fill_deferred() {
                        self.loader
                            .tiff()
                            .ifds
                            .insert(*ifd_offset, ifd_reader.finish());
                        bail!(e.raise(ReadError::fatal(
                            "ifd prep failed: could not fill its values".into()
                        )))
                    }
                    ifd = ifd_reader.finish();
                } else {
                    self.loader.tiff().ifds.insert(*ifd_offset, ifd);
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
