use crate::ByteOrder;

mod metadata;
mod tile;

pub struct TiffMetaSaver {}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct TiffSaver {
    image_width: u64,
    image_height: u64,
    bigtiff: bool,
    byte_order: ByteOrder,
}

impl TiffSaver {
    /// Create a new TiffSaver for the given image size
    pub fn new(image_width: u64, image_height: u64) -> Self {
        Self {
            image_width,
            image_height,
            ..Default::default()
        }
    }

    pub fn bigtiff(mut self, bigtiff: bool) -> Self {
        self.bigtiff = bigtiff;
        self
    }

    pub fn with_byte_order(mut self, byte_order: ByteOrder) -> Self {
        self.byte_order = byte_order;
        self
    }
}
