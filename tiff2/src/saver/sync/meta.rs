use crate::saver::{metadata::TiffSaver, SyncMetaWriter, SyncPush, TiffMetaWriter};

impl<Push: SyncPush, Saver: TiffSaver> SyncMetaWriter<Push> for TiffMetaWriter<Push, Saver> {
    fn open(
        push: Push,
        saver: Saver,
        extension_registry: std::sync::Arc<crate::saver::metadata::TiffExtSaverRegistry>,
    ) -> crate::saver::MetaWriteResult<Self> {
        Ok(Self {
            push,
            saver,
            extension_registry,
        })
    }

    fn next(&mut self) -> crate::saver::MetaWriteResult<Option<u64>> {}
}
