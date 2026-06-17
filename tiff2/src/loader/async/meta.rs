use std::sync::Arc;

use async_trait::async_trait;
use bytes::Bytes;
use exn::{bail, ResultExt};

use crate::loader::metadata::{IfdLoadResponse, TiffLoadResponse};
use crate::loader::{
    AsyncFetch, AsyncIfdReader, AsyncMetaReader, IfdReadError, IfdReadResult, MetaReadError,
    MetaReadResult, TiffExtLoaderRegistry, TiffIfdReader, TiffLoader, TiffMetaReader,
};

#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
impl<Fetch: AsyncFetch, Loader: TiffLoader> AsyncMetaReader<Fetch, Loader>
    for TiffMetaReader<Fetch, Loader>
{
    async fn open(
        fetch: Fetch,
        prefetch: u64,
        extension_registry: Arc<TiffExtLoaderRegistry>,
    ) -> MetaReadResult<Self> {
        let mut buf = fetch
            .fetch_range(0..prefetch)
            .await
            .or_raise(|| MetaReadError::fetch_error("Could not get tiff prefetch".to_string()))?;
        for _ in 0..3 {
            match Loader::from_header(buf)
                .or_raise(|| MetaReadError("Could not open tiff".into()))?
            {
                TiffLoadResponse::NeedData(range) => {
                    buf = fetch.fetch_range(range).await.or_raise(|| {
                        MetaReadError::fetch_error(
                            "Could not get required buffer to open tiff".into(),
                        )
                    })?
                }
                TiffLoadResponse::Complete(loader) => {
                    return Ok(Self {
                        loader,
                        fetch,
                        extension_registry,
                    })
                }
            }
        }
        bail!(MetaReadError(
            "Could not open tiff after 3 tries, non-recoverable".into()
        ))
    }

    /// Load this ifd, ensuring all tags are loaded
    async fn next(&mut self) -> MetaReadResult<Option<u64>> {
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
            match self
                .loader
                .ifd_loader(buf.clone(), offset, self.extension_registry.clone())
                .or_raise(|| MetaReadError("Could not parse next ifd".into()))?
            {
                IfdLoadResponse::NeedData(range) => {
                    buf = self.fetch.fetch_range(range).await.or_raise(|| {
                        MetaReadError::fetch_error("Could not load IFD buffer".into())
                    })?;
                }
                IfdLoadResponse::Complete {
                    ifd,
                    next_ifd_offset,
                } => {
                    self.loader
                        .tiff_mut()
                        .insert_ifd(offset, next_ifd_offset, ifd);
                    return Ok(Some(next_ifd_offset));
                }
                IfdLoadResponse::Partial {
                    mut ifd_loader,
                    mut needed_data,
                } => {
                    for _ in 0..3 {
                        let datas = self.fetch.fetch_ranges(&needed_data).await.or_raise(|| {
                            MetaReadError::fetch_error(format!(
                                "Could not load requested ranges {needed_data:?}"
                            ))
                        })?;
                        match self
                            .loader
                            .resume_loader(needed_data, datas, ifd_loader)
                            .or_raise(|| {
                                MetaReadError("Error when resuming partial loading".to_string())
                            })? {
                            IfdLoadResponse::NeedData(r) => {
                                bail!(MetaReadError(format!(
                                    "Ifd dropped while resuming loading, range {r:?} requested"
                                )));
                            }
                            IfdLoadResponse::Partial {
                                ifd_loader: il,
                                needed_data: nd,
                            } => {
                                ifd_loader = il;
                                needed_data = nd;
                            }
                            IfdLoadResponse::Complete {
                                ifd,
                                next_ifd_offset,
                            } => {
                                self.loader
                                    .tiff_mut()
                                    .insert_ifd(offset, next_ifd_offset, ifd);
                                return Ok(Some(next_ifd_offset));
                            }
                        }
                    }
                }
            }
        }

        bail!(MetaReadError(
            "could not load ifd after 3 tries, non-recoverable".into()
        ))
    }

    /// "Skip" n ifds, loading them as incomplete
    async fn skip(&mut self, n: usize) -> MetaReadResult<Option<u64>> {
        for _ in 0..n {
            let Some(offset) = self.loader.tiff().next_ifd_offset() else {
                return Ok(None);
            };
            let mut buf = Bytes::new();
            for _ in 0..3 {
                match self
                    .loader
                    .ifd_loader(buf.clone(), offset, self.extension_registry.clone())
                    .or_raise(|| MetaReadError("Parse erorr when skipping ifd".to_string()))?
                {
                    IfdLoadResponse::NeedData(range) => {
                        buf = self.fetch.fetch_range(range.clone()).await.or_raise(|| {
                            MetaReadError::fetch_error("could not get data to skip ifd".into())
                        })?;
                    }
                    IfdLoadResponse::Partial {
                        ifd_loader,
                        needed_data: _,
                    } => {
                        self.loader.tiff_mut().insert_ifd(
                            offset,
                            ifd_loader.next_ifd_offset.unwrap(),
                            ifd_loader.finish(),
                        );
                        break;
                    }
                    IfdLoadResponse::Complete {
                        ifd,
                        next_ifd_offset,
                    } => {
                        self.loader
                            .tiff_mut()
                            .insert_ifd(offset, next_ifd_offset, ifd);
                        break;
                    }
                }
            }
        }
        Ok(self.loader.tiff().next_ifd_offset())
    }
}

#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
impl<'a, Fetch: AsyncFetch> AsyncIfdReader<'a, Fetch> for TiffIfdReader<'a, Fetch> {
    async fn fill_deferred(&mut self) -> IfdReadResult<()> {
        let ranges = self.ifd_loader.deferred_ranges().collect::<Vec<_>>();
        let data = self
            .fetch
            .fetch_ranges(&ranges)
            .await
            .or_raise(|| IfdReadError("Could not fill deferred values of ifd".to_string()))?;
        for (tag, buf) in self
            .ifd_loader
            .deferred_tags()
            .collect::<Vec<_>>()
            .into_iter()
            .zip(data)
        {
            self.ifd_loader
                .load_tag_data(&buf, tag)
                .or_raise(|| IfdReadError(format!("Failed to fill tag {tag:?} into ifd")))?;
        }
        Ok(())
    }
}
