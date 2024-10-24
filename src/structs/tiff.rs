//! Tiff struct that holds all *meta*data of a tiff
//! Can be used for both decoding and encoding purposes

use crate::{structs::Image, ByteOrder};

pub struct tiff<R> {
    pub images: Vec<Image>,
    bigtiff: bool,
    byte_order: ByteOrder,
    reader: R,
    // add additional global stuff such as geo-info here
}
