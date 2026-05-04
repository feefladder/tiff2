mod entry;
pub use entry::{IfdEntry, Offset, TagData};
mod ifd;
pub(crate) use ifd::offset_tag_type;
pub use ifd::{entry_size, num_entries_size, offset_size, Ifd};
pub mod tags;
pub use tags::{Tag, TagType};
mod extension;
pub use extension::{TiffExtEq, TiffExtError, TiffExtension};
