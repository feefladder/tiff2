use std::io::Cursor;
use std::ops::Range;

use bytes::Bytes;
use exn::{bail, OptionExt, ResultExt};

use crate::loader::metadata::error::MetaError;
use crate::loader::metadata::MetaResult;
use crate::loader::EndianReader;
use crate::structs::{entry_size, num_entries_size, offset_size, Ifd, IfdEntry, Tag, TagData};
use crate::ByteOrder;

#[derive(Debug, Clone, PartialEq)]
pub struct IfdLoader {
    bigtiff: bool,
    byte_order: ByteOrder,
    ifd: Ifd,
}

impl IfdLoader {
    pub fn count(&self) -> usize {
        self.ifd.count()
    }

    /// given a buffer holding the count value, get the number of entries
    fn ifd_entry_count(
        buf: &[u8],
        offset: u64,
        bigtiff: bool,
        byte_order: ByteOrder,
    ) -> MetaResult<u64> {
        let mut r = EndianReader::wrap(Cursor::new(buf), byte_order);
        let count = if bigtiff {
            r.read_u64().or_raise(|| {
                MetaError::invalid_buffer(
                    offset..offset + 8,
                    format!("could not read number of entries in ifd at {offset}"),
                )
            })?
        } else {
            u64::from(r.read_u16().or_raise(|| {
                MetaError::invalid_buffer(
                    offset..offset + 2,
                    format!("could not read number of entries in ifd at {offset}"),
                )
            })?)
        };
        Ok(count)
    }

    /// Given a buffer holding the IFD, get the underlying IFD
    ///
    /// This also reads the count of the ifd (first value). The exact required
    /// size of this buffer cannot be known beforehand, but a (very) safe assumption is
    /// ~1KiB. If it fails, it will give the exact required range.
    ///
    ///
    pub fn load_ifd(
        mut ifd_buf: &[u8],
        offset: u64,
        bigtiff: bool,
        byte_order: ByteOrder,
    ) -> MetaResult<(Self, u64)> {
        let count = Self::ifd_entry_count(ifd_buf, offset, bigtiff, byte_order)?;

        // act as if we have a cursor: move the start of the buffer
        ifd_buf = &ifd_buf[usize::try_from(num_entries_size(bigtiff)).unwrap()..];

        // check if the entire ifd is in memory
        if u64::try_from(ifd_buf.len()).unwrap()
            < count * entry_size(bigtiff) + offset_size(bigtiff)
        {
            bail!(MetaError::invalid_buffer(
                offset
                    ..offset
                        + count * entry_size(bigtiff)
                        + offset_size(bigtiff)
                        + num_entries_size(bigtiff),
                format!("could not load IFD at offset {offset}")
            ))
        }
        let (ifd, next_offset) = Ifd::from_buffer(ifd_buf, count, byte_order, bigtiff)
            .expect("all reads should be in-range");
        Ok((
            Self {
                bigtiff,
                byte_order,
                ifd,
            },
            next_offset,
        ))
    }

    /// get the in-file ranges of this ifd's deferred tags
    pub fn value_ranges<'a>(&'a self) -> impl Iterator<Item = (Tag, Range<u64>)> + use<'a> {
        self.ifd.data.iter().filter_map(|(t, v)| match v {
            IfdEntry::Offset(o) => Some((*t, o.range())),
            IfdEntry::Value(_) => None,
        })
    }

    pub fn deferred_values_mut(&mut self) -> impl Iterator<Item = (&Tag, &mut IfdEntry)> {
        self.ifd
            .iter_mut()
            .filter(|(_, v)| matches!(v, IfdEntry::Offset(_)))
    }

    /// Load deferred values from value ranges
    ///
    /// ```
    /// let ifd_buf = [
    ///     1,0, // number of entries
    ///     1,1, // tag
    /// ];
    /// ```
    pub fn load_ifd_values(
        &mut self,
        ifd_offset: u64,
        bufs: &mut dyn Iterator<Item = (Tag, Bytes)>,
    ) -> MetaResult<()> {
        // ah, so this is currently in get_tags() function of reader, which is super ugly...
        for (tag, buf) in bufs {
            let entry = self.ifd.data.get_mut(&tag).ok_or_raise(|| {
                MetaError::permanent(format!(
                    "no pre-existing entry (offset) for {tag:?} in ifd at {ifd_offset}"
                ))
            })?;
            *entry = IfdEntry::Value(match entry {
                IfdEntry::Offset(o) => {
                    TagData::from_buffer(&buf[..], o.tag_type, o.count as usize, self.byte_order)
                        .or_raise(|| {
                            MetaError::invalid_buffer(
                                o.offset
                                    ..o.offset
                                        + o.count * u64::try_from(o.tag_type.size()).unwrap(),
                                format!("could not load {tag:?} into ifd"),
                            )
                        })?
                }
                IfdEntry::Value(data) => {
                    TagData::from_buffer(&buf[..], data.tag_type(), data.len(), self.byte_order)
                        .or_raise(|| {
                            // at this point, we don't know the in-file offset. Also re-loading tag data is somewhat weird...
                            MetaError::permanent(format!(
                                "failed to load tag data for {tag:?} into ifd, which already had {data:?}"
                            ))
                        })?
                }
            });
        }
        Ok(())
    }

    pub fn finish(self) -> Ifd {
        self.ifd
    }
}
