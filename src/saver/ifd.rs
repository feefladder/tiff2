use std::io::Cursor;

use exn::{bail, ResultExt};
use smallvec::smallvec;

use crate::loader::EndianReader;
use crate::saver::error::SaverError;
use crate::saver::SaverResult;
use crate::structs::{entry_size, num_entries_size, offset_size, Ifd, IfdEntry, Tag, TagData};
use crate::ByteOrder;

/// An Ifd saver
///
/// This is a struct that can be created from an [`Ifd`] to tell what values need to
/// be written at different places.
///
/// Once all those values have been changed to [`IfdEntry::Offset`], you can call write_to on this, consuming the [`IfdSaver`]
pub struct IfdSaver {
    offset: u64,
    bigtiff: bool,
    byte_order: ByteOrder,
    ifd: Ifd,
}

impl IfdSaver {
    pub fn count(&self) -> usize {
        self.ifd.count()
    }

    /// Returns an iterator of Entries that still need to be changed to Offset before this can be written
    fn to_do(&self) -> impl Iterator<Item = (&Tag, &TagData)> {
        self.ifd.iter().filter_map(|(k, v)| match v {
            IfdEntry::Value(tag_data) => {
                if u64::try_from(tag_data.as_ref().len()).unwrap() > offset_size(self.bigtiff) {
                    Some((k, tag_data))
                } else {
                    None
                }
            }
            IfdEntry::Offset(_) => None,
        })
    }

    /// Given an IFD and a buffer, write the IFD to the buffer
    ///
    /// TODO: how should it treat the IFD? also defer non-inlined values?
    pub fn from_ifd(offset: u64, ifd: Ifd, bigtiff: bool, byte_order: ByteOrder) -> Self {
        Self {
            offset,
            bigtiff,
            byte_order,
            ifd,
        }
    }

    pub fn write(&mut self, buf: &mut [u8], next_ifd_offset: u64) -> SaverResult<()> {
        // check if all entries can be written
        let mut to_dos = self.to_do().peekable();
        if to_dos.peek().is_some() {
            let missing_tags: Vec<_> = to_dos.map(|(k, _)| *k).collect();
            bail!(SaverError {
                status: crate::saver::error::SaverErrorStatus::EasyFix,
                message: format!(
                    "Cannot write ifd yet, {} tags still need to be externalized",
                    missing_tags.len()
                ),
                kind: crate::saver::error::SaverErrorKind::IncompleteIfd { missing_tags },
            })
        }
        // check if the buffer is large enough
        let count = u64::try_from(self.ifd.count()).unwrap();
        let required_len = num_entries_size(self.bigtiff)
            + count * entry_size(self.bigtiff)
            + offset_size(self.bigtiff);
        if u64::try_from(buf.len()).unwrap() < required_len {
            bail!(SaverError::invalid_buffer(
                required_len,
                format!(
                    "ifd buffer of length {:?} too small, need {required_len:?}",
                    buf.len()
                )
            ))
        }
        // all good: start writing
        let mut offset = if self.bigtiff {
            TagData::Long8(smallvec![count])
        } else {
            TagData::Short(smallvec![u16::try_from(count).unwrap()])
        }
        .to_buffer(buf, self.byte_order);
        for (tag, entry) in self.ifd.iter() {
            offset += TagData::Short(smallvec![tag.to_u16(), entry.tag_type().to_u16()])
                .to_buffer(&mut buf[offset..], self.byte_order);

            offset += if self.bigtiff {
                TagData::Long8(smallvec![entry.count()])
            } else {
                TagData::Long(smallvec![u32::try_from(entry.count()).unwrap()])
            }
            .to_buffer(&mut buf[offset..], self.byte_order);

            offset += match entry {
                IfdEntry::Value(v) => {
                    v.to_buffer(&mut buf[offset..], self.byte_order);
                    offset_size(self.bigtiff) as usize
                }
                IfdEntry::Offset(o) => if self.bigtiff {
                    TagData::Long8(smallvec![o.offset])
                } else {
                    TagData::Long(smallvec![u32::try_from(o.offset).unwrap()])
                }
                .to_buffer(&mut buf[offset..], self.byte_order),
            };
        }
        offset += if self.bigtiff {
            TagData::Long8(smallvec![next_ifd_offset])
        } else {
            TagData::Long(smallvec![u32::try_from(next_ifd_offset).or_raise(
                || SaverError::need_bigtiff(format!(
                    "next ifd offset {next_ifd_offset} does not fit in smalltiff"
                ))
            )?])
        }
        .to_buffer(&mut buf[offset..], self.byte_order);
        todo!()
    }
}
