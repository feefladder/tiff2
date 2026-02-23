use crate::{error::TiffResult, loader::tile::ChunkOpts, structs::tags::CompressionMethod};
use std::{
    collections::HashMap,
    fmt::Debug,
    io::{Cursor, Read},
};

#[derive(Debug)]
pub struct EncoderRegistry(HashMap<CompressionMethod, Box<dyn Encoder>>);

impl EncoderRegistry {
    /// Create a new Encoder registry with no Encoders registered
    pub fn empty() -> Self {
        Self(HashMap::new())
    }
}

impl AsRef<HashMap<CompressionMethod, Box<dyn Encoder>>> for EncoderRegistry {
    fn as_ref(&self) -> &HashMap<CompressionMethod, Box<dyn Encoder>> {
        &self.0
    }
}

impl AsMut<HashMap<CompressionMethod, Box<dyn Encoder>>> for EncoderRegistry {
    fn as_mut(&mut self) -> &mut HashMap<CompressionMethod, Box<dyn Encoder>> {
        &mut self.0
    }
}

impl Default for EncoderRegistry {
    fn default() -> Self {
        let mut registry = HashMap::with_capacity(6);
        registry.insert(CompressionMethod::None, Box::new(UncompressedEncoder) as _);
        #[cfg(feature = "deflate")]
        registry.insert(
            CompressionMethod::Deflate,
            Box::new(DeflateEncoder {
                compression_level: flate2::Compression::fast(),
            }) as _,
        );
        #[cfg(feature = "deflate")]
        registry.insert(
            CompressionMethod::OldDeflate,
            Box::new(DeflateEncoder {
                compression_level: flate2::Compression::fast(),
            }) as _,
        );
        #[cfg(feature = "lerc")]
        registry.insert(CompressionMethod::LERC, Box::new(LercEncoder) as _);
        #[cfg(feature = "lzma")]
        registry.insert(CompressionMethod::LZMA, Box::new(LZMAEncoder) as _);
        registry.insert(CompressionMethod::LZW, Box::new(LzwEncoder) as _);
        #[cfg(feature = "jpeg")]
        registry.insert(CompressionMethod::ModernJPEG, Box::new(JpegEncoder) as _);
        #[cfg(feature = "jpeg2k")]
        registry.insert(CompressionMethod::JPEG2k, Box::new(JPEG2kEncoder) as _);
        #[cfg(feature = "webp")]
        registry.insert(CompressionMethod::WebP, Box::new(WebPEncoder) as _);
        #[cfg(feature = "zstd")]
        registry.insert(CompressionMethod::ZSTD, Box::new(ZstdEncoder) as _);
        Self(registry)
    }
}

pub trait Encoder: Debug + Send + Sync {
    fn encode_chunk(
        &self,
        in_buf: &[u8],
        out_buf: &mut [u8],
        chunk_opts: &ChunkOpts,
    ) -> TiffResult<u64>;
}

#[derive(Debug)]
pub struct UncompressedEncoder;

impl Encoder for UncompressedEncoder {
    fn encode_chunk(
        &self,
        in_buf: &[u8],
        out_buf: &mut [u8],
        chunk_opts: &ChunkOpts,
    ) -> TiffResult<u64> {
        out_buf[..in_buf.len()].copy_from_slice(in_buf);
        Ok(u64::try_from(in_buf.len()).unwrap())
    }
}

#[derive(Debug)]
pub struct LzwEncoder;

impl Encoder for LzwEncoder {
    fn encode_chunk(
        &self,
        in_buf: &[u8],
        out_buf: &mut [u8],
        chunk_opts: &ChunkOpts,
    ) -> TiffResult<u64> {
        let mut encoder = weezl::encode::Encoder::with_tiff_size_switch(weezl::BitOrder::Msb, 8);
        let res = encoder.encode_bytes(in_buf, out_buf);
        if res.consumed_in != in_buf.len() {
            todo!("proper error handling")
        }
        Ok(u64::try_from(res.consumed_out).unwrap())
    }
}

#[derive(Debug)]
pub struct DeflateEncoder {
    pub compression_level: flate2::Compression,
}

impl Encoder for DeflateEncoder {
    fn encode_chunk(
        &self,
        in_buf: &[u8],
        out_buf: &mut [u8],
        chunk_opts: &ChunkOpts,
    ) -> TiffResult<u64> {
        let mut encoder =
            flate2::bufread::ZlibEncoder::new(Cursor::new(in_buf), self.compression_level);
        Ok(u64::try_from(encoder.read(out_buf).unwrap()).unwrap())
    }
}
