//! Error type for all LMDB storage operations.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum RepoError {
    #[error("LMDB error: {0}")]
    Lmdb(#[from] heed::Error),

    #[error("Serialization error: {0}")]
    Bincode(#[from] bincode::Error),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Mutex poisoned: {0}")]
    Lock(String),

    #[error("Invalid native block identity mapping: {0}")]
    NativeMapping(String),

    #[error("Invalid native height cursor: {0}")]
    NativeCursor(String),

    #[error("Incompatible blocklace storage: {0}")]
    IncompatibleStorage(String),
}

impl<T> From<std::sync::PoisonError<T>> for RepoError {
    fn from(e: std::sync::PoisonError<T>) -> Self {
        RepoError::Lock(e.to_string())
    }
}
