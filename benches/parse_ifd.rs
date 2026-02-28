use std::collections::BTreeMap;
use std::hint::black_box;
use std::io::{self, Read};

use criterion::{criterion_group, criterion_main, Criterion};
use rand::{RngExt, SeedableRng};
use tiff2::loader::IfdLoader;
use tiff2::structs::{IfdEntry, Tag};
use tiff2::ByteOrder;

fn parse_ifd_tag_data(
    buf: &[u8],
    byte_order: ByteOrder,
    bigtiff: bool,
) -> (BTreeMap<Tag, IfdEntry>, u64) {
    let (loader, next_offset) = IfdLoader::from_buffer(&buf, 0, bigtiff, byte_order).unwrap();
    (loader.ifd.data, next_offset)
}

fn benchmark_parse_ifd(c: &mut Criterion) {
    let n_entries_sizes = (1..4).map(|v| 4u64.pow(v));
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
                    cursor.write_u16(rng.random()).unwrap();
                    cursor.write_u16((rng.random::<u16>() % 12) + 2).unwrap();
                    if bigtiff {
                        cursor.write_u64(1).unwrap();
                        cursor.write_u64(rng.random()).unwrap()
                    } else {
                        cursor.write_u32(1).unwrap();
                        cursor.write_u32(rng.random()).unwrap();
                    }
                }
                c.bench_function(
                    &format!("td_parse_ifd_{n_entries}_{bigtiff:?}_{byte_order:?}"),
                    |bencher| {
                        bencher.iter(|| {
                            parse_ifd_tag_data(
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

/// Reader that is aware of the byte order
/// TODO: **deprecate** in favour of chunk-based approach in `Entry` and `Ifd`
pub struct EndianReader<R> {
    pub reader: R,
    pub byte_order: ByteOrder,
}

impl<R: io::Read> io::Read for EndianReader<R> {
    #[inline]
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.reader.read(buf)
    }
}

impl<R: io::Seek> io::Seek for EndianReader<R> {
    #[inline]
    fn seek(&mut self, pos: io::SeekFrom) -> io::Result<u64> {
        self.reader.seek(pos)
    }
}

impl<W: io::Write> io::Write for EndianReader<W> {
    #[inline]
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.reader.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.reader.flush()
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

macro_rules! write_fn {
    ($name:ident, $type:ty) => {
        /// reads an $type, respecting byte order
        #[inline(always)]
        pub fn $name(&mut self, val: $type) -> Result<(), io::Error> {
            let bytes = match self.byte_order() {
                ByteOrder::LittleEndian => <$type>::to_le_bytes(val),
                ByteOrder::BigEndian => <$type>::to_be_bytes(val),
            };
            self.reader.write_all(&bytes)
        }
    };
}

impl<R> EndianReader<R> {
    /// Wraps a reader
    pub fn wrap(reader: R, byte_order: ByteOrder) -> Self {
        EndianReader { reader, byte_order }
    }

    fn byte_order(&self) -> ByteOrder {
        self.byte_order
    }
}
impl<R: std::io::Read> EndianReader<R> {
    read_fn!(read_u8, u8);
    read_fn!(read_u16, u16);
    read_fn!(read_u32, u32);
    read_fn!(read_u64, u64);
}
impl<R: io::Write> EndianReader<R> {
    write_fn!(write_u8, u8);
    write_fn!(write_u16, u16);
    write_fn!(write_u32, u32);
    write_fn!(write_u64, u64);
}
