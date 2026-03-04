use std::collections::BTreeMap;

use bytes::Bytes;
use exn::{bail, ResultExt};

use crate::loader::tile::DecoderRegistry;
use crate::loader::{
    MetaReadError, MetaReadResult, SyncFetch, SyncMetaReader, SyncReader, TiffLoadError,
    TiffLoader, TiffMetaReader, TiffReader,
};
use crate::structs::{IfdEntry, TagData};

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
                Ok((mut ifd_loader, next_offset)) => {
                    // ensure all deferred values are loaded
                    for (tag, entry) in ifd_loader.deferred_values_mut() {
                        let IfdEntry::Offset(o) = entry else {
                            unreachable!()
                        };
                        *entry = IfdEntry::Value(
                            TagData::from_buffer(
                                &self.fetch.fetch_range(o.range().clone()).or_raise(|| {
                                    MetaReadError::fetch_error(format!(
                                        "could not load data for {tag:?}"
                                    ))
                                })?,
                                o.tag_type,
                                usize::try_from(o.count).unwrap(),
                                self.loader.tiff().byte_order(),
                            )
                            .unwrap(),
                        );
                    }
                    self.loader
                        .tiff_mut()
                        .insert_ifd(offset, ifd_loader.finish(), next_offset);
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
