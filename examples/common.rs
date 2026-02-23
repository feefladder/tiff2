use std::ops::Range;
use std::path::{Path, PathBuf};

use async_trait::async_trait;
use bytes::Bytes;
use image::{DynamicImage, ImageBuffer, Luma, LumaA, Rgb, Rgb32FImage, Rgba, Rgba32FImage};
use log::error;
use tiff2::error::{TiffError, TiffResult};
use tiff2::loader::TileData::*;
use tiff2::loader::{CogReader, TileData};
use tiff2::structs::tags::PhotometricInterpretation::*;
use tiff2::structs::ChunkOpts;
use tokio::fs::File;
use tokio::io::{AsyncReadExt, AsyncSeekExt};
use viuer::{print, Config};
pub struct TokioFile(pub PathBuf);

impl TokioFile {
    pub fn new(p: impl AsRef<Path>) -> TiffResult<Self> {
        if p.as_ref().is_file() {
            Ok(Self(p.as_ref().to_owned()))
        } else {
            Err(TiffError::IoError(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("File not found at path {:?}", p.as_ref()),
            )))
        }
    }
}

#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
impl CogReader for TokioFile {
    const IFD_REQ_SIZE: u64 = 16 * 1024;

    async fn get_range(&self, range: Range<u64>) -> TiffResult<Bytes> {
        let mut f = File::open(self.0.clone()).await?;
        f.seek(std::io::SeekFrom::Start(range.start)).await?;
        let len = usize::try_from(range.end - range.start)?;
        let mut buffer = vec![0u8; len];

        // Read the data into the buffer
        f.read_exact(&mut buffer)
            .await
            .map_err(|e| TiffError::TransportError(Box::new(e)))?;
        Ok(Bytes::copy_from_slice(&buffer))
    }
}

#[rustfmt::skip]
#[allow(unused_must_use)]
pub fn img_print(mut img: TileData, chopts: &ChunkOpts) {
    // let phot_int = chopts.photometric_interpretation;
    let spp = chopts.samples_per_pixel();
    let width = chopts.image_width;
    let height = chopts.image_height;
    if let TileData::F32(ref mut v) = img {
        // https://github.com/image-rs/image/pull/2381/files#diff-12d1cf56d5d03c679a5f8f1c48cfe969a5c4276a1bee8647cc548de13f18e465R393
        for f in (*v).iter_mut() {
            *f = if !(*f < 1.0) { 1.0 } else { f.max(0.0) };
        }
    }

    match (img, spp) {
        (U8 (buf) , 1) => {print(&DynamicImage::from(ImageBuffer::<Luma<_>,_>::from_raw(width, height, buf).expect("could not create image")), &Config::default());},
        (U16(buf), 1) => {print(&DynamicImage::from(ImageBuffer::<Luma<_>,_>::from_raw(width, height, buf).expect("could not create image")), &Config::default());},
        // (U32(buf), 1) => {print(&DynamicImage::from(ImageBuffer::<Luma<_>,_>::from_raw(width, height, buf).expect("could not create image")), &Config::default());},
        // (U64(buf), 1) => {print(&DynamicImage::from(ImageBuffer::<Luma<_>,_>::from_raw(width, height, buf).expect("could not create image")), &Config::default());},
        // (I8 (buf) , 1) => {print(&DynamicImage::from(ImageBuffer::<Luma<_>,_>::from_raw(width, height, buf).expect("could not create image")), &Config::default());},
        // (I16(buf), 1) => {print(&DynamicImage::from(ImageBuffer::<Luma<_>,_>::from_raw(width, height, buf).expect("could not create image")), &Config::default());},
        // (I32(buf), 1) => {print(&DynamicImage::from(ImageBuffer::<Luma<_>,_>::from_raw(width, height, buf).expect("could not create image")), &Config::default());},
        // (I64(buf), 1) => {print(&DynamicImage::from(ImageBuffer::<Luma<_>,_>::from_raw(width, height, buf).expect("could not create image")), &Config::default());},
        (F32(buf), 1) => {print(&DynamicImage::from(ImageBuffer::<Luma<_>,_>::from_raw(width, height, buf).expect("could not create image")), &Config::default());},
        // (F64(buf), 1) => {print(&DynamicImage::from(ImageBuffer::<Luma<_>,_>::from_raw(width, height, buf).expect("could not create image")), &Config::default());},

        (U8 (buf) , 2) => {print(&DynamicImage::from(ImageBuffer::<LumaA<_>,_>::from_raw(width, height, buf).expect("could not create image")), &Config::default());},
        (U16(buf), 2) => {print(&DynamicImage::from(ImageBuffer::<LumaA<_>,_>::from_raw(width, height, buf).expect("could not create image")), &Config::default());},
        // (U32(buf), 2) => {print(&DynamicImage::from(ImageBuffer::<LumaA<_>,_>::from_raw(width, height, buf).expect("could not create image")), &Config::default());},
        // (U64(buf), 2) => {print(&DynamicImage::from(ImageBuffer::<LumaA<_>,_>::from_raw(width, height, buf).expect("could not create image")), &Config::default());},
        // (I8 (buf) ,2) => {print(&DynamicImage::from(ImageBuffer::<LumaA<_>,_>::from_raw(width, height, buf).expect("could not create image")), &Config::default());},
        // (I16(buf), 2) => {print(&DynamicImage::from(ImageBuffer::<LumaA<_>,_>::from_raw(width, height, buf).expect("could not create image")), &Config::default());},
        // (I32(buf), 2) => {print(&DynamicImage::from(ImageBuffer::<LumaA<_>,_>::from_raw(width, height, buf).expect("could not create image")), &Config::default());},
        // (I64(buf), 2) => {print(&DynamicImage::from(ImageBuffer::<LumaA<_>,_>::from_raw(width, height, buf).expect("could not create image")), &Config::default());},
        (F32(buf), 2) => {print(&DynamicImage::from(ImageBuffer::<LumaA<_>,_>::from_raw(width, height, buf).expect("could not create image")), &Config::default());},
        // (F64(buf), 2) => {print(&DynamicImage::from(ImageBuffer::<LumaA<_>,_>::from_raw(width, height, buf).expect("could not create image")), &Config::default());},

        (U8 (buf) , 3) => {print(&DynamicImage::from(ImageBuffer::<Rgb<_>,_>::from_raw(width, height, buf).expect("could not create image")), &Config::default());},
        (U16(buf), 3) => {print(&DynamicImage::from(ImageBuffer::<Rgb<_>,_>::from_raw(width, height, buf).expect("could not create image")), &Config::default());},
        // (U32(buf), 3) => {print(&DynamicImage::from(ImageBuffer::<Rgb<_>,_>::from_raw(width, height, buf).expect("could not create image")), &Config::default());},
        // (U64(buf), 3) => {print(&DynamicImage::from(ImageBuffer::<Rgb<_>,_>::from_raw(width, height, buf).expect("could not create image")), &Config::default());},
        // (I8 (buf) , 3) => {print(&DynamicImage::from(ImageBuffer::<Rgb<_>,_>::from_raw(width, height, buf).expect("could not create image")), &Config::default());},
        // (I16(buf), 3) => {print(&DynamicImage::from(ImageBuffer::<Rgb<_>,_>::from_raw(width, height, buf).expect("could not create image")), &Config::default());},
        // (I32(buf), 3) => {print(&DynamicImage::from(ImageBuffer::<Rgb<_>,_>::from_raw(width, height, buf).expect("could not create image")), &Config::default());},
        // (I64(buf), 3) => {print(&DynamicImage::from(ImageBuffer::<Rgb<_>,_>::from_raw(width, height, buf).expect("could not create image")), &Config::default());},
        (F32(buf), 3) => {print(&DynamicImage::from(Rgb32FImage::from_raw(width, height, buf).expect("could not create image")), &Config::default());},
        // (F64(buf), 3) => {print(&DynamicImage::from(ImageBuffer::<Rgb<_>,_>::from_raw(width, height, buf).expect("could not create image")), &Config::default());},

        (U8 (buf) , 4) => {print(&DynamicImage::from(ImageBuffer::<Rgba<_>,_>::from_raw(width, height, buf).expect("could not create image")), &Config::default());},
        (U16(buf), 4) => {print(&DynamicImage::from(ImageBuffer::<Rgba<_>,_>::from_raw(width, height, buf).expect("could not create image")), &Config::default());},
        // (U32(buf), 4) => {print(&DynamicImage::from(ImageBuffer::<Rgba<_>,_>::from_raw(width, height, buf).expect("could not create image")), &Config::default());},
        // (U64(buf), 4) => {print(&DynamicImage::from(ImageBuffer::<Rgba<_>,_>::from_raw(width, height, buf).expect("could not create image")), &Config::default());},
        // (I8 (buf) , 4) => {print(&DynamicImage::from(ImageBuffer::<Rgba<_>,_>::from_raw(width, height, buf).expect("could not create image")), &Config::default());},
        // (I16(buf), 4) => {print(&DynamicImage::from(ImageBuffer::<Rgba<_>,_>::from_raw(width, height, buf).expect("could not create image")), &Config::default());},
        // (I32(buf), 4) => {print(&DynamicImage::from(ImageBuffer::<Rgba<_>,_>::from_raw(width, height, buf).expect("could not create image")), &Config::default());},
        // (I64(buf), 4) => {print(&DynamicImage::from(ImageBuffer::<Rgba<_>,_>::from_raw(width, height, buf).expect("could not create image")), &Config::default());},
        (F32(buf), 4) => {print(&DynamicImage::from(Rgba32FImage::from_raw(width, height, buf).expect("could not create image")), &Config::default());},
        // (F64(buf), 4) => {print(&DynamicImage::from(ImageBuffer::<Rgba<_>,_>::from_raw(width, height, buf).expect("could not create image")), &Config::default());},
        _ => error!("could not show image"),
    }
}

#[allow(unused)]
fn main() {}
