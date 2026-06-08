mod common;
use std::boxed::Box;
use std::fs::File;
use std::sync::Arc;

use exn::ResultExt;
use tiff2::loader::{DecoderRegistry, SyncMetaReader, SyncReader, TiffMetaReader};
use tiff2::structs::metadata::tags::{
    CompressionMethod, PhotometricInterpretation, PlanarConfiguration, Predictor, SampleFormat,
};
use tiff2::structs::{Tag, Tiff, TileCoord, TileOpts};

#[test]
fn test_read_file() {
    let size = 256;
    let fpath = common::setup().unwrap();
    let data = common::test_data(256);
    // read the file with tiff2
    // we basically assume that there is one ifd and it's the first "tile" so to speak
    // there may be a "read whole image"-type convenience function added later
    let mut meta_reader: TiffMetaReader<File, Tiff> = TiffMetaReader::open(
        File::open(fpath).unwrap(),
        1024,
        Arc::new(Vec::new().into()),
    )
    .unwrap();
    let ifd_offset = meta_reader.loader.next_ifd_offset().unwrap();
    while meta_reader.next().unwrap().is_some() {}
    let ifd = meta_reader.loader.ifd(0);
    eprintln!(
        "{:?}, {:?}",
        ifd.tags[&Tag::StripOffsets],
        ifd.tags[&Tag::StripByteCounts]
    );
    let mut reader = meta_reader.finish(DecoderRegistry::default());
    reader.prep_ifd(0).unwrap();
    let tloader = &reader.tile_loaders[&ifd_offset];
    eprintln!("{:?}, {:?}", tloader.tile_offsets, tloader.tile_byte_counts);
    // sanity check how gdal creates its "default" tiff file
    // interesting how rows_per_strip is set to 8?
    assert_eq!(
        reader.tile_opts(0).unwrap(),
        &TileOpts {
            byte_order: tiff2::ByteOrder::LittleEndian,
            image_width: size,
            image_height: size,
            bits_per_sample: 32,
            samples_per_pixel: 1,
            sample_format: SampleFormat::IEEEFP,
            photometric_interpretation: PhotometricInterpretation::BlackIsZero,
            compression_method: CompressionMethod::LZW,
            predictor: Predictor::FloatingPoint,
            jpeg_tables: None,
            planar_config: PlanarConfiguration::Chunky,
            tile_width: 256,
            tile_height: 8,
        }
    );
    let mut coords = Vec::with_capacity(256 / 8);
    for y in 0..256 / 8 {
        coords.push(TileCoord::from((0, y)));
    }
    let mut result_data = vec![0.0f32; (size * size) as usize];
    for (idx, buf) in result_data.chunks_exact_mut(256 * 8).enumerate() {
        reader
            .get_tile_into(bytemuck::cast_slice_mut(buf), 0, (0, idx as u32).into())
            .unwrap();
    }
    assert_eq!(result_data, data);
    // assert_eq!(2 + 2, 5);
}
