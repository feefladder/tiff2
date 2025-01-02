use std::error::Error;
use std::fmt;
use std::fmt::Display;
use std::io;
use std::str;
use std::string;
use std::sync::Arc;

use jpeg::UnsupportedFeature;
use thiserror::Error;
use weezl::LzwError;

use crate::{
    structs::{
        tags::{
            CompressionMethod, PhotometricInterpretation, PlanarConfiguration, SampleFormat, Tag,
        },
        Offset, TagData,
    },
    ChunkType, ColorType,
};

/// Result of an image decoding/encoding process
pub type TiffResult<T> = Result<T, TiffError>;

/// Tiff error kinds.
#[derive(Debug, Error)]
pub enum TiffError {
    /// The Image is not formatted properly.
    #[error("{0}")]
    FormatError(#[from] TiffFormatError),

    /// The Decoder does not support features required by the image.
    #[error("The Decoder does not support the image format `{0}`")]
    UnsupportedError(#[from] TiffUnsupportedError),

    /// An I/O Error occurred while decoding the image.
    #[error("{0}")]
    IoError(#[from] io::Error),
    /// The Limits of the Decoder is exceeded.
    #[error("The Decoder limits are exceeded")]
    LimitsExceeded,

    /// An integer conversion to or from a platform size failed, either due to
    /// limits of the platform size or limits of the format.
    #[error("Failed integer conversion {0}")]
    IntSizeError(#[from] std::num::TryFromIntError),

    /// The image does not support the requested operation
    #[error("Usage error: {0}")]
    UsageError(#[from] UsageError),

    /// Error from transport
    #[error("Transport error: {0}")]
    TransportError(Box<dyn Error + Send + Sync + 'static>),
}

/// The image is not formatted properly.
///
/// This indicates that the encoder producing the image might behave incorrectly or that the input
/// file has been corrupted.
///
/// The list of variants may grow to incorporate errors of future features. Matching against this
/// exhaustively is not covered by interface stability guarantees.
#[derive(Debug, Clone, PartialEq, Error)]
#[non_exhaustive]
pub enum TiffFormatError {
    #[error("TIFF signature not found.")]
    TiffSignatureNotFound,
    #[error("TIFF signature invalid.")]
    TiffSignatureInvalid,
    #[error("Image file directory not found.")]
    ImageFileDirectoryNotFound,
    #[error("Inconsistent sizes encountered. {0:?}")]
    InconsistentSizesEncountered(TagData),
    #[error("Decompression returned different amount of bytes than expected: got {actual_bytes}, expected {required_bytes}.")]
    UnexpectedCompressedData {
        actual_bytes: usize,
        required_bytes: usize,
    },
    #[error("Inconsistent elements in strip: got {actual_samples}, expected {required_samples}.")]
    InconsistentStripSamples {
        actual_samples: usize,
        required_samples: usize,
    },
    #[error("Invalid dimensions: {0}x{1}.")]
    InvalidDimensions(u32, u32),
    #[error("Image contains invalid tag.")]
    InvalidTag,
    #[error("Tag `{0:?}` did not have the expected value type.")]
    InvalidTagValueType(u16),
    #[error("Required tag `{0:?}` not found.")]
    RequiredTagNotFound(Tag),
    #[error("Unknown predictor `{0}` encountered")]
    UnknownPredictor(u16),
    #[error("Unknown planar configuration `{0}` encountered")]
    UnknownPlanarConfiguration(u16),
    // #[error("Expected byte, {0:?} found.")]
    // ByteExpected(TagData),
    // #[error("Expected signed byte, {0:?} found.")]
    // SignedByteExpected(TagData),
    // #[error("Expected signed short, {0:?} found.")]
    // SignedShortExpected(TagData),
    #[error("Expected unsigned integer, {0:?} found.")]
    UnsignedIntegerExpected(TagData),
    #[error("Expected signed integer, {0:?} found.")]
    SignedIntegerExpected(TagData),
    #[error("Expected float or double, {0:?} found")]
    FloatExpected(TagData),
    #[error("Expected Ascii, Byte or Undefined, {0:?} found")]
    AsciiExpected(TagData),
    #[error("Expected Rational, {0:?} found")]
    RationalExpected(TagData),
    #[error("Expected signed rational, {0:?} found")]
    SignedRationalExpected(TagData),
    #[error("Invalid format: {0:?}.")]
    Format(String),
    #[error("Required tag {0:?} was empty.")]
    RequiredTagEmpty(Tag),
    #[error("File should contain either (StripByteCounts and StripOffsets) or (TileByteCounts and TileOffsets), other combination was found.")]
    StripTileTagConflict,
    #[error("File contained a cycle in the list of IFDs")]
    CycleInOffsets,
    #[error("{0}")]
    JpegDecoder(#[from] JpegDecoderError),
    #[error("Samples per pixel is zero")]
    SamplesPerPixelIsZero,
    #[error("output row stride is larger than chunk width (in bits")]
    RowStrideLargerThanWidth(usize, usize),
}

/// The Decoder does not support features required by the image.
///
/// This only captures known failures for which the standard either does not require support or an
/// implementation has been planned but not yet completed. Some variants may become unused over
/// time and will then get deprecated before being removed.
///
/// The list of variants may grow. Matching against this exhaustively is not covered by interface
/// stability guarantees.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Error)]
#[non_exhaustive]
pub enum TiffUnsupportedError {
    #[error("Floating point predictor for {0:?} is unsupported.")]
    FloatingPointPredictor(ColorType),
    #[error("Horizontal predictor for {0:?} is unsupported.")]
    HorizontalPredictor(ColorType),
    #[error("Inconsistent bits per sample: {0:?}.")]
    InconsistentBitsPerSample(Vec<u8>),
    #[error("{0:?} with {0:?} bits per sample is unsupported")]
    InterpretationWithBits(PhotometricInterpretation, Vec<u8>),
    #[error("The image is using an unknown photometric interpretation.")]
    UnknownInterpretation,
    #[error("Unknown compression method.")]
    UnknownCompressionMethod,
    #[error("Compression method {0:?} is unsupported")]
    UnsupportedCompressionMethod(CompressionMethod),
    #[error("{0} samples per pixel is unsupported.")]
    UnsupportedSampleDepth(u8),
    #[error("Sample format {0:?} is unsupported.")]
    UnsupportedSampleFormat(Vec<SampleFormat>),
    #[error("Color type {0:?} is unsupported")]
    UnsupportedColorType(ColorType),
    #[error("{0} bits per channel not supported")]
    UnsupportedBitsPerChannel(u8),
    #[error("Unsupported planar configuration “{0:?}”.")]
    UnsupportedPlanarConfig(Option<PlanarConfiguration>),
    #[error("Unsupported data type.")]
    UnsupportedDataType,
    #[error("Unsupported photometric interpretation \"{0:?}\".")]
    UnsupportedInterpretation(PhotometricInterpretation),
    #[error("Unsupported JPEG feature {0:?}")]
    UnsupportedJpegFeature(UnsupportedFeature),
    #[error("Tile rows are not aligned to byte boundaries")]
    MisalignedTileBoundaries,
}

/// User attempted to use the Decoder in a way that is incompatible with a specific image.
///
/// For example: attempting to read a tile from a stripped image.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum UsageError {
    #[error("Requested operation is only valid for images with chunk encoding of type: {0:?}, got {0:?}.")]
    InvalidChunkType(ChunkType, ChunkType),
    #[error("Image chunk index ({0}) requested.")]
    InvalidChunkIndex(u32),
    #[error("The requested predictor is not compatible with the requested compression")]
    PredictorCompressionMismatch,
    #[error("The requested predictor is not compatible with the image's format")]
    PredictorIncompatible,
    #[error("The requested predictor is not available")]
    PredictorUnavailable,
    #[error("Tried loading tag data into an IFD, while it was already present")]
    DuplicateTagData,
    #[error("Tried to add data to an IFD that didn't have this tag: {0:?}")]
    TagOfDataNotPresent(Tag),
    #[error("Required tag {0:?} with type {:?} and count {} not loaded from {}", .1.tag_type, .1.count, .1.offset)]
    RequiredTagNotLoaded(Tag, Offset),
    #[error("Overview level {0} not in tiff")]
    OverviewNotLoaded(usize),
    #[error("Ifd at offset {0} is not an image or not fully loaded")]
    NotAnImage(u64),
}

impl From<str::Utf8Error> for TiffError {
    fn from(_err: str::Utf8Error) -> TiffError {
        TiffError::FormatError(TiffFormatError::InvalidTag)
    }
}

impl From<string::FromUtf8Error> for TiffError {
    fn from(_err: string::FromUtf8Error) -> TiffError {
        TiffError::FormatError(TiffFormatError::InvalidTag)
    }
}

impl From<LzwError> for TiffError {
    fn from(err: LzwError) -> TiffError {
        match err {
            LzwError::InvalidCode => TiffError::FormatError(TiffFormatError::Format(String::from(
                "LZW compressed data corrupted",
            ))),
        }
    }
}

#[derive(Debug, Clone, Error)]
pub struct JpegDecoderError {
    inner: Arc<jpeg::Error>,
}

impl JpegDecoderError {
    fn new(error: jpeg::Error) -> Self {
        Self {
            inner: Arc::new(error),
        }
    }
}

impl PartialEq for JpegDecoderError {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.inner, &other.inner)
    }
}

impl Display for JpegDecoderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.inner.fmt(f)
    }
}

impl From<JpegDecoderError> for TiffError {
    fn from(error: JpegDecoderError) -> Self {
        TiffError::FormatError(TiffFormatError::JpegDecoder(error))
    }
}

impl From<jpeg::Error> for TiffError {
    fn from(error: jpeg::Error) -> Self {
        JpegDecoderError::new(error).into()
    }
}
