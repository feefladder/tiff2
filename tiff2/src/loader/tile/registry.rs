use std::collections::HashMap;
use std::fmt::Debug;
use std::io::{Cursor, Read};

use exn::{ensure, ResultExt};
#[cfg(feature = "deflate")]
use flate2::bufread::ZlibDecoder;
#[cfg(feature = "lzw")]
use weezl::LzwStatus;

#[cfg(feature = "lzw")]
use crate::loader::CodingResult;
#[cfg(feature = "lzw")]
use crate::structs::error::CodingError;
use crate::structs::metadata::tags::CompressionMethod;
use crate::structs::TileOpts;

// from async-tiff
/// A registry of decoders.
///
/// This allows end users to register their own decoders, for custom compression methods, or
/// override the default decoder implementations.
///
/// ```
/// use tiff2::loader::DecoderRegistry;
///
/// // Default registry includes Deflate, LZW.
/// let registry = DecoderRegistry::default();
/// assert_eq!(registry.0.len(), 4);
///
/// // Empty registry for manual configuration.
/// let empty = DecoderRegistry::empty();
/// assert_eq!(empty.0.len(), 0);
/// ```
#[derive(Debug)]
#[non_exhaustive]
pub struct DecoderRegistry(HashMap<CompressionMethod, Box<dyn Decoder>>);

impl DecoderRegistry {
    /// Create a new decoder registry with no decoders registered
    pub fn empty() -> Self {
        Self(HashMap::new())
    }
}

impl AsRef<HashMap<CompressionMethod, Box<dyn Decoder>>> for DecoderRegistry {
    fn as_ref(&self) -> &HashMap<CompressionMethod, Box<dyn Decoder>> {
        &self.0
    }
}

impl AsMut<HashMap<CompressionMethod, Box<dyn Decoder>>> for DecoderRegistry {
    fn as_mut(&mut self) -> &mut HashMap<CompressionMethod, Box<dyn Decoder>> {
        &mut self.0
    }
}

impl Default for DecoderRegistry {
    fn default() -> Self {
        let mut registry = HashMap::with_capacity(6);
        registry.insert(CompressionMethod::None, Box::new(UncompressedDecoder) as _);
        #[cfg(feature = "deflate")]
        registry.insert(CompressionMethod::Deflate, Box::new(DeflateDecoder) as _);
        registry.insert(CompressionMethod::OldDeflate, Box::new(DeflateDecoder) as _);
        // #[cfg(feature = "lerc")]
        // registry.insert(CompressionMethod::LERC, Box::new(LercDecoder) as _);
        // #[cfg(feature = "lzma")]
        // registry.insert(CompressionMethod::LZMA, Box::new(LZMADecoder) as _);
        registry.insert(CompressionMethod::LZW, Box::new(LZWDecoder) as _);
        #[cfg(feature = "jpeg-decoder")]
        registry.insert(CompressionMethod::JPEG, Box::new(JpegDecoder) as _);
        #[cfg(feature = "jpeg-decoder")]
        registry.insert(CompressionMethod::ModernJPEG, Box::new(JpegDecoder) as _);
        // #[cfg(feature = "jpeg")]
        // registry.insert(CompressionMethod::JPEG2k, Box::new(JPEG2kDecoder) as _);
        #[cfg(feature = "webp-agpl")]
        registry.insert(CompressionMethod::WebP, Box::new(ZenWebPDecoder) as _);
        #[cfg(feature = "webp-cpp")]
        registry.insert(CompressionMethod::WebP, Box::new(WebPDecoder) as _);
        #[cfg(feature = "zstd")]
        registry.insert(CompressionMethod::ZSTD, Box::new(ZstdDecoder) as _);
        Self(registry)
    }
}

pub trait Decoder: Debug + Send + Sync {
    fn decode_tile(
        &self,
        in_buf: &[u8],
        out_buf: &mut [u8],
        tile_opts: &TileOpts,
    ) -> CodingResult<()>;
}

#[derive(Debug, Clone, Copy)]
pub struct UncompressedDecoder;

impl Decoder for UncompressedDecoder {
    fn decode_tile(
        &self,
        in_buf: &[u8],
        out_buf: &mut [u8],
        _tile_opts: &TileOpts,
    ) -> exn::Result<(), CodingError> {
        ensure!(
            in_buf.len() == out_buf.len(),
            CodingError::incomplete(in_buf.len(), out_buf.len())
        );
        out_buf.copy_from_slice(in_buf);
        Ok(())
    }
}

#[cfg(feature = "deflate")]
#[derive(Debug, Clone, Copy)]
pub struct DeflateDecoder;

#[cfg(feature = "deflate")]
impl Decoder for DeflateDecoder {
    fn decode_tile(
        &self,
        in_buf: &[u8],
        out_buf: &mut [u8],
        _tile_opts: &TileOpts,
    ) -> exn::Result<(), CodingError> {
        let mut decoder = ZlibDecoder::new(Cursor::new(in_buf));
        decoder
            .read_exact(out_buf)
            .or_raise(|| CodingError::failed("decoding failed".to_string()))?;
        Ok(())
    }
}

#[cfg(feature = "lzw")]
#[derive(Debug, Clone, Copy)]
pub struct LZWDecoder;

#[cfg(feature = "lzw")]
impl Decoder for LZWDecoder {
    fn decode_tile(
        &self,
        in_buf: &[u8],
        out_buf: &mut [u8],
        _tile_opts: &TileOpts,
    ) -> CodingResult<()> {
        // https://github.com/image-rs/image-tiff/blob/90ae5b8e54356a35e266fb24e969aafbcb26e990/src/decoder/stream.rs#L147
        let mut decoder = weezl::decode::Decoder::with_tiff_size_switch(weezl::BitOrder::Msb, 8);
        let res = decoder.decode_bytes(in_buf, out_buf);
        let lzw_status = res
            .status
            .or_raise(|| CodingError::failed("decoding failed".to_string()))?;
        // verify the output
        if res.consumed_out != out_buf.len() || !matches!(lzw_status, LzwStatus::Done) {
            Err(CodingError::incomplete(res.consumed_out, out_buf.len()).into())
        } else {
            Ok(())
        }
    }
}

#[cfg(feature = "zstd")]
#[derive(Debug, Clone, Copy)]
pub struct ZstdDecoder;

#[cfg(feature = "zstd")]
impl Decoder for ZstdDecoder {
    fn decode_tile(
        &self,
        buf: &[u8],
        out_buf: &mut [u8],
        tile_opts: &TileOpts,
    ) -> CodingResult<()> {
        let mut decoder = ruzstd::decoding::StreamingDecoder::new(Cursor::new(buf))
            .or_raise(|| CodingError::failed("could not create Zstd decoder".to_string()))?;
        decoder
            .read_exact(out_buf)
            .or_raise(|| CodingError::failed("Could not zstd decode into buffer".to_string()))?;
        Ok(())
    }
}

#[cfg(feature = "zstd")]
#[derive(Debug, Clone, Copy)]
pub struct ZstdCppDecoder;

#[cfg(feature = "zstd")]
impl Decoder for ZstdCppDecoder {
    fn decode_tile(
        &self,
        buf: &[u8],
        out_buf: &mut [u8],
        tile_opts: &TileOpts,
    ) -> CodingResult<()> {
        let mut decoder = ruzstd::decoding::StreamingDecoder::new(Cursor::new(buf))
            .or_raise(|| CodingError::failed("could not create Zstd decoder".to_string()))?;
        decoder
            .read_exact(out_buf)
            .or_raise(|| CodingError::failed("Could not zstd decode into buffer".to_string()))?;
        Ok(())
    }
}

#[cfg(feature = "jpeg-decoder")]
#[derive(Debug, Clone, Copy)]
pub struct JpegDecoder;

#[cfg(feature = "jpeg-encoder")]
// https://github.com/image-rs/image-tiff/blob/3bfb43e83e31b0da476832067ada68a82b378b7b/src/decoder/image.rs#L389-L450
impl Decoder for JpegDecoder {
    fn decode_tile(
        &self,
        buf: &[u8],
        out_buf: &mut [u8],
        tile_opts: &TileOpts,
    ) -> CodingResult<()> {
        use crate::structs::metadata::tags::PhotometricInterpretation;

        ensure!(
            tile_opts.jpeg_tables.is_none() || buf.len() >= 2,
            CodingError::failed("invalid JPEG tables data".to_string())
        );

        let compressed_length = u64::try_from(buf.len()).unwrap();
        // Construct new jpeg_reader wrapping a SmartReader.
        //
        // JPEG compression in TIFF allows saving quantization and/or huffman tables in one
        // central location. These `jpeg_tables` are simply prepended to the remaining jpeg image data.
        // Because these `jpeg_tables` start with a `SOI` (HEX: `0xFFD8`) or __start of image__ marker
        // which is also at the beginning of the remaining JPEG image data and would
        // confuse the JPEG renderer, one of these has to be taken off. In this case the first two
        // bytes of the remaining JPEG data is removed because it follows `jpeg_tables`.
        // Similary, `jpeg_tables` ends with a `EOI` (HEX: `0xFFD9`) or __end of image__ marker,
        // this has to be removed as well (last two bytes of `jpeg_tables`).
        let reader = Cursor::new(buf);

        let jpeg_reader = match &tile_opts.jpeg_tables {
            Some(jpeg_tables) => {
                let mut reader = reader.take(compressed_length);
                reader
                    .read_exact(&mut [0; 2])
                    .or_raise(|| CodingError::failed("failed to decode into buffer".to_string()))?;

                Box::new(
                    Cursor::new(&jpeg_tables[..jpeg_tables.len() - 2])
                        .chain(reader.take(compressed_length)),
                ) as Box<dyn Read>
            }
            None => Box::new(reader.take(compressed_length)),
        };

        let mut decoder = jpeg::Decoder::new(jpeg_reader);

        match tile_opts.photometric_interpretation {
            PhotometricInterpretation::RGB => {
                decoder.set_color_transform(jpeg::ColorTransform::RGB)
            }
            PhotometricInterpretation::WhiteIsZero => {
                decoder.set_color_transform(jpeg::ColorTransform::None)
            }
            PhotometricInterpretation::BlackIsZero => {
                decoder.set_color_transform(jpeg::ColorTransform::None)
            }
            PhotometricInterpretation::TransparencyMask => {
                decoder.set_color_transform(jpeg::ColorTransform::None)
            }
            PhotometricInterpretation::CMYK => {
                decoder.set_color_transform(jpeg::ColorTransform::CMYK)
            }
            PhotometricInterpretation::YCbCr => {
                decoder.set_color_transform(jpeg::ColorTransform::YCbCr)
            }
            photometric_interpretation => {
                use exn::bail;

                bail!(CodingError::failed(format!(
                    "unsupported photometric interpretation {photometric_interpretation:?}"
                )))
            }
        }

        // copying data, so sad
        let data = decoder
            .decode()
            .or_raise(|| CodingError::failed("JPEG decoding failed".to_string()))?;
        out_buf.copy_from_slice(&data);
        Ok(())
    }
}

#[cfg(feature = "webp-agpl")]
#[derive(Debug, Clone)]
pub struct ZenWebPDecoder;

#[cfg(feature = "webp-agpl")]
impl Decoder for ZenWebPDecoder {
    fn decode_tile(
        &self,
        in_buf: &[u8],
        out_buf: &mut [u8],
        tile_opts: &TileOpts,
    ) -> CodingResult<()> {
        let mut decoder = zenwebp::WebPDecoder::build(in_buf).or_raise(|| {
            CodingError::failed("could not load metadata for webp decoding".to_string())
        })?;
        decoder.read_image(out_buf).or_raise(|| {
            CodingError::incomplete(
                out_buf.len(),
                decoder.output_buffer_size().unwrap_or(std::usize::MAX),
            )
        })
    }
}

#[cfg(feature = "webp-cpp")]
#[derive(Debug, Clone)]
pub struct WebPDecoder;

#[cfg(feature = "webp-cpp")]
impl Decoder for WebPDecoder {
    fn decode_tile(
        &self,
        in_buf: &[u8],
        out_buf: &mut [u8],
        tile_opts: &TileOpts,
    ) -> CodingResult<()> {
        use exn::OptionExt;

        let decoded = webp::Decoder::new(&in_buf)
            .decode()
            .ok_or_raise(|| CodingError::failed("webp decoding failed".to_string()))?;

        if decoded.len() == out_buf.len() {
            out_buf.copy_from_slice(&decoded);
            Ok(())
        } else
        // WebP lossy compression may discard fully-opaque alpha channels.
        // If the TIFF expects 4 samples but WebP decoded to 3, expand RGB to RGBA.
        // Only do this for 8-bit data since WebP only supports 8-bit.
        if tile_opts.samples_per_pixel == 4
            && tile_opts.bits_per_sample == 8
            && !decoded.is_alpha()
        {
            for (rgb, rgba) in decoded.chunks_exact(3).zip(out_buf.chunks_exact_mut(4)) {
                rgba[..3].copy_from_slice(rgb);
                rgba[3] = 255; // opaque alpha
            }
            Ok(())
        } else {
            Err(CodingError::incomplete(decoded.len(), out_buf.len()).into())
        }
    }
}
