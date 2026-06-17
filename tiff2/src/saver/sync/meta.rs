use std::sync::Arc;

use crate::saver::metadata::{TiffExtSaverRegistry, TiffSaver};
use crate::saver::{MetaWriteResult, SyncMetaWriter, SyncPush, TiffMetaWriter, TiffWriter};
use crate::structs::TileOpts;

impl<Push: SyncPush, Saver: TiffSaver> SyncMetaWriter<Push, Saver> for TiffMetaWriter<Push, Saver> {
    fn open(
        push: Push,
        saver: Saver,
        extension_registry: Arc<TiffExtSaverRegistry>,
    ) -> MetaWriteResult<Self> {
        Ok(Self {
            push,
            saver,
            extension_registry,
        })
    }

    fn next(&mut self) -> MetaWriteResult<Option<u64>> {
        todo!()
    }

    fn finish(self) -> MetaWriteResult<TiffWriter<Push, Saver>> {
        todo!()
    }
}
