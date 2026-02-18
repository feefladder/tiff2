use std::sync::Arc;

use crate::decoder::CogReader;
use crate::structs::Image;

#[non_exhaustive]
pub struct ImageDecoder<R: CogReader> {
    pub image: Image,
    pub reader: Arc<R>,
}

impl<R: CogReader> ImageDecoder<R> {}
