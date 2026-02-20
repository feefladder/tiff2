//! Cog caching middleware
//!
//! Also to see if we can expose the iterator-like functionality

use std::{
    collections::BTreeMap,
    ops::{Bound, RangeBounds},
};

use bytes::Bytes;
use exn::{OptionExt, ResultExt};
use log::{debug, error};

use crate::{
    decoder::metadata::{
        error::{CacheMiss, MetaError},
        MetaResult, Tiff,
    },
    structs::num_entries_size,
};

pub struct CogCache {
    cache: BTreeMap<usize, Bytes>,
    tiff: Tiff,
}

impl CogCache {
    /// Create a new CogCache with a prefetch buffer
    pub fn new(prefetch: Bytes) -> MetaResult<Self> {
        let tiff = Tiff::from_header(&prefetch)?;
        Ok(Self {
            cache: BTreeMap::from([(0, prefetch)]),
            tiff,
        })
    }

    /// Load the next ifd
    ///
    /// If all the ifd's values are not present in the cache, an error is
    /// returned. This error may be ignored if this ifd is not of interest
    pub fn next(&mut self) -> MetaResult<Option<u64>> {
        let Some(offset) = self.tiff.next_ifd_offset() else {
            return Ok(None);
        };
        println!("next ifd offset: {offset:x?}");
        let (mut ifd_loader, next_ifd) = self.tiff.ifd_loader(
            &self.slice(offset..).or_raise(|| {
                // TODO: this is an advisory, too-small range which will only load the number-of-entries
                // if the ifd is not in the cache at all, so a non-cog tiff
                // (or an extremely small prefetch)
                MetaError::invalid_buffer(
                    offset..offset + num_entries_size(self.tiff.bigtiff),
                    "failed to create ifd loader".into(),
                )
            })?,
            offset,
        )?;
        // so the iterator magic
        // (`ifd_loader.insert(ifd_loader.value_ranges().map(get_data))`) breaks
        // down here, because we can't give an iterator that borrows &ifd_loader
        // and pass it to ifd_loader.load_ifd_values(&mut self).
        //
        // Anyways that would be inserting stuff in an iterator we're iterating
        //
        // At most that's ~~one~~ _three_ allocations per ifd, so that's ok
        //
        // let's allocate some vecs
        let mut ifd_value_bufs = Vec::with_capacity(ifd_loader.count());
        // in the happy path, everything is loaded, so no allocations there
        let mut deferred_values = Vec::new();
        let mut deferred_tags = Vec::new();

        // try to get all values from the cache
        for (t, range) in ifd_loader.value_ranges() {
            match self.slice(range.clone()) {
                Ok(data) => ifd_value_bufs.push((t, data)),
                Err(e) => {
                    error!("{e}");
                    // insert magic caching/filtering strategies here
                    deferred_values.push(range);
                    deferred_tags.push(t);
                }
            }
        }
        ifd_loader.load_ifd_values(offset, &mut ifd_value_bufs.into_iter())?;
        self.tiff.insert_ifd(offset, ifd_loader.finish(), next_ifd);
        if !deferred_values.is_empty() {
            let n = deferred_tags.len();
            Err(MetaError::incomplete_ifd(
                deferred_values,
                offset,
                deferred_tags,
                format!("not all entries loaded, {n} tags are deferred"),
            )
            .into())
        } else {
            Ok(Some(offset))
        }
    }

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
    /// Insert more cache at the given offset
    ///
    /// Note that inserting a lot of small caches will decrease performance
    pub fn insert(&mut self, start: usize, data: Bytes) {
        self.cache.insert(start, data);
    }

    /// Get a slice from the cache
    ///
    /// This actually searches back-to-front, so `..=42` will give the smallest slice that contains `42`
    fn slice(&self, range: impl RangeBounds<u64>) -> exn::Result<Bytes, CacheMiss> {
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
            .ok_or_raise(|| CacheMiss(start, end))
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
    use bytemuck::BoxBytes;
    use exn::bail;

    use crate::decoder::metadata::error::{MetaErrorKind, MetaErrorStatus};

    use super::*;
    use std::{collections::BTreeMap, ops::Range};

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

    #[tokio::test]
    async fn test_async_copy() {
        // for now everything is infallible
        #[async_trait::async_trait]
        trait AsyncRead {
            async fn read_range(&self, range: Range<u64>) -> Bytes;
            // yeah so I guess this is kind of sad, because we could coalesce_ranges downstream,
            // so I guess just a &[Range<u64>]->&[Bytes] should suffice?
            async fn read_ranges(&self, ranges: &[Range<u64>]) -> impl Iterator<Item = Bytes>;
        }
        async fn async_copy(reader: impl AsyncRead) -> MetaResult<Tiff> {
            let prefetch = reader.read_range(0..1024 * 16).await;
            let mut writer = CogCache::new(prefetch).unwrap();
            while match writer.next() {
                // safe to unwrap the downcast_ref because of `writer.next()` return type
                Err(e) => match e.frame().error().downcast_ref::<MetaError>().unwrap() {
                    MetaError {
                        status: MetaErrorStatus::MissingRange { required },
                        kind,
                        message,
                    } => {
                        writer.insert(
                            usize::try_from(required.start).unwrap(),
                            reader.read_range(required.clone()).await,
                        );
                        true
                        // so the idea is here that calling `next` which errors doesn't advance the iterator, so calling "next" again will retry
                        // except that it may be the idea to skip ifds
                    }
                    // any other error is bad, why can't I raise without cloning?
                    // ah well, whatevs
                    e => bail!(e.clone()),
                },
                Ok(Some(v)) => true,
                Ok(None) => false,
            } {
                println!("weee");
            }
            Ok(writer.finish())
        }
        #[async_trait::async_trait]
        impl AsyncRead for Bytes {
            async fn read_range(&self, range: Range<u64>) -> Bytes {
                self.slice(range.start as usize..self.len().min(range.end as usize))
            }
            async fn read_ranges(&self, ranges: &[Range<u64>]) -> impl Iterator<Item = Bytes> {
                ranges
                    .iter()
                    .map(|r| self.slice(r.start as usize..r.end as usize))
            }
        }
        assert_eq!(
            async_copy(circular_tiff())
                .await
                .unwrap_err()
                .frame()
                .error()
                .downcast_ref::<MetaError>()
                .unwrap(),
            &MetaError {
                message: "Cycle in offsets detected at ifd 8".into(),
                status: MetaErrorStatus::Permanent,
                kind: MetaErrorKind::InvalidTiff
            }
        )
    }

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
