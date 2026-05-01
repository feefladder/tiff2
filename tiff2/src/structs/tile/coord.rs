/// A tile coordinate within a single sub-image (overview)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TileCoord {
    /// The x-coordinate
    pub x: u32,
    /// The y-coordinate
    pub y: u32,
    /// The band.
    ///
    /// In chunky configuration, this should be `1`
    pub band: u16,
}

impl From<(u32, u32)> for TileCoord {
    fn from(xy: (u32, u32)) -> Self {
        Self {
            x: xy.0,
            y: xy.1,
            band: 0,
        }
    }
}

impl From<(u32, u32, u16)> for TileCoord {
    fn from(xyb: (u32, u32, u16)) -> Self {
        Self {
            x: xyb.0,
            y: xyb.1,
            band: xyb.2,
        }
    }
}
