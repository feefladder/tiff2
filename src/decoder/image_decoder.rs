use crate::{decoder::CogReader, structs::Image};

use std::sync::Arc;

#[non_exhaustive]
pub struct ImageDecoder<R: CogReader> {
    pub image: Image,
    pub reader: Arc<R>,
}

impl<R: CogReader> ImageDecoder<R> {}
