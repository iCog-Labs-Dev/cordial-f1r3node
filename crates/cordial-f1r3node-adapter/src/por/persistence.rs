//! Crash-safe filesystem persistence for finalized Proof-of-Reputation state.
//!
//! `cordial-por` owns the versioned snapshot bytes and their validation. This
//! adapter owns the node data-directory layout, durable replacement, and
//! append-only reputation-block history. A snapshot write is flushed to a
//! temporary file, atomically renamed over the current snapshot, and followed
//! by a directory sync. A history append creates a new immutable round file.
//! Startup validates both stores and reconciles the one supported crash window
//! in which the snapshot committed immediately before its history entry.

use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::Mutex,
};

use cordial_por::{
    MAX_REPUTATION_STATE_SNAPSHOT_LEN, PorConfig, PorError, ReputationState,
    decode_reputation_state_snapshot, encode_reputation_state_snapshot,
};
use thiserror::Error;

use super::{
    checkpoint::AttestedPorCheckpoint,
    history::{PorReputationBlockHistory, PorReputationBlockHistoryError},
    lifecycle::CompletedPorRatingRound,
    transition::{
        AppliedPorReputationRound, stage_attested_checkpoint, stage_completed_reputation_round,
    },
};

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

/// Failures while restoring or durably advancing live PoR state.
#[derive(Debug, Error)]
pub enum DurablePorStateError {
    #[error("PoR state persistence failed: {0}")]
    Persistence(#[source] PorStateStoreError),

    #[error("PoR reputation-block history failed: {0}")]
    History(#[source] PorReputationBlockHistoryError),

    #[error("PoR reputation transition failed: {0}")]
    Transition(#[source] PorError),

    #[error("durable PoR state requires startup recovery after a storage failure")]
    RecoveryRequired,
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

/// Startup and commit boundary for one shard's live reputation state.
///
/// A fresh data directory is initialized from the caller-supplied state and
/// immediately persisted. An existing snapshot always takes precedence over
/// that fallback. Startup validates the retained block chain and reconciles an
/// empty or one-block-behind history from the snapshot's latest audited block.
/// Completed rounds are fully staged and audited, then written through
/// [`PorStateStore`] and [`PorReputationBlockHistory`] before the in-memory
/// state is replaced.
///
/// A storage error makes the owner unavailable until it is reopened. This
/// fail-closed rule covers errors whose on-disk commit outcome may be ambiguous
/// and the supported snapshot-before-history crash window.
#[derive(Debug)]
pub struct DurablePorState {
    store: PorStateStore,
    history: PorReputationBlockHistory,
    state: ReputationState,
    recovery_required: bool,
}

impl DurablePorState {
    /// Restore committed state or durably initialize a fresh data directory.
    pub fn open(
        data_dir: &Path,
        initial_state: ReputationState,
    ) -> Result<Self, DurablePorStateError> {
        let store = PorStateStore::open(data_dir).map_err(DurablePorStateError::Persistence)?;
        let state = match store.restore().map_err(DurablePorStateError::Persistence)? {
            Some(restored) => restored,
            None => {
                store
                    .persist(&initial_state)
                    .map_err(DurablePorStateError::Persistence)?;
                initial_state
            }
        };
        let history =
            PorReputationBlockHistory::open(data_dir).map_err(DurablePorStateError::History)?;
        history
            .reconcile_state_tip(state.latest_block())
            .map_err(DurablePorStateError::History)?;

        Ok(Self {
            store,
            history,
            state,
            recovery_required: false,
        })
    }

    /// Return the live state while the durable owner is healthy.
    pub fn state(&self) -> Result<&ReputationState, DurablePorStateError> {
        self.ensure_healthy()?;
        Ok(&self.state)
    }

    /// Return the committed snapshot path for diagnostics and backup tooling.
    pub fn snapshot_path(&self) -> &Path {
        self.store.snapshot_path()
    }

    /// Return the immutable block-history directory for diagnostics and backup tooling.
    pub fn history_directory_path(&self) -> &Path {
        self.history.directory_path()
    }

    /// Return the validated append-only history owned by this runtime.
    pub fn history(&self) -> &PorReputationBlockHistory {
        &self.history
    }

    /// Stage, durably commit, and publish one completed reputation round.
    ///
    /// Transition failures happen before filesystem I/O and leave this owner
    /// usable. The staged snapshot is committed first, followed by its immutable
    /// block-history entry. Storage failures leave the in-memory state
    /// unpublished and fail-close the owner, requiring startup recovery before
    /// more state is read or applied.
    pub fn apply_completed_round(
        &mut self,
        completed: &CompletedPorRatingRound,
        config: &PorConfig,
        shard_id: &[u8],
    ) -> Result<AppliedPorReputationRound, DurablePorStateError> {
        self.ensure_healthy()?;
        let (staged, applied) =
            stage_completed_reputation_round(completed, &self.state, config, shard_id)
                .map_err(DurablePorStateError::Transition)?;

        self.commit_staged(staged, applied)
    }

    /// Re-audit, durably commit, and publish an attested peer checkpoint.
    ///
    /// Replaying at this boundary prevents an attestation collected against
    /// stale state or different ratings, configuration, or shard context from
    /// reaching disk. Storage uses the same snapshot-first fail-closed sequence
    /// as locally constructed rounds.
    pub fn apply_attested_checkpoint(
        &mut self,
        attested: &AttestedPorCheckpoint,
        completed: &CompletedPorRatingRound,
        config: &PorConfig,
        shard_id: &[u8],
    ) -> Result<AppliedPorReputationRound, DurablePorStateError> {
        self.ensure_healthy()?;
        let (staged, applied) =
            stage_attested_checkpoint(completed, &self.state, config, shard_id, attested.block())
                .map_err(DurablePorStateError::Transition)?;

        self.commit_staged(staged, applied)
    }

    fn commit_staged(
        &mut self,
        staged: ReputationState,
        applied: AppliedPorReputationRound,
    ) -> Result<AppliedPorReputationRound, DurablePorStateError> {
        if let Err(error) = self.store.persist(&staged) {
            self.recovery_required = true;
            return Err(DurablePorStateError::Persistence(error));
        }
        if let Err(error) = self.history.append(&applied.block) {
            self.recovery_required = true;
            return Err(DurablePorStateError::History(error));
        }

        self.state = staged;
        Ok(applied)
    }

    fn ensure_healthy(&self) -> Result<(), DurablePorStateError> {
        if self.recovery_required {
            Err(DurablePorStateError::RecoveryRequired)
        } else {
            Ok(())
        }
    }
}
