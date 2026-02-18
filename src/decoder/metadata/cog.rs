//! Cog caching middleware
//!
//! Also to see if we can expose the iterator-like functionality

use std::{
    collections::BTreeMap,
    ops::{Bound, Range, RangeBounds},
};

use bytes::Bytes;
use exn::{OptionExt, ResultExt};
use log::error;

use crate::decoder::metadata::{
    error::{CacheMiss, MetaError},
    MetaResult, Tiff,
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
    /// If all the ifd's values are not present in the cache, an error is returned
    pub fn next(&mut self) -> MetaResult<()> {
        let offset = self.tiff.next_ifd_offset();
        let mut ifd_loader = self.tiff.ifd_loader(
            &self
                .slice(usize::try_from(offset).unwrap()..)
                .or_raise(|| {
                    MetaError::invalid_buffer(
                        offset..offset + 1024,
                        "failed to create ifd loader".into(),
                    )
                })?,
            offset,
        )?;
        // so the iterator magic breaks down here, because we can't give an
        // iterator that borrows &ifd_loader and pass it to
        // ifd_loader.load_ifd_values(&mut self)
        //
        // let's allocate some vec's
        let mut ifd_value_bufs = Vec::with_capacity(ifd_loader.count());
        // in the happy path, everything is loaded, so no allocations there
        let mut deferred_values = Vec::new();
        let mut deferred_tags = Vec::new();
        for (t, range) in ifd_loader.value_ranges() {
            match self
                .slice(usize::try_from(range.start).unwrap()..usize::try_from(range.end).unwrap())
            {
                Ok(data) => ifd_value_bufs.push((t, data)),
                Err(e) => {
                    error!("{e}");
                    deferred_values.push(range);
                    deferred_tags.push(t);
                }
            }
        }
        ifd_loader.load_ifd_values(offset, &mut ifd_value_bufs.into_iter())?;
        self.tiff.insert_ifd(offset, ifd_loader.finish());
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
            Ok(())
        }
    }

    // so the sad thing here is that we can't create a BytesMut from a &mut
    // self, since cloning will increase the refcount, so this is always a
    // deep clone:
    //
    // let mut new = BytesMut::from(self.cache.clone());
    //
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
    fn slice(&self, range: impl RangeBounds<usize>) -> exn::Result<Bytes, CacheMiss> {
        let start = range.start_bound();
        let end = range.end_bound();
        self.cache
            // make a range from 0..start
            .range((Bound::Included(&0), start))
            // find the last chunk that has the bounds
            .rfind(|(section_start, bytes)| match end {
                Bound::Unbounded => true,
                // the normal case
                //    |----|  req
                //         v
                //   |--len--| section
                // sstart    >=
                Bound::Excluded(v) => bytes.len() + **section_start >= *v,
                // the special case
                //    |----|  req
                //         v
                //   |--len--| section
                // sstart    >
                Bound::Included(v) => bytes.len() + **section_start > *v,
            })
            // So...
            .map(|(section_start, bytes)| {
                bytes.slice((
                    start.map(|s| s - section_start),
                    end.map(|e| e - section_start),
                ))
            })
            .ok_or_raise(|| CacheMiss(start.map(|s| *s), end.map(|e| *e)))
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
    use super::*;
    use std::{collections::BTreeMap, ops::Range};

    #[test]
    fn test_fancy_cache() {
        /// Get a slice from the cache
        ///
        /// This actually searches back-to-front, so `..=42` will give the smallest slice that contains `42`
        fn slice(
            cache: &BTreeMap<usize, Bytes>,
            range: impl RangeBounds<usize>,
        ) -> exn::Result<Bytes, CacheMiss> {
            let start = range.start_bound();
            let end = range.end_bound();
            cache
                // make a range from 0..start
                .range((Bound::Included(&0), start))
                // find the last chunk that has the bounds
                .rfind(|(section_start, bytes)| match end {
                    Bound::Unbounded => true,
                    // the normal case
                    //    |----|  req
                    //         v
                    //   |--len--| section
                    // sstart    >=
                    Bound::Excluded(v) => bytes.len() + **section_start >= *v,
                    // the special case
                    //    |----|  req
                    //         v
                    //   |--len--| section
                    // sstart    >
                    Bound::Included(v) => bytes.len() + **section_start > *v,
                })
                // So...
                .map(|(section_start, bytes)| {
                    bytes.slice((
                        start.map(|s| s - section_start),
                        end.map(|e| e - section_start),
                    ))
                })
                .ok_or_raise(|| CacheMiss(start.map(|s| *s), end.map(|e| *e)))
        }
        let mut cache = BTreeMap::new();
        cache.insert(0, Bytes::copy_from_slice(&[42; 42]));
        let desired_range = 10..32usize;
        assert_eq!(
            &slice(&cache, desired_range.clone()).unwrap()[..],
            &vec![42; (10..32).len()]
        );

        // now there's the completely useless edge-case of adding a buffer that breaks everything, because we don't support broken slices
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
                desired_range.start_bound().map(|s| *s),
                desired_range.end_bound().map(|e| *e)
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
}
