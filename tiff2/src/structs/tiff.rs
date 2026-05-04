//! Tiff struct that holds all *meta*data of a tiff
//! Can be used for both decoding and encoding purposes
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use crate::structs::Ifd;
use crate::structs::TiffExtension;
use crate::ByteOrder;

/// The header byte size of a tiff file in bytes
///
/// |field|small|big|
/// |:----|:---:|:-:|
/// |byte-order mark|2|2|
/// |magic number  | 2|2|
/// |offset size   |  |2|
/// |reserved      |  |2|
/// |offset        | 4|8|
/// |total         |8|16|
#[inline]
#[must_use]
#[rustfmt::skip]
pub const fn header_size(bigtiff: bool) -> u64 {
    2+2+ // byte-order mark str[2] + magic number u16
    if bigtiff {
        2+ // offset size u16 (always 8)
        2+ // reserved ?u16? (always 0)
        8  // offset u64
    } else {
        4  // offset u32
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Tiff {
    // hmm, there may be a problem with encoding later, because we can only
    // insert ifds once the offset is already known and then there's no real
    // flag to say if it's written or not...
    pub(crate) ifds: BTreeMap<u64, Ifd>,
    pub(crate) ifd_offsets: Vec<u64>,
    pub(crate) extensions: HashMap<&'static str, Arc<dyn TiffExtension>>,
    pub(crate) bigtiff: bool,
    pub(crate) byte_order: ByteOrder,
    // add additional global stuff such as geo-info here
}

impl Tiff {
    /// is this tiff a bigtiff?
    pub fn bigtiff(&self) -> bool {
        self.bigtiff
    }

    /// returns byte order
    pub fn byte_order(&self) -> ByteOrder {
        self.byte_order
    }

    /// Get the nth ifd
    pub fn ifd(&self, idx: usize) -> &Ifd {
        &self.ifds[&self.ifd_offsets[idx]]
    }

    pub fn len(&self) -> usize {
        self.ifd_offsets.len()
    }

    pub fn is_empty(&self) -> bool {
        self.ifd_offsets.is_empty()
    }
}
