//! Crash-safe filesystem persistence for finalized Proof-of-Reputation state.
//!
//! `cordial-por` owns the versioned snapshot bytes and their validation. This
//! adapter owns only the node data-directory layout and durable replacement.
//! A write is flushed to a temporary file, atomically renamed over the current
//! snapshot, and followed by a directory sync. Restore never consumes the
//! temporary file, so an interrupted write leaves the last committed snapshot
//! readable.

use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::Mutex,
};

use cordial_por::{
    MAX_REPUTATION_STATE_SNAPSHOT_LEN, PorError, ReputationState, decode_reputation_state_snapshot,
    encode_reputation_state_snapshot,
};
use thiserror::Error;

/// Directory below the node data directory containing PoR state.
pub const POR_STATE_DIRECTORY: &str = "por";

/// Atomic snapshot target inside [`POR_STATE_DIRECTORY`].
pub const POR_STATE_FILE_NAME: &str = "reputation-state.bin";

const POR_STATE_TEMP_FILE_NAME: &str = ".reputation-state.bin.tmp";

/// Errors opening, persisting, or restoring the durable PoR snapshot.
#[derive(Debug, Error)]
pub enum PorStateStoreError {
    #[error("PoR state storage I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("invalid PoR state snapshot: {0}")]
    InvalidSnapshot(#[from] PorError),

    #[error("PoR state writer lock is poisoned")]
    WriterLockPoisoned,
}

/// Filesystem-backed store for one shard's latest finalized PoR state.
///
/// Construct one store per shard-specific data directory. Calls to `persist`
/// on the same instance are serialized; readers see either the previous file
/// or the fully synced replacement.
#[derive(Debug)]
pub struct PorStateStore {
    directory: PathBuf,
    snapshot_path: PathBuf,
    temporary_path: PathBuf,
    writer: Mutex<()>,
}

impl PorStateStore {
    /// Open the PoR store below `data_dir`, creating its directory if needed.
    pub fn open(data_dir: &Path) -> Result<Self, PorStateStoreError> {
        let directory = data_dir.join(POR_STATE_DIRECTORY);
        fs::create_dir_all(&directory)?;
        File::open(data_dir)?.sync_all()?;
        Ok(Self {
            snapshot_path: directory.join(POR_STATE_FILE_NAME),
            temporary_path: directory.join(POR_STATE_TEMP_FILE_NAME),
            directory,
            writer: Mutex::new(()),
        })
    }

    /// Return the committed snapshot path for diagnostics and backup tooling.
    pub fn snapshot_path(&self) -> &Path {
        &self.snapshot_path
    }

    /// Atomically persist the complete finalized PoR state.
    ///
    /// Encoding and invariant validation happen before the writer lock or any
    /// filesystem mutation. A failure therefore leaves the current snapshot
    /// untouched.
    pub fn persist(&self, state: &ReputationState) -> Result<(), PorStateStoreError> {
        let encoded = encode_reputation_state_snapshot(state)?;
        let _writer = self
            .writer
            .lock()
            .map_err(|_| PorStateStoreError::WriterLockPoisoned)?;

        let mut options = OpenOptions::new();
        options.create(true).truncate(true).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }

        let mut temporary = options.open(&self.temporary_path)?;
        temporary.write_all(&encoded)?;
        temporary.sync_all()?;
        drop(temporary);

        fs::rename(&self.temporary_path, &self.snapshot_path)?;
        File::open(&self.directory)?.sync_all()?;
        Ok(())
    }

    /// Restore the last committed snapshot, or `None` on a fresh data dir.
    ///
    /// Corrupt, truncated, oversized, or unsupported files are returned as
    /// errors. They are never treated as an empty first boot.
    pub fn restore(&self) -> Result<Option<ReputationState>, PorStateStoreError> {
        let file = match File::open(&self.snapshot_path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.into()),
        };

        let read_limit = u64::try_from(MAX_REPUTATION_STATE_SNAPSHOT_LEN)
            .expect("snapshot limit fits in u64")
            + 1;
        let mut encoded = Vec::new();
        file.take(read_limit).read_to_end(&mut encoded)?;
        if encoded.len() > MAX_REPUTATION_STATE_SNAPSHOT_LEN {
            return Err(PorError::ReputationStateSnapshotTooLarge.into());
        }

        decode_reputation_state_snapshot(&encoded)
            .map(Some)
            .map_err(Into::into)
    }
}
