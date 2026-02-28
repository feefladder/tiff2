use std::collections::HashMap;
use std::fmt::Debug;
use std::io::{Cursor, Read};

use flate2::bufread::ZlibDecoder;
use weezl::LzwStatus;

use crate::error::{TiffError, TiffFormatError, TiffResult, TiffUnsupportedError};
use crate::loader::tile::ChunkOpts;
use crate::structs::tags::CompressionMethod;

// from async-tiff
/// A registry of decoders.
///
/// This allows end users to register their own decoders, for custom compression methods, or
/// override the default decoder implementations.
///
/// ```
/// use tiff2::loader::DecoderRegistry;
///
/// // Default registry includes Deflate, LZW, JPEG, ZSTD.
/// let registry = DecoderRegistry::default();
/// assert_eq!(registry.0.len(), 4);
///
/// // Empty registry for manual configuration.
/// let empty = DecoderRegistry::empty();
/// assert_eq!(empty.0.len(), 0);
/// ```
#[derive(Debug)]
#[non_exhaustive]
pub struct DecoderRegistry(pub HashMap<CompressionMethod, Box<dyn Decoder>>);

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
        #[cfg(feature = "lerc")]
        registry.insert(CompressionMethod::LERC, Box::new(LercDecoder) as _);
        #[cfg(feature = "lzma")]
        registry.insert(CompressionMethod::LZMA, Box::new(LZMADecoder) as _);
        registry.insert(CompressionMethod::LZW, Box::new(LZWDecoder) as _);
        #[cfg(feature = "jpeg")]
        registry.insert(CompressionMethod::ModernJPEG, Box::new(JpegDecoder) as _);
        #[cfg(feature = "jpeg2k")]
        registry.insert(CompressionMethod::JPEG2k, Box::new(JPEG2kDecoder) as _);
        #[cfg(feature = "webp")]
        registry.insert(CompressionMethod::WebP, Box::new(WebPDecoder) as _);
        #[cfg(feature = "zstd")]
        registry.insert(CompressionMethod::ZSTD, Box::new(ZstdDecoder) as _);
        Self(registry)
    }
}

pub trait Decoder: Debug + Send + Sync {
    fn decode_chunk(
        &self,
        in_buf: &[u8],
        out_buf: &mut [u8],
        chunk_opts: &ChunkOpts,
    ) -> TiffResult<()>;
}

#[derive(Debug, Clone, Copy)]
pub struct UncompressedDecoder;

impl Decoder for UncompressedDecoder {
    fn decode_chunk(
        &self,
        in_buf: &[u8],
        out_buf: &mut [u8],
        _chunk_opts: &ChunkOpts,
    ) -> TiffResult<()> {
        out_buf.copy_from_slice(in_buf);
        Ok(())
    }
}

#[derive(Debug, Clone, Copy)]
pub struct DeflateDecoder;

impl Decoder for DeflateDecoder {
    fn decode_chunk(
        &self,
        in_buf: &[u8],
        out_buf: &mut [u8],
        _chunk_opts: &ChunkOpts,
    ) -> TiffResult<()> {
        let mut decoder = ZlibDecoder::new(Cursor::new(in_buf));
        decoder.read_exact(out_buf)?;
        Ok(())
    }
}

#[derive(Debug, Clone, Copy)]
pub struct LZWDecoder;

impl Decoder for LZWDecoder {
    fn decode_chunk(
        &self,
        in_buf: &[u8],
        out_buf: &mut [u8],
        _chunk_opts: &ChunkOpts,
    ) -> TiffResult<()> {
        // https://github.com/image-rs/image-tiff/blob/90ae5b8e54356a35e266fb24e969aafbcb26e990/src/decoder/stream.rs#L147
        let mut decoder = weezl::decode::Decoder::with_tiff_size_switch(weezl::BitOrder::Msb, 8);
        let res = decoder.decode_bytes(in_buf, out_buf);
        // verify the output
        if res.consumed_out != out_buf.len() || !matches!(res.status?, LzwStatus::Done) {
            Err(TiffError::UnsupportedError(
                TiffUnsupportedError::UnsupportedCompressionMethod(CompressionMethod::LZW),
            ))
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
    fn decode_chunk(
        &self,
        buf: &[u8],
        out_buf: &mut [&mut [u8]],
        chunk_opts: &ChunkOpts,
    ) -> TiffResult<()> {
        let mut decoder = zstd::Decoder::new(Cursor::new(buf))?;
        for out_buf in out_buf {
            decoder.read_exact(out_buf)?;
        }
        Ok(())
    }
}

#[cfg(feature = "jpeg")]
#[derive(Debug, Clone, Copy)]
pub struct JpegDecoder;

#[cfg(feature = "jpeg")]
// https://github.com/image-rs/image-tiff/blob/3bfb43e83e31b0da476832067ada68a82b378b7b/src/decoder/image.rs#L389-L450
impl Decoder for JpegDecoder {
    fn decode_chunk(
        &self,
        buf: &[u8],
        out_buf: &mut [&mut [u8]],
        chunk_opts: &ChunkOpts,
    ) -> TiffResult<()> {
        use crate::structs::tags::PhotometricInterpretation;

        if chunk_opts.jpeg_tables.is_some() && buf.len() < 2 {
            use crate::error::{TiffError, TiffFormatError};
            use crate::structs::Tag;

            return Err(TiffError::FormatError(
                TiffFormatError::InvalidTagValueType(Tag::JPEGTables.to_u16()),
            ));
        }

        let compressed_length =
            u64::try_from(buf.len()).expect("buffer length should fit in usize");
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

        let jpeg_reader = match &chunk_opts.jpeg_tables {
            Some(jpeg_tables) => {
                let mut reader = reader.take(compressed_length);
                reader.read_exact(&mut [0; 2])?;

                Box::new(
                    Cursor::new(&jpeg_tables[..jpeg_tables.len() - 2])
                        .chain(reader.take(compressed_length)),
                ) as Box<dyn Read>
            }
            None => Box::new(reader.take(compressed_length)),
        };

        let mut decoder = jpeg::Decoder::new(jpeg_reader);

        match chunk_opts.photometric_interpretation {
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
                use crate::error::{TiffError, TiffUnsupportedError};

                return Err(TiffError::UnsupportedError(
                    TiffUnsupportedError::UnsupportedInterpretation(photometric_interpretation),
                ));
            }
        }

        // copying data, so sad
        let data = decoder.decode()?;
        out_buf.copy_from_slice(&data[buf_start..buf_start + out_buf.len()]);
        Ok(())
    }
}
