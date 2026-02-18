//! Tiff struct that holds all *meta*data of a tiff
//! Can be used for both decoding and encoding purposes
use crate::structs::Image;
use crate::ByteOrder;

pub struct Tiff<R> {
    pub images: Vec<Image>,
    bigtiff: bool,
    byte_order: ByteOrder,
    reader: R,
    // add additional global stuff such as geo-info here
}

impl<R> Tiff<R> {
    /// is this tiff a bigtiff?
    pub fn bigtiff(&self) -> bool {
        self.bigtiff
    }

    /// returns byte order
    pub fn byte_order(&self) -> ByteOrder {
        self.byte_order
    }

    /// the internal reader
    pub fn reader(&self) -> &R {
        &self.reader
    }
}
