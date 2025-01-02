use crate::{
    decoder::Limits,
    error::{TiffError, TiffResult},
};

/// Result of a decoding process
#[derive(Debug)]
pub enum DecodingResult {
    /// A vector of unsigned bytes
    U8(Vec<u8>),
    /// A vector of unsigned words
    U16(Vec<u16>),
    /// A vector of 32 bit unsigned ints
    U32(Vec<u32>),
    /// A vector of 64 bit unsigned ints
    U64(Vec<u64>),
    /// A vector of 32 bit IEEE floats
    F32(Vec<f32>),
    /// A vector of 64 bit IEEE floats
    F64(Vec<f64>),
    /// A vector of 8 bit signed ints
    I8(Vec<i8>),
    /// A vector of 16 bit signed ints
    I16(Vec<i16>),
    /// A vector of 32 bit signed ints
    I32(Vec<i32>),
    /// A vector of 64 bit signed ints
    I64(Vec<i64>),
}

impl DecodingResult {
    pub fn new_u8(size: usize, limits: &Limits) -> TiffResult<DecodingResult> {
        if size > limits.decoding_buffer_size {
            Err(TiffError::LimitsExceeded)
        } else {
            Ok(DecodingResult::U8(vec![0; size]))
        }
    }

    pub fn new_u16(size: usize, limits: &Limits) -> TiffResult<DecodingResult> {
        if size > limits.decoding_buffer_size / 2 {
            Err(TiffError::LimitsExceeded)
        } else {
            Ok(DecodingResult::U16(vec![0; size]))
        }
    }

    pub fn new_u32(size: usize, limits: &Limits) -> TiffResult<DecodingResult> {
        if size > limits.decoding_buffer_size / 4 {
            Err(TiffError::LimitsExceeded)
        } else {
            Ok(DecodingResult::U32(vec![0; size]))
        }
    }

    pub fn new_u64(size: usize, limits: &Limits) -> TiffResult<DecodingResult> {
        if size > limits.decoding_buffer_size / 8 {
            Err(TiffError::LimitsExceeded)
        } else {
            Ok(DecodingResult::U64(vec![0; size]))
        }
    }

    pub fn new_f32(size: usize, limits: &Limits) -> TiffResult<DecodingResult> {
        if size > limits.decoding_buffer_size / std::mem::size_of::<f32>() {
            Err(TiffError::LimitsExceeded)
        } else {
            Ok(DecodingResult::F32(vec![0.0; size]))
        }
    }

    pub fn new_f64(size: usize, limits: &Limits) -> TiffResult<DecodingResult> {
        if size > limits.decoding_buffer_size / std::mem::size_of::<f64>() {
            Err(TiffError::LimitsExceeded)
        } else {
            Ok(DecodingResult::F64(vec![0.0; size]))
        }
    }

    pub fn new_i8(size: usize, limits: &Limits) -> TiffResult<DecodingResult> {
        if size > limits.decoding_buffer_size / std::mem::size_of::<i8>() {
            Err(TiffError::LimitsExceeded)
        } else {
            Ok(DecodingResult::I8(vec![0; size]))
        }
    }

    pub fn new_i16(size: usize, limits: &Limits) -> TiffResult<DecodingResult> {
        if size > limits.decoding_buffer_size / 2 {
            Err(TiffError::LimitsExceeded)
        } else {
            Ok(DecodingResult::I16(vec![0; size]))
        }
    }

    pub fn new_i32(size: usize, limits: &Limits) -> TiffResult<DecodingResult> {
        if size > limits.decoding_buffer_size / 4 {
            Err(TiffError::LimitsExceeded)
        } else {
            Ok(DecodingResult::I32(vec![0; size]))
        }
    }

    pub fn new_i64(size: usize, limits: &Limits) -> TiffResult<DecodingResult> {
        if size > limits.decoding_buffer_size / 8 {
            Err(TiffError::LimitsExceeded)
        } else {
            Ok(DecodingResult::I64(vec![0; size]))
        }
    }

    pub fn as_buffer(&mut self, start: usize) -> &mut [u8] {
        match *self {
            DecodingResult::U8(ref mut buf) => bytemuck::cast_slice_mut(&mut buf[start..]),
            DecodingResult::U16(ref mut buf) => bytemuck::cast_slice_mut(&mut buf[start..]),
            DecodingResult::U32(ref mut buf) => bytemuck::cast_slice_mut(&mut buf[start..]),
            DecodingResult::U64(ref mut buf) => bytemuck::cast_slice_mut(&mut buf[start..]),
            DecodingResult::F32(ref mut buf) => bytemuck::cast_slice_mut(&mut buf[start..]),
            DecodingResult::F64(ref mut buf) => bytemuck::cast_slice_mut(&mut buf[start..]),
            DecodingResult::I8(ref mut buf) => bytemuck::cast_slice_mut(&mut buf[start..]),
            DecodingResult::I16(ref mut buf) => bytemuck::cast_slice_mut(&mut buf[start..]),
            DecodingResult::I32(ref mut buf) => bytemuck::cast_slice_mut(&mut buf[start..]),
            DecodingResult::I64(ref mut buf) => bytemuck::cast_slice_mut(&mut buf[start..]),
        }
    }

    pub fn len(&self) -> usize {
        match self {
            DecodingResult::U8(v) => v.len(),
            DecodingResult::U16(v) => v.len(),
            DecodingResult::U32(v) => v.len(),
            DecodingResult::U64(v) => v.len(),
            DecodingResult::F32(v) => v.len(),
            DecodingResult::F64(v) => v.len(),
            DecodingResult::I8(v) => v.len(),
            DecodingResult::I16(v) => v.len(),
            DecodingResult::I32(v) => v.len(),
            DecodingResult::I64(v) => v.len(),
        }
    }

    pub fn is_empty(&self) -> bool {
        match self {
            DecodingResult::U8(v) => v.is_empty(),
            DecodingResult::U16(v) => v.is_empty(),
            DecodingResult::U32(v) => v.is_empty(),
            DecodingResult::U64(v) => v.is_empty(),
            DecodingResult::F32(v) => v.is_empty(),
            DecodingResult::F64(v) => v.is_empty(),
            DecodingResult::I8(v) => v.is_empty(),
            DecodingResult::I16(v) => v.is_empty(),
            DecodingResult::I32(v) => v.is_empty(),
            DecodingResult::I64(v) => v.is_empty(),
        }
    }
}
