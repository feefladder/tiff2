#[derive(Debug, Clone, Copy)]
pub enum TileDataType {
    U8,
    U16,
    U32,
    U64,
    I8,
    I16,
    I32,
    I64,
    F32,
    F64,
}

/// Result of a decoding process
#[derive(Debug)]
pub enum TileData {
    /// A vector of unsigned bytes
    U8(Vec<u8>),
    /// A vector of unsigned words
    U16(Vec<u16>),
    /// A vector of 32 bit unsigned ints
    U32(Vec<u32>),
    /// A vector of 64 bit unsigned ints
    U64(Vec<u64>),
    /// A vector of 8 bit signed ints
    I8(Vec<i8>),
    /// A vector of 16 bit signed ints
    I16(Vec<i16>),
    /// A vector of 32 bit signed ints
    I32(Vec<i32>),
    /// A vector of 64 bit signed ints
    I64(Vec<i64>),
    /// A vector of 32 bit IEEE floats
    F32(Vec<f32>),
    /// A vector of 64 bit IEEE floats
    F64(Vec<f64>),
}

impl TileData {
    pub fn new(size: usize, dtype: TileDataType) -> TileData {
        match dtype {
            TileDataType::U8 => TileData::U8(vec![0; size]),
            TileDataType::U16 => TileData::U16(vec![0; size]),
            TileDataType::U32 => TileData::U32(vec![0; size]),
            TileDataType::U64 => TileData::U64(vec![0; size]),
            TileDataType::I8 => TileData::I8(vec![0; size]),
            TileDataType::I16 => TileData::I16(vec![0; size]),
            TileDataType::I32 => TileData::I32(vec![0; size]),
            TileDataType::I64 => TileData::I64(vec![0; size]),
            TileDataType::F32 => TileData::F32(vec![0.0; size]),
            TileDataType::F64 => TileData::F64(vec![0.0; size]),
        }
    }

    pub fn len(&self) -> usize {
        match self {
            TileData::U8(v) => v.len(),
            TileData::U16(v) => v.len(),
            TileData::U32(v) => v.len(),
            TileData::U64(v) => v.len(),
            TileData::F32(v) => v.len(),
            TileData::F64(v) => v.len(),
            TileData::I8(v) => v.len(),
            TileData::I16(v) => v.len(),
            TileData::I32(v) => v.len(),
            TileData::I64(v) => v.len(),
        }
    }

    pub fn is_empty(&self) -> bool {
        match self {
            TileData::U8(v) => v.is_empty(),
            TileData::U16(v) => v.is_empty(),
            TileData::U32(v) => v.is_empty(),
            TileData::U64(v) => v.is_empty(),
            TileData::F32(v) => v.is_empty(),
            TileData::F64(v) => v.is_empty(),
            TileData::I8(v) => v.is_empty(),
            TileData::I16(v) => v.is_empty(),
            TileData::I32(v) => v.is_empty(),
            TileData::I64(v) => v.is_empty(),
        }
    }
}

impl AsMut<[u8]> for TileData {
    fn as_mut(&mut self) -> &mut [u8] {
        match self {
            TileData::U8(buf) => bytemuck::cast_slice_mut(buf),
            TileData::U16(buf) => bytemuck::cast_slice_mut(buf),
            TileData::U32(buf) => bytemuck::cast_slice_mut(buf),
            TileData::U64(buf) => bytemuck::cast_slice_mut(buf),
            TileData::F32(buf) => bytemuck::cast_slice_mut(buf),
            TileData::F64(buf) => bytemuck::cast_slice_mut(buf),
            TileData::I8(buf) => bytemuck::cast_slice_mut(buf),
            TileData::I16(buf) => bytemuck::cast_slice_mut(buf),
            TileData::I32(buf) => bytemuck::cast_slice_mut(buf),
            TileData::I64(buf) => bytemuck::cast_slice_mut(buf),
        }
    }
}
