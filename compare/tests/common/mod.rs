use std::boxed::Box;
use std::error::Error;
use std::io::{Seek, SeekFrom};
use std::os::unix::fs::FileExt;
use std::path::PathBuf;

use bytes::Bytes;
use compare::tiff_dir;
use exn::ResultExt;
use gdal::DriverManager;
use gdal::raster::{Buffer, RasterCreationOption};
use tiff2::loader::FetchError;

pub fn test_data(size: usize) -> Vec<f32> {
    let start = 0.0;
    let delta = 0.25;
    let mut arr = vec![start; size * size];
    for (i, v) in arr.iter_mut().enumerate() {
        *v += delta * i as f32;
    }
    arr
}

/// Setup code that writes a boring tiff with gdal
pub fn setup() -> Result<PathBuf, Box<dyn Error>> {
    // 1. Build a linearly increasing array of f32
    let size = 256;
    let tiff_dir = tiff_dir()?;
    let arr = test_data(size);

    let options = [
        RasterCreationOption {
            key: "COMPRESS",
            value: "LZW",
        },
        RasterCreationOption {
            key: "PREDICTOR",
            value: "3",
        }
        .into(),
        RasterCreationOption {
            key: "BLOCKSIZE",
            value: "16",
        },
    ];

    // 2. Write to GeoTIFF
    let driver = DriverManager::get_driver_by_name("GTiff")?;
    let mut dataset = driver.create_with_band_type_with_options::<f32, PathBuf>(
        tiff_dir.join("output.tiff"),
        size as _,
        size as _,
        1,
        &options,
    )?;

    let mut band = dataset.rasterband(1)?;
    let buffer = Buffer {
        size: (size, size),
        data: arr,
    };
    band.write((0, 0), (size, size), &buffer)?;
    dataset.set_geo_transform(&[0.0, 1.0, 0.0, 0.0, 0.0, -1.0])?; // Simple transform

    let _cog_ds = dataset.create_copy(
        &DriverManager::get_driver_by_name("COG")?,
        tiff_dir.join("output-cog.tiff"),
        &options,
    )?;

    Ok(tiff_dir.join("output.tiff"))
}
