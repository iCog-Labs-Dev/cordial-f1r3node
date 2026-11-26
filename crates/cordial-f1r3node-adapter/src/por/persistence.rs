//! Crash-safe filesystem persistence for finalized Proof-of-Reputation state.
//!
//! `cordial-por` owns the versioned snapshot bytes and their validation. This
//! adapter owns the node data-directory layout, durable replacement, and
//! append-only reputation-block history, and weight-activation lifecycle. A
//! snapshot write is flushed to a
//! temporary file, atomically renamed over the current snapshot, and followed
//! by a directory sync. A history append creates a new immutable round file.
//! Startup validates both stores and reconciles the one supported crash window
//! in which the snapshot committed immediately before its history entry. The
//! separately committed activation marker retains the actual active weight map,
//! allowing a newer state snapshot to be safely retried against a fresh
//! Cordial ingress after failure or restart.

use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::Mutex,
};

use cordial_por::{
    MAX_REPUTATION_STATE_SNAPSHOT_LEN, PorConfig, PorError, RatingSourceCheckpoint,
    ReputationState, authorized_validator_weights, decode_reputation_state_snapshot,
    encode_reputation_state_snapshot,
};
use thiserror::Error;

use crate::live_ingress::{LiveIngress, PorWeightActivationError};

use super::{
    activation::{
        PorWeightActivationOutcome, PorWeightActivationRecord, PorWeightActivationStore,
        PorWeightActivationStoreError,
    },
    checkpoint::AttestedPorCheckpoint,
    history::{PorReputationBlockHistory, PorReputationBlockHistoryError},
    lifecycle::CompletedPorRatingRound,
    lifecycle::checkpoint::CompletedCheckpointRatingRound,
    transition::{
        AppliedPorReputationRound, stage_attested_checkpoint, stage_completed_checkpoint_round,
        stage_completed_reputation_round,
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

    #[error("PoR weight-activation persistence failed: {0}")]
    ActivationPersistence(#[source] PorWeightActivationStoreError),

    #[error("PoR weight activation failed: {0}")]
    Activation(#[source] PorWeightActivationError),

    #[error("activated PoR round {activated_round} is ahead of committed round {committed_round}")]
    ActivationAheadOfState {
        activated_round: u64,
        committed_round: u64,
    },

    #[error("activated PoR round {0} does not match the committed reputation checkpoint")]
    ActivationCheckpointMismatch(u64),
    #[error("activated PoR round {0} has weights that do not match its committed state")]
    ActivationProjectionMismatch(u64),

    #[error("committed PoR round {committed_round} has no durable active-weight projection")]
    MissingActivatedWeights { committed_round: u64 },

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
    activation_store: PorWeightActivationStore,
    state: ReputationState,
    activated_weights: Option<PorWeightActivationRecord>,
    process_activation: Option<PorWeightActivationRecord>,
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
        let activation_store = PorWeightActivationStore::open(data_dir)
            .map_err(DurablePorStateError::ActivationPersistence)?;
        let activated_weights = activation_store
            .restore()
            .map_err(DurablePorStateError::ActivationPersistence)?;
        validate_activation_record(activated_weights.as_ref(), &state)?;

        Ok(Self {
            store,
            history,
            state,
            activation_store,
            recovery_required: false,
            activated_weights,
            process_activation: None,
        })
    }

    /// Return the live state while the durable owner is healthy.
    pub fn state(&self) -> Result<&ReputationState, DurablePorStateError> {
        self.ensure_healthy()?;
        Ok(&self.state)
    }

    /// Source cursor committed in the latest v3 reputation block, if any.
    pub fn committed_checkpoint_source(
        &self,
    ) -> Result<Option<RatingSourceCheckpoint>, DurablePorStateError> {
        self.ensure_healthy()?;
        Ok(self
            .state
            .latest_block()
            .and_then(|block| block.header.source_checkpoint))
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

    /// Return the activation-record path for diagnostics and backup tooling.
    pub fn weight_activation_record_path(&self) -> &Path {
        self.activation_store.record_path()
    }

    /// Return the last durable PoR projection accepted by Cordial.
    pub fn activated_weight_record(
        &self,
    ) -> Result<Option<&PorWeightActivationRecord>, DurablePorStateError> {
        self.ensure_healthy()?;
        Ok(self.activated_weights.as_ref())
    }

    /// Restore the exact projection proven by the durable activation record.
    ///
    /// This must run before startup recomputes finality. If state committed
    /// after the last activation, the older recorded projection remains the
    /// only valid basis for deciding whether the pending weights are safe.
    pub fn restore_activated_weights<A>(
        &mut self,
        ingress: &mut LiveIngress<A>,
    ) -> Result<Option<PorWeightActivationOutcome>, DurablePorStateError> {
        self.ensure_healthy()?;

        let Some(activated) = self.activated_weights.clone() else {
            if self.state.round() != 0 {
                return Err(DurablePorStateError::MissingActivatedWeights {
                    committed_round: self.state.round(),
                });
            }
            return Ok(None);
        };

        ingress
            .restore_activated_por_weights(activated.weights())
            .map_err(DurablePorStateError::Activation)?;
        self.process_activation = Some(activated);
        Ok(Some(PorWeightActivationOutcome::Restored))
    }

    /// Whether the committed reputation round is newer than the activation marker.
    ///
    /// Startup must first call [`Self::restore_activated_weights`] because
    /// Cordial's in-memory bonds need restoration in every process. This query
    /// then identifies the state-commit/activation crash window.
    pub fn has_unactivated_committed_round(&self) -> Result<bool, DurablePorStateError> {
        self.ensure_healthy()?;
        match &self.activated_weights {
            Some(record) if record.reputation_round() == self.state.round() => record
                .matches_state_checkpoint(&self.state)
                .map(|matches| !matches)
                .map_err(|error| {
                    DurablePorStateError::Activation(PorWeightActivationError::from(error))
                }),
            _ => Ok(true),
        }
    }

    /// Project and activate the latest committed PoR state in Cordial.
    ///
    /// The state snapshot always commits before this method is called. Live
    /// weights are replaced only after all Cordial safety checks pass. The
    /// activation record is then atomically persisted. An ordinary activation
    /// rejection is retryable. A marker-write failure fail-closes this owner,
    /// because the durable result may be ambiguous; restart restores the prior
    /// active projection before evaluating and recording the latest committed
    /// state.
    pub fn activate_weights<A>(
        &mut self,
        ingress: &mut LiveIngress<A>,
    ) -> Result<PorWeightActivationOutcome, DurablePorStateError> {
        self.ensure_healthy()?;

        let mut authorized_validators: Vec<_> = ingress.bonds().keys().cloned().collect();
        authorized_validators.sort();
        let projected = authorized_validator_weights(&self.state, &authorized_validators)
            .map_err(PorWeightActivationError::from)
            .map_err(DurablePorStateError::Activation)?;
        if self.state.round() != 0 && self.state.latest_block().is_none() {
            return Err(DurablePorStateError::Activation(
                PorWeightActivationError::MissingReputationCheckpoint {
                    round: self.state.round(),
                },
            ));
        }
        let desired = PorWeightActivationRecord::from_state_and_weights(&self.state, &projected)
            .map_err(PorWeightActivationError::from)
            .map_err(DurablePorStateError::Activation)?;

        validate_activation_record(self.activated_weights.as_ref(), &self.state)?;
        if self.process_activation.as_ref() == Some(&desired) && ingress.bonds() == &projected {
            return Ok(PorWeightActivationOutcome::AlreadyActive);
        }

        let already_durable = self.activated_weights.as_ref() == Some(&desired);
        ingress
            .apply_por_weights(&self.state)
            .map_err(DurablePorStateError::Activation)?;

        if !already_durable {
            if let Err(error) = self.activation_store.persist(&desired) {
                self.recovery_required = true;
                return Err(DurablePorStateError::ActivationPersistence(error));
            }
            self.activated_weights = Some(desired.clone());
        }
        self.process_activation = Some(desired);

        Ok(if already_durable {
            PorWeightActivationOutcome::Restored
        } else {
            PorWeightActivationOutcome::Activated
        })
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

    /// Audit and durably commit one complete checkpoint rating round.
    ///
    /// The snapshot's v3 reputation block is the source cursor. Replaying a
    /// completed source after restart fails the consecutive-round audit.
    pub fn apply_completed_checkpoint_round(
        &mut self,
        completed: &CompletedCheckpointRatingRound,
        config: &PorConfig,
        shard_id: &[u8],
    ) -> Result<AppliedPorReputationRound, DurablePorStateError> {
        self.ensure_healthy()?;
        let (staged, applied) =
            stage_completed_checkpoint_round(completed, &self.state, config, shard_id)
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

fn validate_activation_record(
    activated: Option<&PorWeightActivationRecord>,
    state: &ReputationState,
) -> Result<(), DurablePorStateError> {
    let Some(activated) = activated else {
        return Ok(());
    };
    if activated.reputation_round() > state.round() {
        return Err(DurablePorStateError::ActivationAheadOfState {
            activated_round: activated.reputation_round(),
            committed_round: state.round(),
        });
    }
    if activated.reputation_round() == state.round() {
        if !activated
            .matches_state_checkpoint(state)
            .map_err(PorWeightActivationError::from)
            .map_err(DurablePorStateError::Activation)?
        {
            return Err(DurablePorStateError::ActivationCheckpointMismatch(
                state.round(),
            ));
        }

        let mut validators: Vec<_> = activated.weights().keys().cloned().collect();
        validators.sort();
        let projected = authorized_validator_weights(state, &validators)
            .map_err(PorWeightActivationError::from)
            .map_err(DurablePorStateError::Activation)?;
        if &projected != activated.weights() {
            return Err(DurablePorStateError::ActivationProjectionMismatch(
                state.round(),
            ));
        }
    }
    Ok(())
}
