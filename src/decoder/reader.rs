use crate::{
    error::TiffResult,
    structs::{Offset, Tag, TagData},
    util::fix_endianness,
    ByteOrder, NATIVE_ENDIAN,
};

use async_trait::async_trait;
use bytes::Bytes;
use futures::{stream::StreamExt, TryStreamExt};
use log::debug;
use std::{
    collections::BTreeMap,
    io::{self, BufRead, BufReader, Read, Take},
    num::TryFromIntError,
    ops::Range,
};

/// Trait for a CogReader to implement.
///
/// In fact these functions can be all the same, but caching can be optimized based on which part of the tiff we're reading in.
#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
#[allow(clippy::single_range_in_vec_init)]
pub trait CogReader: Sync {
    /// Default buffer/request size for fetching ifd data. Should be in the
    /// order of 16-128 kB. ChatGPT says COGs' tag data generally fits within
    /// the first 16 kB. other data is currently not checked for fitting in the
    /// user-space buffer, so there's not much of a point in making this much larger.
    const IFD_REQ_SIZE: u64;
    // https://blog.rust-lang.org/2023/12/21/async-fn-rpit-in-traits.html#where-the-gaps-lie
    /// Read an ifd. Ideally, this would
    // async fn read_ifd(&self, byte_start: u64) -> TiffResult<Bytes> {
    //     // Bytes is cheaply cloneable
    //     self.get_ranges(&[byte_start..byte_start + Self::IFD_REQ_SIZE])
    //         .await
    //         .map(|v| v[0].clone())
    // }
    // async fn read_tag_data(&self, byte_start: u64, n_bytes: u64) -> TiffResult<Bytes> {
    //     // Bytes is cheaply cloneable
    //     self.get_ranges(&[byte_start..byte_start + n_bytes])
    //         .await
    //         .map(|v| v[0].clone())
    // }
    // async fn read_image_data(&self, byte_start: u64, n_bytes: u64) -> TiffResult<Bytes> {
    //     // Bytes is cheaply cloneable
    //     self.get_ranges(&[byte_start..byte_start + n_bytes])
    //         .await
    //         .map(|v| v[0].clone())
    // }
    async fn get_range(&self, range: Range<u64>) -> TiffResult<Bytes>;
}

#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
pub trait CogReaderExt: CogReader {
    async fn get_ranges(&self, ranges: &[Range<u64>]) -> TiffResult<Vec<Bytes>> {
        coalesce_ranges(
            ranges,
            |range| self.get_range(range),
            OBJECT_STORE_COALESCE_DEFAULT,
        )
        .await
    }
    /// get tags _and fix endianness_
    ///
    /// TODO: should we move endianness-fixing downstream, so this is easier to implement?
    async fn get_tags(
        &self,
        tags: BTreeMap<Tag, Offset>,
        byte_order: ByteOrder,
    ) -> TiffResult<BTreeMap<Tag, TagData>> {
        let mut ranges = Vec::new();
        for offset in tags.values() {
            ranges.push(
                offset.offset
                    ..offset.offset + offset.count * u64::try_from(offset.tag_type.size())?,
            );
        }
        let resp = self.get_ranges(&ranges).await?;
        debug!(
            "received data, lengths: {:?}",
            resp.iter().map(|v| v.len()).collect::<Vec<_>>()
        );
        let mut res = BTreeMap::new();
        // BTreeMap keeps order, so we can safely iterate over that again
        for ((tag, offset), buffer) in tags.iter().zip(&resp) {
            let e = TagData::from_buffer(
                &buffer,
                offset.tag_type,
                offset.count.try_into().unwrap(),
                byte_order,
            )?;
            res.insert(*tag, e);
        }
        // debug!("Received tags: {res:?}");
        Ok(res)
    }
    /// get compressed chunks of data
    ///
    /// actually a bit redundant, since we have get_ranges, which is the same
    async fn get_chunks(&self, chunks: &[Range<u64>]) -> TiffResult<Vec<Bytes>> {
        self.get_ranges(chunks).await
    }
}

impl<T: CogReader> CogReaderExt for T {}

/// Range requests with a gap less than or equal to this,
/// will be coalesced into a single request by [`coalesce_ranges`]
pub const OBJECT_STORE_COALESCE_DEFAULT: u64 = 1024 * 1024;

/// Up to this number of range requests will be performed in parallel by [`coalesce_ranges`]
pub(crate) const OBJECT_STORE_COALESCE_PARALLEL: usize = 10;

/// Takes a function `fetch` that can fetch a range of bytes and uses this to
/// fetch the provided byte `ranges`
///
/// To improve performance it will:
///
/// * Combine ranges less than `coalesce` bytes apart into a single call to `fetch`
/// * Make multiple `fetch` requests in parallel (up to maximum of 10)
pub async fn coalesce_ranges<F, E, Fut>(
    ranges: &[Range<u64>],
    fetch: F,
    coalesce: u64,
) -> Result<Vec<Bytes>, E>
where
    F: Send + FnMut(Range<u64>) -> Fut,
    E: Send + From<TryFromIntError>,
    Fut: std::future::Future<Output = Result<Bytes, E>> + Send,
{
    let fetch_ranges = merge_ranges(ranges, coalesce);

    let fetched: Vec<_> = futures::stream::iter(fetch_ranges.iter().cloned())
        .map(fetch)
        .buffered(OBJECT_STORE_COALESCE_PARALLEL)
        .try_collect()
        .await?;

    ranges
        .iter()
        .map(|range| {
            let idx = fetch_ranges.partition_point(|v| v.start <= range.start) - 1;
            let fetch_range = &fetch_ranges[idx];
            let fetch_bytes = &fetched[idx];

            let start = range.start - fetch_range.start;
            let end = range.end - fetch_range.start;
            Ok(fetch_bytes
                .slice(usize::try_from(start)?..usize::try_from(end)?.min(fetch_bytes.len())))
        })
        .collect::<Result<_, _>>()
}

/// Returns a sorted list of ranges that cover `ranges`
fn merge_ranges(ranges: &[Range<u64>], coalesce: u64) -> Vec<Range<u64>> {
    if ranges.is_empty() {
        return vec![];
    }

    let mut ranges = ranges.to_vec();
    ranges.sort_unstable_by_key(|range| range.start);

    let mut ret = Vec::with_capacity(ranges.len());
    let mut start_idx = 0;
    let mut end_idx = 1;

    while start_idx != ranges.len() {
        let mut range_end = ranges[start_idx].end;

        while end_idx != ranges.len()
            && ranges[end_idx]
                .start
                .checked_sub(range_end)
                .map(|delta| delta <= coalesce)
                .unwrap_or(true)
        {
            range_end = range_end.max(ranges[end_idx].end);
            end_idx += 1;
        }

        let start = ranges[start_idx].start;
        let end = range_end;
        ret.push(start..end);

        start_idx = end_idx;
        end_idx += 1;
    }

    ret
}

// #[cfg(feature="object_store")]
// #[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
// #[cfg_attr(not(target_arch = "wasm32"), async_trait)]
// impl<T: object_store::ObjectStore> CogReader for T {
//     async fn get_ranges(&self, ranges: &[Range<u64>]) -> TiffResult<Vec<Bytes>> {
//         object_store::ObjectStore::get_ranges(self, &ranges)
//     }
// }

/// Reader that is aware of the byte order
/// TODO: **deprecate** in favour of chunk-based approach in `Entry` and `Ifd`
pub struct EndianReader<R> {
    pub(super) reader: R,
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
    // read_fn!(read_i8, i8);
    read_fn!(read_u16, u16);
    // read_fn!(read_i16, i16);
    read_fn!(read_u32, u32);
    // read_fn!(read_i32, i32);
    read_fn!(read_u64, u64);
    // read_fn!(read_i64, i64);

    // read_fn!(read_f32, f32);
    // read_fn!(read_f64, f64);
}
impl<R: io::Write> EndianReader<R> {
    write_fn!(write_u8, u8);
    // write_fn!(write_i8, i8);
    write_fn!(write_u16, u16);
    // write_fn!(write_i16, i16);
    write_fn!(write_u32, u32);
    // write_fn!(write_i32, i32);
    write_fn!(write_u64, u64);
    // write_fn!(write_i64, i64);

    // write_fn!(write_f32, f32);
    // write_fn!(write_f64, f64);
}

// impl<R: io::Write> EndianReader<R> {
//     pub fn write_u64(&mut self, val: u64) -> io::Result<()> {
//         let bytes = match self.byte_order {
//             ByteOrder::LittleEndian => u64::to_le_bytes(val),
//             ByteOrder::BigEndian => u64::to_be_bytes(val),
//         };
//         self.reader.write_all(&bytes)
//     }
// }

// ///
// /// # READERS
// ///

///
/// ## Deflate Reader
pub type DeflateReader<R> = flate2::read::ZlibDecoder<R>;

// ///
// /// ## LZW Reader
// ///
/// Reader that decompresses LZW streams
pub struct LZWReader<R: Read> {
    reader: BufReader<Take<R>>,
    decoder: weezl::decode::Decoder,
}

impl<R: Read> LZWReader<R> {
    /// Wraps a reader
    pub fn new(reader: R, compressed_length: usize) -> LZWReader<R> {
        Self {
            reader: BufReader::with_capacity(
                (32 * 1024).min(compressed_length),
                reader.take(u64::try_from(compressed_length).unwrap()),
            ),
            decoder: weezl::decode::Decoder::with_tiff_size_switch(weezl::BitOrder::Msb, 8),
        }
    }
}

impl<R: Read> Read for LZWReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        loop {
            let result = self.decoder.decode_bytes(self.reader.fill_buf()?, buf);
            self.reader.consume(result.consumed_in);

            match result.status {
                Ok(weezl::LzwStatus::Ok) => {
                    if result.consumed_out == 0 {
                        continue;
                    } else {
                        return Ok(result.consumed_out);
                    }
                }
                Ok(weezl::LzwStatus::NoProgress) => {
                    assert_eq!(result.consumed_in, 0);
                    assert_eq!(result.consumed_out, 0);
                    assert!(self.reader.buffer().is_empty());
                    return Err(io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        "no lzw end code found",
                    ));
                }
                Ok(weezl::LzwStatus::Done) => {
                    return Ok(result.consumed_out);
                }
                Err(err) => return Err(io::Error::new(io::ErrorKind::InvalidData, err)),
            }
        }
    }
}

// ///
// /// ## PackBits Reader
// ///
enum PackBitsReaderState {
    Header,
    Literal,
    Repeat { value: u8 },
}

/// Reader that unpacks Apple's `PackBits` format
pub struct PackBitsReader<R: Read> {
    reader: Take<R>,
    state: PackBitsReaderState,
    count: usize,
}

impl<R: Read> PackBitsReader<R> {
    /// Wraps a reader
    pub fn new(reader: R, length: u64) -> Self {
        Self {
            reader: reader.take(length),
            state: PackBitsReaderState::Header,
            count: 0,
        }
    }
}

impl<R: Read> Read for PackBitsReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        while let PackBitsReaderState::Header = self.state {
            if self.reader.limit() == 0 {
                return Ok(0);
            }
            let mut header: [u8; 1] = [0];
            self.reader.read_exact(&mut header)?;
            let h = header[0] as i8;
            if (-127..=-1).contains(&h) {
                let mut data: [u8; 1] = [0];
                self.reader.read_exact(&mut data)?;
                self.state = PackBitsReaderState::Repeat { value: data[0] };
                self.count = (1 - h as isize) as usize;
            } else if h >= 0 {
                self.state = PackBitsReaderState::Literal;
                self.count = h as usize + 1;
            } else {
                // h = -128 is a no-op.
            }
        }

        let length = buf.len().min(self.count);
        let actual = match self.state {
            PackBitsReaderState::Literal => self.reader.read(&mut buf[..length])?,
            PackBitsReaderState::Repeat { value } => {
                for b in &mut buf[..length] {
                    *b = value;
                }

                length
            }
            PackBitsReaderState::Header => unreachable!(),
        };

        self.count -= actual;
        if self.count == 0 {
            self.state = PackBitsReaderState::Header;
        }
        Ok(actual)
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_packbits() {
        let encoded = vec![
            0xFE, 0xAA, 0x02, 0x80, 0x00, 0x2A, 0xFD, 0xAA, 0x03, 0x80, 0x00, 0x2A, 0x22, 0xF7,
            0xAA,
        ];
        let encoded_len = encoded.len();

        let buff = io::Cursor::new(encoded);
        let mut decoder = PackBitsReader::new(buff, encoded_len as u64);

        let mut decoded = Vec::new();
        decoder.read_to_end(&mut decoded).unwrap();

        let expected = vec![
            0xAA, 0xAA, 0xAA, 0x80, 0x00, 0x2A, 0xAA, 0xAA, 0xAA, 0xAA, 0x80, 0x00, 0x2A, 0x22,
            0xAA, 0xAA, 0xAA, 0xAA, 0xAA, 0xAA, 0xAA, 0xAA, 0xAA, 0xAA,
        ];
        assert_eq!(decoded, expected);
    }
}
