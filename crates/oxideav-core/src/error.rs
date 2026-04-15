//! Shared error type for oxideav.

use thiserror::Error;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Error)]
pub enum Error {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("unsupported: {0}")]
    Unsupported(String),

    #[error("invalid data: {0}")]
    InvalidData(String),

    #[error("end of stream")]
    Eof,

    #[error("need more data")]
    NeedMore,

    #[error("format not found: {0}")]
    FormatNotFound(String),

    #[error("codec not found: {0}")]
    CodecNotFound(String),

    #[error("{0}")]
    Other(String),
}

impl Error {
    pub fn unsupported(msg: impl Into<String>) -> Self {
        Self::Unsupported(msg.into())
    }

    pub fn invalid(msg: impl Into<String>) -> Self {
        Self::InvalidData(msg.into())
    }

    pub fn other(msg: impl Into<String>) -> Self {
        Self::Other(msg.into())
    }
}
