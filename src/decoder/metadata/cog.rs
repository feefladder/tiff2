//! Cog caching middleware
//!
//! Also to see if we can expose the iterator-like functionality

use bytes::{Bytes, BytesMut};

use crate::decoder::metadata::{error::MetaError, MetaResult, Tiff};

pub struct CogCache {
    cache: Bytes,
    tiff: Tiff,
}

impl CogCache {
    /// Create a new CogCache with a prefetch buffer
    pub fn new(prefetch: Bytes) -> MetaResult<Self> {
        let tiff = Tiff::from_header(&prefetch)?;
        Ok(Self {
            cache: prefetch,
            tiff,
        })
    }

    /// Load the next ifd
    ///
    /// If all the ifd's values are not present in the cache, an error is returned
    ///
    pub fn next(&mut self) -> MetaResult<()> {
        let offset = self.tiff.next_ifd_offset();
        let mut ifd_loader = self.tiff.ifd_loader(
            &self.cache.slice(usize::try_from(offset).unwrap()..),
            offset,
        )?;
        // so the iterator magic breaks down here, because we can't give an
        // iterator that borrows &ifd_loader and pass it to
        // ifd_loader.load_ifd_values(&mut self)
        let mut ifd_value_bufs = Vec::with_capacity(ifd_loader.count());
        // in the happy path, everything is loaded, so no allocations there
        let mut deferred_values = Vec::new();
        let mut deferred_tags = Vec::new();
        for (t, range) in ifd_loader.value_ranges() {
            if usize::try_from(range.end).unwrap() <= self.cache.len() {
                ifd_value_bufs.push((
                    t,
                    &self.cache[usize::try_from(range.start).unwrap()
                        ..usize::try_from(range.end).unwrap()],
                ))
            } else {
                deferred_values.push(range);
                deferred_tags.push(t);
            }
        }
        ifd_loader.load_ifd_values(offset, &mut ifd_value_bufs.into_iter())?;
        self.tiff.insert_ifd(offset, ifd_loader.finish());
        if !deferred_values.is_empty() {
            let n = deferred_tags.len();
            // TODO: should this error type be improved?
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

    pub fn extend_cache(&mut self, cache: Bytes) {
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
        // altogether under these invariants: cache start always points to a
        // logical value cache end is estimated from some heuristic so if
        // there's a BTreeMap<Range<u64>,Bytes>>, and we want to insert a new
        // cache block, but the previous one overlaps, then we shrink the
        // previous blocks' range. I'm not superduper sure
        //
        //
        // new.extend_from_slice(&cache[..]);
        // self.cache = new.into();
    }

    /// Finish metadata parsing and remove the cache
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
        let mut cache = BTreeMap::new();
        cache.insert(0, Bytes::copy_from_slice(&[42; 42]));
        let desired_range = 10..32;
        fn slice(cache: BTreeMap<usize, Bytes>, range: Range<usize>) -> Bytes {
            cache
                .range(0..=range.start)
                .find_map(|(start, bytes)| {
                    if bytes.len() > range.end - start {
                        Some(bytes.slice(range.start - start..range.end - start))
                    } else {
                        None
                    }
                })
                .unwrap()
        }
        let slice = slice(cache, desired_range)
            assert_eq!(&slice[..], &vec![42; (10..32).len()]);
        // now there's the complete
    }
}
