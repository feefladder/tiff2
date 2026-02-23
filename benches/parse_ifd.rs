use std::io::{self, Seek};

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use rand::{Rng, SeedableRng};
use rayon::prelude::*;
use tiff2::error::TiffResult;
use tiff2::loader::EndianReader;
use tiff2::structs::{Directory, IfdEntry, Tag};
use tiff2::ByteOrder;

fn parse_ifd_serial(buf: &[u8], byte_order: ByteOrder, bigtiff: bool) -> (Directory, u64) {
    // maybe make this a parameter (num_entries), since we'd need to read
    // that in order for us to get the correct range of the IFD
    let mut r = EndianReader::wrap(io::Cursor::new(buf), byte_order);
    let n_entries_byte_size: u64 = if bigtiff { 8 } else { 2 };
    let entry_byte_size: u64 = if bigtiff {
        2 + 2 + 8 + 8
    } else {
        2 + 2 + 4 + 4
    };
    let num_entries: u64 = if bigtiff {
        r.read_u64().unwrap()
    } else {
        r.read_u16().unwrap().into()
    };
    assert_eq!(r.stream_position().unwrap(), n_entries_byte_size);
    if (buf.len() as u64) < r.stream_position().unwrap() + num_entries * entry_byte_size {
        panic!(
            "buffer of size {} not big enough for {num_entries}",
            buf.len()
        );
    }

    let mut directory = Directory::with_capacity(num_entries as usize);
    for _idx in 0..num_entries {
        // this can be exhaustive because we have `unknown` enum value
        let tag = Tag::from_u16_exhaustive(r.read_u16().unwrap());
        directory.insert(tag, IfdEntry::from_reader(&mut r, bigtiff).unwrap());
    }
    assert_eq!(
        r.stream_position().unwrap(),
        n_entries_byte_size + entry_byte_size * num_entries
    );
    let next = if bigtiff {
        r.read_u64().unwrap()
    } else {
        r.read_u32().unwrap().into()
    };
    (directory, next)
}

fn parse_ifd_parallel(buf: &[u8], byte_order: ByteOrder, bigtiff: bool) -> (Directory, u64) {
    // maybe make this a parameter (num_entries), since we'd need to read
    // that in order for us to get the correct range of the IFD
    let mut r = EndianReader::wrap(io::Cursor::new(buf), byte_order);
    let n_entries_byte_size: u64 = if bigtiff { 8 } else { 2 };
    let entry_byte_size: u64 = if bigtiff {
        2 + 2 + 8 + 8
    } else {
        2 + 2 + 4 + 4
    };
    let num_entries: u64 = if bigtiff {
        r.read_u64().unwrap()
    } else {
        r.read_u16().unwrap().into()
    };

    let directory = buf[n_entries_byte_size.try_into().unwrap()..]
        .par_chunks_exact(entry_byte_size.try_into().unwrap())
        .take(num_entries.try_into().unwrap())
        .map(|chunk| {
            let mut r = EndianReader::wrap(io::Cursor::new(chunk), byte_order);
            // this can be exhaustive because we have `unknown` enum value
            let tag = Tag::from_u16_exhaustive(r.read_u16()?);
            Ok((tag, IfdEntry::from_reader(&mut r, bigtiff)?))
        })
        .collect::<TiffResult<Directory>>()
        .unwrap();
    r.seek(io::SeekFrom::Start(
        n_entries_byte_size + entry_byte_size * num_entries,
    ))
    .unwrap();
    let next = if bigtiff {
        r.read_u64().unwrap()
    } else {
        r.read_u32().unwrap().into()
    };
    (directory, next)
}

fn benchmark_parse_ifd(c: &mut Criterion) {
    let n_entries_sizes = (8..10).map(|v| 4u64.pow(v));
    let bigtiffs = [true, false];
    let byte_orders = [ByteOrder::BigEndian, ByteOrder::LittleEndian];

    for n_entries in n_entries_sizes {
        for bigtiff in bigtiffs {
            let n_entries_byte_size: u64 = if bigtiff { 8 } else { 2 };
            if !bigtiff && n_entries > u16::MAX as u64 {
                continue;
            }
            let entry_byte_size: u64 = if bigtiff {
                2 + 2 + 8 + 8
            } else {
                2 + 2 + 4 + 4
            };
            let next_offset_byte_size = if bigtiff { 8 } else { 4 };
            for byte_order in byte_orders {
                let mut rng = rand::rngs::StdRng::seed_from_u64(
                    n_entries + if bigtiff { 1 } else { 0 } + byte_order as u64,
                );
                let mut buf =
                    vec![
                        0;
                        (n_entries_byte_size + n_entries * entry_byte_size + next_offset_byte_size)
                            .try_into()
                            .unwrap()
                    ];
                let mut cursor = EndianReader::wrap(std::io::Cursor::new(&mut buf), byte_order);
                if bigtiff {
                    cursor.write_u64(n_entries).unwrap()
                } else {
                    cursor.write_u16(n_entries.try_into().unwrap()).unwrap()
                }
                for _ in 0..n_entries {
                    cursor.write_u16(rng.gen()).unwrap();
                    cursor.write_u16((rng.gen::<u16>() % 12) + 2).unwrap();
                    if bigtiff {
                        cursor.write_u64(1).unwrap();
                        cursor.write_u64(rng.gen()).unwrap()
                    } else {
                        cursor.write_u32(1).unwrap();
                        cursor.write_u32(rng.gen()).unwrap();
                    }
                }
                c.bench_function(
                    &format!("par_parse_ifd_{n_entries}_{bigtiff:?}_{byte_order:?}"),
                    |bencher| {
                        bencher.iter(|| {
                            parse_ifd_parallel(
                                black_box(&buf),
                                black_box(byte_order),
                                black_box(bigtiff),
                            );
                        });
                    },
                );
                c.bench_function(
                    &format!("parse_ifd_{n_entries}_{bigtiff:?}_{byte_order:?}"),
                    |bencher| {
                        bencher.iter(|| {
                            parse_ifd_serial(
                                black_box(&buf),
                                black_box(byte_order),
                                black_box(bigtiff),
                            );
                        });
                    },
                );
            }
        }
    }
}

criterion_group!(benches, benchmark_parse_ifd);
criterion_main!(benches);
