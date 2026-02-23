pub mod error;
mod ifd;

pub type SaverResult<T> = exn::Result<T, error::SaverError>;
