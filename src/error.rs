use std::io;

use thiserror::Error;

#[derive(Error, Debug)]
pub enum FvpError {
    #[error("IO error: {0}")]
    Io(#[from] io::Error),
    #[error("Decoding error: failed to decode Shift-JIS string")]
    DecodingError,
    #[error("Encoding error: failed to encode string to Shift-JIS")]
    EncodingError,
    #[error("Archive format error: {0}")]
    FormatError(String),
    #[error("Entry count mismatch: expected {expected}, got {actual}")]
    CountMismatch { expected: usize, actual: usize },
}

pub type Result<T> = std::result::Result<T, FvpError>;
