//! Cog caching middleware
//!
//! Also to see if we can expose the iterator-like functionality

use std::collections::BTreeMap;
use std::ops::{Bound, Range, RangeBounds};

use bytes::Bytes;
use exn::{Result, ResultExt};

use crate::loader::metadata::error::TiffLoadError;
use crate::loader::metadata::{
    IfdLoadResponse, Tiff, TiffLoadResponse, TiffLoadResult, TiffLoader,
};
use crate::structs::{IfdEntry, TagData};

pub struct CogCache {
    cache: BTreeMap<usize, Bytes>,
    tiff: Tiff,
}

impl TiffLoader for CogCache {
    fn tiff(&self) -> &Tiff {
        &self.tiff
    }
    fn tiff_mut(&mut self) -> &mut Tiff {
        &mut self.tiff
    }
    fn into_tiff(self) -> Tiff {
        self.tiff
    }
    /// Create a new CogCache with a prefetch buffer
    fn from_header(buf: Bytes) -> TiffLoadResult<TiffLoadResponse<Self>> {
        let tiff_repsonse = Tiff::from_header(buf.clone())
            .or_raise(|| TiffLoadError::permanent("could not load tiff".into()))?;
        match tiff_repsonse {
            TiffLoadResponse::NeedData(d) => Ok(TiffLoadResponse::NeedData(d)),
            TiffLoadResponse::Complete(tiff) => Ok(TiffLoadResponse::Complete(Self {
                cache: BTreeMap::from([(0, buf)]),
                tiff,
            })),
        }
    }

    fn ifd_loader(&mut self, buf: Bytes, offset: u64) -> Result<IfdLoadResponse, TiffLoadError> {
        let resp = self
            .tiff
            .ifd_loader(
                self.slice(offset..).unwrap_or(buf),
                // transparently raise the required range for the ifd
                offset,
            )
            .or_raise(|| TiffLoadError::permanent("Could not parse tiff".into()))?;
        match resp {
            // in case there is not enough data to read the ifd, we tell upper layers to retry with an updated range
            IfdLoadResponse::NeedData(range) => return Ok(IfdLoadResponse::NeedData(range)),
            // if it's done, there's nothing for us to do
            IfdLoadResponse::Complete(ifd, next_ifd_offset) => {
                return Ok(IfdLoadResponse::Complete(ifd, next_ifd_offset))
            }
            // If some tags are not loaded, try to get them from the cache
            IfdLoadResponse::Partial {
                mut ifd_loader,
                next_ifd_offset,
                needed_data,
            } => {
                let mut found_data = Vec::new();
                let mut found_ranges = Vec::new();
                let mut missing_ranges = Vec::new();
                // try to get all needed data from the cache and pass it down
                for range in needed_data {
                    if let Some(data) = self.slice(range.clone()) {
                        found_data.push(data);
                        found_ranges.push(range);
                    } else {
                        missing_ranges.push(range);
                    }
                }
                self.tiff.give_more_data(found_ranges, found_data);
                // in the happy path, everything is loaded, so no allocations there
                let mut deferred_tags = Vec::new();

                // try to get all values from the cache
                for (t, entry) in ifd_loader.deferred_values_mut() {
                    let IfdEntry::Offset(o) = entry else {
                        unreachable!()
                    };
                    if let Some(data) = self.slice(o.range().clone()) {
                        *entry = IfdEntry::Value(
                            TagData::from_buffer(
                                &data,
                                o.tag_type,
                                o.count as usize,
                                self.tiff.byte_order,
                            )
                            .unwrap(),
                        )
                    } else {
                        // insert magic caching/filtering strategies here
                        // For now, we just error at the end with all deferred values
                        missing_ranges.push(o.range());
                        deferred_tags.push(*t);
                    }
                }
                // FIXME: this is bad on skipping behaviour, because we can't recover from the error
                if !missing_ranges.is_empty() {
                    Ok(IfdLoadResponse::Partial {
                        needed_data: missing_ranges,
                        ifd_loader,
                        next_ifd_offset,
                    })
                } else {
                    Ok(IfdLoadResponse::Complete(
                        ifd_loader.finish(),
                        next_ifd_offset,
                    ))
                }
            }
        }
    }

    /// Insert more cache at the given offset
    ///
    /// Note that inserting a lot of small caches will decrease performance
    fn give_more_data(&mut self, ranges: Vec<Range<u64>>, data: Vec<Bytes>) {
        // TODO: this can overflow and lead to unpredictable behaviour
        for (range, buf) in ranges.iter().zip(data) {
            self.cache.insert(range.start as usize, buf);
        }
    }
}

impl CogCache {
    /// Load the next ifd
    ///
    /// If all the ifd's values are not present in the cache, an error is
    /// returned. This error may be ignored if this ifd is not of interest

    // so the sad thing here is that we can't create a BytesMut from a &mut
    // self, since cloning will increase the refcount, so this is always a
    // deep clone:
    //
    // let mut new = BytesMut::from(self.cache.clone());
    //
    // So I think we need a fancy cache, otherwise there's something like
    // https://github.com/developmentseed/async-tiff/blob/538196d9a7b8988f1ae1fa2c8659abf3ba8e7c07/src/metadata/cache.rs#L41
    // with broken cache hits and such and I think that can be avoided
    // altogether under these invariants:
    //
    // cache start always points to a determinate location (ifd start, tagdata start) and is long enough to read that data
    //
    // cache end is estimated from some heuristic so if there's a
    // `BTreeMap<usize,Bytes>`, and we want to insert a new cache block, but
    // the previous one overlaps, then we ~~shrink the previous blocks' range~~.
    // There's no need to shrink the range, since the allocation won't get
    // dropped anyways
    //

    /// Get a slice from the cache
    ///
    /// This searches back-to-front, so `..=42` will give the smallest slice that contains `42`
    fn slice(&self, range: impl RangeBounds<u64>) -> Option<Bytes> {
        let start = range.start_bound().map(|s| usize::try_from(*s).unwrap());
        let end = range.end_bound().map(|e| usize::try_from(*e).unwrap());
        self.cache
            // make a range from 0..start
            .range((Bound::Included(0), start))
            // find the last chunk that has the bounds
            .rfind(|(section_start, bytes)| match end {
                Bound::Unbounded => true,
                // the normal case
                //    |----|  req
                //         v
                //   |--len--| section
                // sstart    >=
                Bound::Excluded(v) => bytes.len() + **section_start >= v,
                // the special case
                //    |----|  req
                //         v
                //   |--len--| section
                // sstart    >
                Bound::Included(v) => bytes.len() + **section_start > v,
            })
            // So...
            .map(|(section_start, bytes)| {
                bytes.slice((
                    start.map(|s| s - section_start),
                    end.map(|e| e - section_start),
                ))
            })
    }

    /// Finish metadata parsing and remove the cache
    ///
    /// Consumes `Self`, so drops it
    pub fn finish(self) -> Tiff {
        self.tiff
    }
}

#[cfg(test)]
mod test {
    use std::collections::BTreeMap;

    use super::*;

    #[test]
    fn test_too_fancy_cache() {
        // This is a cache that is actually too fancy, it does checks/modifications that don't improve performance/memory use and only fragment the cache
        /// Get a slice from the cache
        ///
        /// This actually searches back-to-front, so `..=42` will give the smallest slice that contains `42`
        fn slice(
            cache: &BTreeMap<usize, Bytes>,
            range: impl RangeBounds<u64>,
        ) -> exn::Result<Bytes, CacheMiss> {
            let start = range.start_bound().map(|s| usize::try_from(*s).unwrap());
            let end = range.end_bound().map(|e| usize::try_from(*e).unwrap());
            cache
                // make a range from 0..start
                .range((Bound::Included(0), start))
                // find the last chunk that has the bounds
                .rfind(|(section_start, bytes)| match end {
                    Bound::Unbounded => true,
                    // the normal case
                    //    |----|  req
                    //         v
                    //   |--len--| section
                    // sstart    >=
                    Bound::Excluded(v) => bytes.len() + **section_start >= v,
                    // the special case
                    //    |----|  req
                    //         v
                    //   |--len--| section
                    // sstart    >
                    Bound::Included(v) => bytes.len() + **section_start > v,
                })
                // So...
                .map(|(section_start, bytes)| {
                    bytes.slice((
                        start.map(|s| s - section_start),
                        end.map(|e| e - section_start),
                    ))
                })
                .ok_or_raise(|| CacheMiss(start, end))
        }
        let mut cache = BTreeMap::new();
        cache.insert(0, Bytes::copy_from_slice(&[42; 42]));
        let desired_range = 10..32;
        assert_eq!(
            &slice(&cache, desired_range.clone()).unwrap()[..],
            &vec![42; (10..32).len()]
        );

        /// This is the part where it's too fancy now there's the completely useless edge-case of adding a buffer that breaks everything, because we don't support broken slices
        fn insert(cache: &mut BTreeMap<usize, Bytes>, offset: usize, mut data: Bytes) {
            // if there was a previous blob, truncate it (not sure if that is really needed though, but it is nice)
            // ```text
            // |------|      old
            //      |------| new
            // |---||------| combined
            // ```
            if let Some((prev_start, prev_bytes)) = cache.range_mut(0..offset).last() {
                if prev_bytes.len() + prev_start > offset {
                    // this actually keeps the entire previous allocation around, just inaccessible
                    *prev_bytes = prev_bytes.slice(..offset - prev_start)
                }
            }
            // if there is an overlapping blob, truncate self (also not sure if that is needed)
            // ```text
            //        |------|  old
            // |--------|       new
            // |-----||------| combined
            // ```
            if let Some((next_start, _)) = cache.range(offset..offset + data.len()).next() {
                // this actually keeps the entire previous allocation around, just inaccessible
                data = data.slice(offset..*next_start)
            }
            // There is also the case of new completely overlapping old, but let's not think about that for now
            //
            // ```text
            //    |--|  old
            // |------| new
            // |-||--|  combined (so little efficient...)
            // ```
            //
            // and the case where the offset is already present in the cache, so we'll just panic for now, lol
            assert!(cache.insert(offset, data).is_none());
        }
        insert(&mut cache, 13, Bytes::copy_from_slice(&[43; 43]));
        // now we broke the cache for this request...
        assert_eq!(
            slice(&cache, desired_range.clone())
                .unwrap_err()
                .frame()
                .error()
                .downcast_ref::<CacheMiss>()
                .unwrap(),
            &CacheMiss(
                desired_range
                    .start_bound()
                    .map(|s| usize::try_from(*s).unwrap()),
                desired_range
                    .end_bound()
                    .map(|e| usize::try_from(*e).unwrap())
            )
        );
        assert_eq!(
            cache,
            BTreeMap::from([
                (0, Bytes::copy_from_slice(&[42; 13])),
                (13, Bytes::copy_from_slice(&[43; 43]))
            ])
        );
        insert(&mut cache, 5, Bytes::copy_from_slice(&[44; 44]));
        assert_eq!(
            cache,
            BTreeMap::from([
                (0, Bytes::copy_from_slice(&[42; 5])),
                (5, Bytes::copy_from_slice(&[44; 8])),
                (13, Bytes::copy_from_slice(&[43; 43]))
            ])
        );
    }

    #[rustfmt::skip]
    fn circular_tiff() -> Bytes {
        Bytes::from_owner([
        //    0     1    2  3
            b'I', b'I',
            42, 0,// header
        //  4 5 6 7
            8,0,0,0,       // first ifd offset, u32
        //  8 9
            0,0,           // first ifd entry count, u16
        //   A B C D
            14,0,0,0,       // next ifd offset, u32
            0,0,            // second ifd entry count, u16
            8,0,0,0         // next ifd offset (points to 0)
        ])
    }

    // #[tokio::test]
    // async fn test_async_copy() {
    //     // for now everything is infallible
    //     #[async_trait::async_trait]
    //     trait AsyncRead {
    //         async fn read_range(&self, range: Range<u64>) -> Bytes;
    //         // yeah so I guess this is kind of sad, because we could coalesce_ranges downstream,
    //         // so I guess just a &[Range<u64>]->&[Bytes] should suffice?
    //         async fn read_ranges(&self, ranges: &[Range<u64>]) -> impl Iterator<Item = Bytes>;
    //     }
    //     async fn async_copy(reader: impl AsyncRead) -> TiffLoadResult<Tiff> {
    //         let prefetch = reader.read_range(0..1024 * 16).await;
    //         let mut writer = CogCache::new(prefetch).unwrap();
    //         while match writer.next() {
    //             // ok to unwrap the downcast_ref because of `writer.next()` return type
    //             Err(e) => match e.frame().error().downcast_ref::<TiffLoadError>().unwrap() {
    //                 TiffLoadError {
    //                     status: ErrorStatus::MissingRanges { required },
    //                     kind: TiffLoadErrorKind::NeedMoreData,
    //                     message: _,
    //                 } => {
    //                     for range in required {
    //                         writer.insert(
    //                             usize::try_from(range.start).unwrap(),
    //                             reader.read_range(range.clone()).await,
    //                         )
    //                     }
    //                     true
    //                     // so the idea is here that calling `next` which errors doesn't advance the iterator, so calling "next" again will retry
    //                     // except that it may be the idea to skip ifds
    //                 }
    //                 // any other error is bad, why can't I raise without cloning?
    //                 // ah well, whatevs
    //                 _ => bail!(e),
    //             },
    //             Ok(Some(v)) => true,
    //             Ok(None) => false,
    //         } {
    //             // println!("weee");
    //         }
    //         Ok(writer.finish())
    //     }
    //     #[async_trait::async_trait]
    //     impl AsyncRead for Bytes {
    //         async fn read_range(&self, range: Range<u64>) -> Bytes {
    //             self.slice(range.start as usize..self.len().min(range.end as usize))
    //         }
    //         async fn read_ranges(&self, ranges: &[Range<u64>]) -> impl Iterator<Item = Bytes> {
    //             ranges
    //                 .iter()
    //                 .map(|r| self.slice(r.start as usize..r.end as usize))
    //         }
    //     }
    //     assert_eq!(
    //         async_copy(circular_tiff())
    //             .await
    //             .unwrap_err()
    //             .frame()
    //             .error()
    //             .downcast_ref::<TiffLoadError>()
    //             .unwrap(),
    //         &TiffLoadError {
    //             message: "Cycle in offsets detected at ifd 8".into(),
    //             status: ErrorStatus::Permanent,
    //             kind: TiffLoadErrorKind::InvalidTiff
    //         }
    //     )
    // }

    #[test]
    fn test_rangebounds() {
        // sanity check on rangebounds
        fn get_range(range: impl RangeBounds<u64>) -> Vec<u8> {
            let datasource = [42u8; 42];
            let start_bound = range.start_bound().map(|s| usize::try_from(*s).unwrap());
            let end_bound = range.end_bound().map(|e| usize::try_from(*e).unwrap());
            datasource[(start_bound, end_bound)].to_vec()
        }
        assert_eq!(get_range(2..10), vec![42; (2..10).len()]);
        fn get_range_from_btree_map(range: impl RangeBounds<u64>) -> Vec<u8> {
            let datasource = BTreeMap::from([(5, vec![42u8; 42])]);
            let start_bound = range.start_bound().map(|s| usize::try_from(*s).unwrap());
            let end_bound = range.end_bound().map(|e| usize::try_from(*e).unwrap());
            datasource
                .range((start_bound, end_bound))
                .next()
                .unwrap()
                .1
                .clone()
        }
    }
}
