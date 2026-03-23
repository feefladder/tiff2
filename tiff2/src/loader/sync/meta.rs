use bytes::Bytes;
use exn::{bail, ResultExt};

use crate::loader::{
    IfdReadError, IfdReadResult, IfdReader, MetaReadError, MetaReadResult, SyncFetch,
    SyncIfdReader, SyncMetaReader, TiffIfdReader, TiffLoadError, TiffLoader, TiffMetaReader,
};

impl<Fetch: SyncFetch, Loader: TiffLoader> SyncMetaReader<Fetch> for TiffMetaReader<Fetch, Loader> {
    fn open(fetch: Fetch, prefetch: u64) -> MetaReadResult<Self> {
        let loader = match Loader::from_header(
            fetch
                .fetch_range(0..prefetch)
                .or_raise(|| MetaReadError::fetch_error(format!("Could not create loader")))?,
        ) {
            Err(e) => match &*e {
                TiffLoadError::InvalidBuffer { required } => {
                    Loader::from_header(fetch.fetch_range(required.clone()).or_raise(|| {
                        MetaReadError::fetch_error(format!("Could not create loader on retry"))
                    })?)
                    .or_raise(|| MetaReadError("parsing header failed".into()))?
                }
                _ => bail!(e.raise(MetaReadError("parsing header failed".into()))),
            },
            Ok(l) => l,
        };
        Ok(Self { loader, fetch })
    }

    /// Load this ifd, ensuring all tags are loaded
    fn next(&mut self) -> MetaReadResult<Option<u64>> {
        let Some(offset) = self.loader.tiff().next_ifd_offset() else {
            return Ok(None);
        };
        // ah yes the problem here is that a cache will not necessarily need a buffer to create an IfdLoader, but a bare tiff will need one
        let mut buf = Bytes::new();
        // in the worst case, we need three times:
        // 1. empty buffer -> entry count size
        // 2. entry count size -> ifd size
        // 3. ifd size -> ok
        for _ in 0..3 {
            // buf is cheaply cloneable
            match self.loader.ifd_loader(buf.clone(), offset) {
                Err(e) => match &*e {
                    TiffLoadError::InvalidBuffer { required } => {
                        buf = self.fetch.fetch_range(required.clone()).or_raise(|| {
                            MetaReadError::fetch_error("could not load IFD buffer".into())
                        })?;
                    }
                    TiffLoadError::NeedMoreData {
                        ifd: _,
                        next_ifd_offset: _,
                        required,
                    } => {
                        for range in required {
                            self.loader.give_more_data(
                                range.start,
                                self.fetch.fetch_range(range.clone()).or_raise(|| {
                                    MetaReadError::fetch_error("Could not load data".into())
                                })?,
                            )
                        }
                    }
                    _ => bail!(e.raise(MetaReadError("could not load ifd".into()))),
                },
                Ok((ifd_loader, next_offset)) => {
                    let mut ifd_reader = TiffIfdReader::wrap(&mut self.fetch, ifd_loader);
                    // ensure all deferred values are loaded
                    ifd_reader
                        .fill_deferred()
                        .or_raise(|| MetaReadError(format!("could not finalize ifd {offset}")))?;
                    self.loader
                        .tiff_mut()
                        .insert_ifd(offset, ifd_reader.finish(), next_offset);
                    return Ok(Some(next_offset));
                }
            }
        }

        bail!(MetaReadError(
            "could not load ifd after 3 tries, non-recoverable".into()
        ))
    }

    /// "Skip" n ifds, loading them as incomplete
    fn skip(&mut self, n: usize) -> MetaReadResult<Option<u64>> {
        for _ in 0..n {
            let Some(offset) = self.loader.tiff().next_ifd_offset() else {
                return Ok(None);
            };
            let mut buf = Bytes::new();
            for _ in 0..3 {
                match self.loader.ifd_loader(buf.clone(), offset) {
                    Err(e) => {
                        if e.is_partial() {
                            let TiffLoadError::NeedMoreData {
                                ifd,
                                next_ifd_offset,
                                required: _,
                            } = e.into_error()
                            else {
                                unreachable!()
                            };
                            self.loader
                                .tiff_mut()
                                .insert_ifd(offset, ifd, next_ifd_offset);
                            break;
                        } else {
                            match &*e {
                                TiffLoadError::InvalidBuffer { required } => {
                                    buf = self.fetch.fetch_range(required.clone()).or_raise(
                                        || {
                                            MetaReadError::fetch_error(
                                                "could not get data to skip ifd".into(),
                                            )
                                        },
                                    )?;
                                }
                                _ => bail!(e.raise(MetaReadError(format!(
                                    "could not skip ifd {offset}, no next ifd known"
                                )))),
                            }
                        }
                    }
                    Ok((ifd_loader, next_ifd_offset)) => {
                        self.loader.tiff_mut().insert_ifd(
                            offset,
                            ifd_loader.finish(),
                            next_ifd_offset,
                        );
                        break;
                    }
                }
            }
        }
        Ok(self.loader.tiff().next_ifd_offset())
    }
}

impl<'a, Fetch: SyncFetch> SyncIfdReader<'a, Fetch> for TiffIfdReader<'a, Fetch> {
    fn fill_deferred(&mut self) -> IfdReadResult<()> {
        let ranges = self.ifd_loader.deferred_ranges().collect::<Vec<_>>();
        let data = self
            .fetch
            .fetch_ranges(&ranges)
            .or_raise(|| IfdReadError(format!("Could not fill deferred values of ifd")))?;
        let byte_order = self.ifd_loader.byte_order;
        for ((tag, entry), buf) in self.ifd_loader.deferred_values_mut().zip(data) {
            entry
                .to_value(&buf, byte_order)
                .or_raise(|| IfdReadError(format!("could nto read entry for tag {tag:?}")))?;
        }
        Ok(())
    }
}
