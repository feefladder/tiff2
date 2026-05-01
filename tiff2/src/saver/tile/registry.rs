use std::collections::HashMap;
use std::fmt::Debug;
use std::io::{Cursor, Read};

use exn::{ensure, ResultExt};
use weezl::LzwStatus;

use crate::loader::CodingResult;
use crate::structs::error::CodingError;
use crate::structs::metadata::tags::CompressionMethod;
use crate::structs::TileOpts;

type EncodingResult<T> = exn::Result<T, CodingError>;

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
                // TODO: compression level is actually a tag that we should write
                compression_level: flate2::Compression::fast(),
            }) as _,
        );
        // #[cfg(feature = "lerc")]
        // registry.insert(CompressionMethod::LERC, Box::new(LercEncoder) as _);
        // #[cfg(feature = "lzma")]
        // registry.insert(CompressionMethod::LZMA, Box::new(LZMAEncoder) as _);
        registry.insert(CompressionMethod::LZW, Box::new(LzwEncoder) as _);
        #[cfg(feature = "jpeg-encoder")]
        registry.insert(
            CompressionMethod::ModernJPEG,
            Box::new(JpegEncoder { quality: 90 }) as _,
        );
        // #[cfg(feature = "jpeg2k")]
        // registry.insert(CompressionMethod::JPEG2k, Box::new(JPEG2kEncoder) as _);
        #[cfg(feature = "webp-agpl")]
        registry.insert(
            CompressionMethod::WebP,
            Box::new(ZenWebPEncoder(zenwebp::EncoderConfig::Lossless(
                zenwebp::LosslessConfig::new(),
            ))) as _,
        );
        #[cfg(feature = "webp-cpp")]
        registry.insert(
            CompressionMethod::WebP,
            Box::new(WebPEncoder { quality: 0.9 }) as _,
        );
        #[cfg(feature = "zstd")]
        registry.insert(CompressionMethod::ZSTD, Box::new(ZstdEncoder) as _);
        Self(registry)
    }
}

// not sure why this can't be `Clone`...
pub trait Encoder: Debug + Send + Sync {
    fn encode_tile(
        &self,
        in_buf: &[u8],
        out_buf: &mut [u8],
        tile_opts: &TileOpts,
        tile_width: u32,
        tile_height: u32,
    ) -> CodingResult<u64>;
}

#[derive(Debug, Clone, Copy)]
pub struct UncompressedEncoder;

impl Encoder for UncompressedEncoder {
    fn encode_tile(
        &self,
        in_buf: &[u8],
        out_buf: &mut [u8],
        _tile_opts: &TileOpts,
        _tile_width: u32,
        _tile_height: u32,
    ) -> EncodingResult<u64> {
        ensure!(
            in_buf.len() == out_buf.len(),
            CodingError::incomplete(in_buf.len(), out_buf.len())
        );
        out_buf[..in_buf.len()].copy_from_slice(in_buf);
        Ok(u64::try_from(in_buf.len()).unwrap())
    }
}

#[cfg(feature = "lzw")]
#[derive(Debug, Clone, Copy)]
pub struct LzwEncoder;

#[cfg(feature = "lzw")]
impl Encoder for LzwEncoder {
    fn encode_tile(
        &self,
        in_buf: &[u8],
        out_buf: &mut [u8],
        _tile_opts: &TileOpts,
        _tile_width: u32,
        _tile_height: u32,
    ) -> EncodingResult<u64> {
        let mut encoder = weezl::encode::Encoder::with_tiff_size_switch(weezl::BitOrder::Msb, 8);
        let res = encoder.encode_bytes(in_buf, out_buf);
        let lzw_status = res
            .status
            .or_raise(|| CodingError::failed("encoding failed".to_string()))?;
        if res.consumed_in != in_buf.len() || !matches!(lzw_status, LzwStatus::Done) {
            Err(CodingError::incomplete(res.consumed_in, out_buf.len()).into())
        } else {
            Ok(u64::try_from(res.consumed_out).unwrap())
        }
    }
}

#[cfg(feature = "deflate")]
#[derive(Debug, Clone, Copy)]
pub struct DeflateEncoder {
    pub compression_level: flate2::Compression,
}

#[cfg(feature = "deflate")]
impl Encoder for DeflateEncoder {
    fn encode_tile(
        &self,
        in_buf: &[u8],
        out_buf: &mut [u8],
        _tile_opts: &TileOpts,
        _tile_width: u32,
        _tile_height: u32,
    ) -> EncodingResult<u64> {
        let mut encoder =
            flate2::bufread::ZlibEncoder::new(Cursor::new(in_buf), self.compression_level);
        Ok(u64::try_from(encoder.read(out_buf).unwrap()).unwrap())
    }
}

#[cfg(feature = "jpeg-encoder")]
#[derive(Debug, Clone)]
pub struct JpegEncoder {
    quality: u8,
}

#[cfg(feature = "jpeg-encoder")]
impl Encoder for JpegEncoder {
    fn encode_tile(
        &self,
        in_buf: &[u8],
        out_buf: &mut [u8],
        tile_opts: &TileOpts,
        tile_width: u32,
        tile_height: u32,
    ) -> CodingResult<u64> {
        use crate::ColorType;
        let encoder = jpeg_encoder::Encoder::new(Cursor::new(out_buf), self.quality);
        let color_type = match tile_opts
            .colortype()
            .or_raise(|| CodingError::failed("could not find colortype".to_string()))?
        {
            ColorType::RGB(8) => jpeg_encoder::ColorType::Rgb,
            ColorType::RGBA(8) => jpeg_encoder::ColorType::Rgba,
            ColorType::CMYK(8) => jpeg_encoder::ColorType::Cmyk,
            ColorType::Gray(8) => jpeg_encoder::ColorType::Luma,
            ColorType::YCbCr(8) => jpeg_encoder::ColorType::Ycbcr,
            ct => exn::bail!(CodingError::failed(format!(
                "color type {ct:?} not supported with jpeg"
            ))),
        };
        let colortype = encoder
            .encode(
                in_buf,
                u16::try_from(tile_width).or_raise(|| {
                    CodingError::failed(format!("tile width {tile_width} too large for encoding"))
                })?,
                u16::try_from(tile_height).or_raise(|| {
                    CodingError::failed(format!("tile height {tile_height} too large for encoding"))
                })?,
                color_type,
            )
            .or_raise(|| CodingError::failed(format!("could not jpeg encode tile")))?;
        Ok(42)
    }
}

#[cfg(feature = "webp-cpp")]
#[derive(Debug, Clone)]
pub struct WebPEncoder {
    quality: f32,
}

#[cfg(feature = "webp-cpp")]
impl Encoder for WebPEncoder {
    fn encode_tile(
        &self,
        in_buf: &[u8],
        out_buf: &mut [u8],
        tile_opts: &TileOpts,
        tile_width: u32,
        tile_height: u32,
    ) -> CodingResult<u64> {
        use webp::PixelLayout;

        use crate::ColorType;

        let layout = match tile_opts.colortype() {
            Ok(ColorType::RGB(8)) => PixelLayout::Rgb,
            Ok(ColorType::RGBA(8)) => PixelLayout::Rgba,
            Ok(c) => exn::bail!(CodingError::failed(format!(
                "incomprehensible colortype {c:?} for webp encoding"
            ))),
            Err(e) => exn::bail!(e.raise(CodingError::failed(format!(
                "colortype unsupported with webp"
            )))),
        };
        let encoder = webp::Encoder::new(in_buf, layout, tile_width, tile_height);
        let res = encoder.encode(self.quality);
        ensure!(
            res.len() <= out_buf.len(),
            CodingError::incomplete(res.len(), out_buf.len())
        );
        out_buf.copy_from_slice(&res[..out_buf.len()]);
        Ok(out_buf.len().try_into().unwrap())
    }
}

#[cfg(feature = "webp-agpl")]
#[derive(Debug, Clone)]
pub struct ZenWebPEncoder(zenwebp::EncoderConfig);

#[cfg(feature = "webp-agpl")]
impl Encoder for ZenWebPEncoder {
    fn encode_tile(
        &self,
        in_buf: &[u8],
        out_buf: &mut [u8],
        tile_opts: &TileOpts,
        tile_width: u32,
        tile_height: u32,
    ) -> CodingResult<u64> {
        use crate::structs::metadata::tags::PhotometricInterpretation;
        use exn::bail;

        let bit_depth = tile_opts.bits_per_sample;
        ensure!(
            bit_depth == 8,
            CodingError::unsupported_bit_depth(bit_depth, "webp only supports 8-bit pixels")
        );
        let color_type = match (
            tile_opts.photometric_interpretation,
            tile_opts.samples_per_pixel,
        ) {
            (PhotometricInterpretation::RGB, 3) => zenwebp::PixelLayout::Rgb8,
            (PhotometricInterpretation::RGB, 4) => zenwebp::PixelLayout::Rgba8,
            (PhotometricInterpretation::BlackIsZero, 1) => zenwebp::PixelLayout::L8,
            (PhotometricInterpretation::WhiteIsZero, 1) => zenwebp::PixelLayout::L8, // TODO: Should we invert colors here?
            (photometric_interpretation, spp) => {
                bail!(CodingError::failed(format!("photometric interpretation {photometric_interpretation:?} and samples {spp} unsupported for webp")));
            }
        };
        let encoded =
            zenwebp::EncodeRequest::new(&self.0, in_buf, color_type, tile_width, tile_height)
                .encode()
                .or_raise(|| CodingError::failed("could not encode image".to_string()))?;
        out_buf[..encoded.len()].copy_from_slice(&encoded);
        Ok(u64::try_from(encoded.len()).expect("128-bit pointers not supported"))
    }
}

#[cfg(feature = "zstd")]
#[derive(Debug, Clone)]
pub struct ZstdEncoder;

#[cfg(feature = "zstd")]
impl Encoder for ZstdEncoder {
    fn encode_tile(
        &self,
        in_buf: &[u8],
        out_buf: &mut [u8],
        tile_opts: &TileOpts,
        tile_width: u32,
        tile_height: u32,
    ) -> CodingResult<u64> {
        let mut c = std::io::Cursor::new(out_buf);
        // TODO: make this a parameter, but it needs to implement debug first
        ruzstd::encoding::compress(in_buf, &mut c, ruzstd::encoding::CompressionLevel::Fastest);
        Ok(c.position())
    }
}
