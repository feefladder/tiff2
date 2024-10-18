use std::io::{self, Read};

use crate::ByteOrder;

use async_trait::async_trait;

/// Trait for a CogReader to implement. In fact these are all the same, but caching can be optimized based on which part of the tiff we're reading in.
#[async_trait]
pub trait CogReader {
    // https://blog.rust-lang.org/2023/12/21/async-fn-rpit-in-traits.html#where-the-gaps-lie
    async fn read_ifd(&self, byte_start: u64, n_bytes: u64) -> Vec<u8>;
    async fn read_tag_data(&self, byte_start: u64, n_bytes: u64) -> Vec<u8>;
    async fn read_image_data(&self, byte_start: u64, n_bytes: u64) -> Vec<u8>;
}

pub struct EndianReader<R> {
    pub(super) reader: R,
    pub byte_order: ByteOrder,
}

impl<R: io::Read> io::Read for EndianReader<R> {
    #[inline]
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.reader.read(buf)
    }
}

macro_rules! read_fn {
    ($name:ident, $type:ty) => {
        /// reads an $type, respecting byte order
        #[inline(always)]
        pub fn $name(&mut self) -> Result<$type, io::Error> {
            let mut n = [0u8; std::mem::size_of::<$type>()];
            self.read_exact(&mut n)?;
            Ok(match self.byte_order() {
                ByteOrder::LittleEndian => <$type>::from_le_bytes(n),
                ByteOrder::BigEndian => <$type>::from_be_bytes(n),
            })
        }
    };
}

impl<R: io::Read> EndianReader<R> {
    /// Wraps a reader
    pub fn wrap(reader: R, byte_order: ByteOrder) -> Self {
        EndianReader { reader, byte_order }
    }

    fn byte_order(&self) -> ByteOrder {
        self.byte_order
    }

    read_fn!(read_u8, u8);
    read_fn!(read_i8, i8);
    read_fn!(read_u16, u16);
    read_fn!(read_i16, i16);
    read_fn!(read_u32, u32);
    read_fn!(read_i32, i32);
    read_fn!(read_u64, u64);
    read_fn!(read_i64, i64);

    read_fn!(read_f32, f32);
    read_fn!(read_f64, f64);
}
