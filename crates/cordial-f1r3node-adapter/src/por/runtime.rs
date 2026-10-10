//! Production lifecycle owner for PoR state and Cordial weight activation.
//!
//! Construction restores the durable reputation state and the previously
//! activated projection, recomputes the Cordial finalized view under those
//! weights, and activates any newer committed projection before ingress can be
//! exposed. Completed local or attested rounds always commit state first and
//! immediately attempt weight activation.

use std::path::Path;

use cordial_por::{MAX_REPUTATION_BLOCK_SHARD_ID_LEN, PorConfig, PorError, ReputationState};
use thiserror::Error;

use crate::{
    live_ingress::LiveIngress,
    ordered_output::OrderedFinalizedOutput,
    snapshot::{CORDIAL_WAVELENGTH, SnapshotError},
};

use super::{
    activation::PorWeightActivationOutcome,
    checkpoint::AttestedPorCheckpoint,
    lifecycle::CompletedPorRatingRound,
    persistence::{DurablePorState, DurablePorStateError},
    transition::AppliedPorReputationRound,
};

/// Startup or finalized-output failures at the combined PoR runtime boundary.
#[derive(Debug, Error)]
pub enum PorRuntimeError {
    #[error("invalid PoR configuration: {0}")]
    InvalidConfig(#[source] PorError),

    #[error("PoR runtime wavelength {actual} does not match Cordial wavelength {expected}")]
    UnsupportedWavelength { expected: u64, actual: u64 },

    #[error("invalid PoR runtime shard: {0}")]
    InvalidShard(#[source] PorError),

    #[error("cannot publish the Cordial finalized view: {0:?}")]
    FinalizedOutput(SnapshotError),

    #[error("cannot restore or activate durable PoR state: {0}")]
    Durable(#[from] DurablePorStateError),
}

/// A reputation round that is durably committed, together with its activation result.
///
/// `activation` is intentionally separate from the outer result. Once this
/// value exists, the reputation round committed successfully even when Cordial
/// temporarily rejected activation. Call [`PorRuntime::retry_weight_activation`]
/// after the missing finality or membership condition is resolved.
#[derive(Debug)]
pub struct CommittedPorRound {
    pub applied: AppliedPorReputationRound,
    pub activation: Result<PorWeightActivationOutcome, DurablePorStateError>,
}

impl CommittedPorRound {
    pub fn activation_pending(&self) -> bool {
        self.activation.is_err()
    }
}

/// Owns live Cordial ingress and durable PoR state as one lifecycle.
///
/// The wrapper is returned only after startup has recomputed the current
/// finalized output and restored committed weights. Host code should construct
/// this owner before sharing its ingress with networking tasks.
pub struct PorRuntime<A> {
    ingress: LiveIngress<A>,
    state: DurablePorState,
    config: PorConfig,
    shard_id: Vec<u8>,
    wavelength: u64,
    startup_activation: PorWeightActivationOutcome,
}

impl<A> PorRuntime<A> {
    /// Restore one shard's PoR state and activate it before exposing ingress.
    pub fn open(
        data_dir: &Path,
        initial_state: ReputationState,
        mut ingress: LiveIngress<A>,
        config: PorConfig,
        shard_id: impl Into<Vec<u8>>,
        wavelength: u64,
    ) -> Result<Self, PorRuntimeError> {
        if wavelength != CORDIAL_WAVELENGTH {
            return Err(PorRuntimeError::UnsupportedWavelength {
                expected: CORDIAL_WAVELENGTH,
                actual: wavelength,
            });
        }
        config.validate().map_err(PorRuntimeError::InvalidConfig)?;
        let shard_id = shard_id.into();
        validate_shard_id(&shard_id)?;

        let mut state = DurablePorState::open(data_dir, initial_state)?;
        let restored_activation = state.restore_activated_weights(&mut ingress)?;
        ingress
            .latest_finalized_ordered_output(wavelength)
            .map_err(PorRuntimeError::FinalizedOutput)?;
        let startup_activation = match (
            state.has_unactivated_committed_round()?,
            restored_activation,
        ) {
            (false, Some(restored)) => restored,
            _ => state.activate_weights(&mut ingress)?,
        };

        Ok(Self {
            ingress,
            state,
            config,
            shard_id,
            wavelength,
            startup_activation,
        })
    }

    /// How startup synchronized durable PoR weights into this ingress.
    pub fn startup_activation(&self) -> PorWeightActivationOutcome {
        self.startup_activation
    }

    pub fn ingress(&self) -> &LiveIngress<A> {
        &self.ingress
    }

    /// Access live ingress for normal block ingestion and Cordial membership updates.
    ///
    /// Callers cannot access this until [`Self::open`] has restored weights.
    pub fn ingress_mut(&mut self) -> &mut LiveIngress<A> {
        &mut self.ingress
    }

    pub fn state(&self) -> &DurablePorState {
        &self.state
    }

    pub fn config(&self) -> &PorConfig {
        &self.config
    }

    pub fn shard_id(&self) -> &[u8] {
        &self.shard_id
    }

    pub fn wavelength(&self) -> u64 {
        self.wavelength
    }

    /// Recompute and publish Cordial's latest finalized view.
    pub fn publish_finalized_output(&mut self) -> Result<OrderedFinalizedOutput, PorRuntimeError> {
        self.ingress
            .latest_finalized_ordered_output(self.wavelength)
            .map_err(PorRuntimeError::FinalizedOutput)
    }

    /// Commit a completed local rating round and immediately activate its weights.
    ///
    /// # Safety
    ///
    /// This method bypasses the checkpoint-attestation requirement and activates
    /// weights based solely on the local validator's view of which rating batches
    /// arrived. Two honest validators can produce different `ReputationBlock`
    /// hashes and weights if they closed on different (but both quorum-satisfying)
    /// subsets of complete batches.
    ///
    /// **Production code must use [`Self::commit_attested_checkpoint`] instead.**
    /// This method is preserved for emergency single-validator scenarios and tests.
    ///
    /// An outer error means the durable state transition did not publish in
    /// memory (and may require startup recovery after an ambiguous storage
    /// failure). A successful return always contains the activation attempt;
    /// an activation error leaves the committed round available for retry.
    #[deprecated(
        since = "0.0.0",
        note = "bypasses checkpoint attestation — use commit_attested_checkpoint for production"
    )]
    pub fn commit_completed_round(
        &mut self,
        completed: &CompletedPorRatingRound,
    ) -> Result<CommittedPorRound, DurablePorStateError> {
        let applied = self
            .state
            .apply_completed_round(completed, &self.config, &self.shard_id)?;
        let activation = self.state.activate_weights(&mut self.ingress);
        Ok(CommittedPorRound {
            applied,
            activation,
        })
    }

    /// Commit an attested checkpoint and immediately activate its weights.
    ///
    /// Prefer [`Self::commit_with_attested_checkpoint`] for the semantically explicit production name.
    pub fn commit_attested_checkpoint(
        &mut self,
        attested: &AttestedPorCheckpoint,
        completed: &CompletedPorRatingRound,
    ) -> Result<CommittedPorRound, DurablePorStateError> {
        let applied = self.state.apply_attested_checkpoint(
            attested,
            completed,
            &self.config,
            &self.shard_id,
        )?;
        let activation = self.state.activate_weights(&mut self.ingress);
        Ok(CommittedPorRound {
            applied,
            activation,
        })
    }

    /// Commit a completed rating round after checkpoint attestation and immediately
    /// activate its weights.
    ///
    /// This is the **canonical production commit path**. The `attested` checkpoint
    /// guarantees that a 2/3 weighted supermajority of authorized validators signed
    /// the same `ReputationBlock` hash, preventing divergent weight activation
    /// across honest validators.
    ///
    /// An outer error means the durable state transition did not publish in
    /// memory. An inner `activation` error leaves the committed round available
    /// for retry via [`Self::retry_weight_activation`].
    pub fn commit_with_attested_checkpoint(
        &mut self,
        attested: &AttestedPorCheckpoint,
        completed: &CompletedPorRatingRound,
    ) -> Result<CommittedPorRound, DurablePorStateError> {
        self.commit_attested_checkpoint(attested, completed)
    }

    /// Retry the latest committed projection after a transient activation failure.
    pub fn retry_weight_activation(
        &mut self,
    ) -> Result<PorWeightActivationOutcome, DurablePorStateError> {
        self.state.activate_weights(&mut self.ingress)
    }

    /// Consume the owner after shutdown.
    pub fn into_parts(self) -> (LiveIngress<A>, DurablePorState) {
        (self.ingress, self.state)
    }
}

fn validate_shard_id(shard_id: &[u8]) -> Result<(), PorRuntimeError> {
    if shard_id.is_empty() {
        return Err(PorRuntimeError::InvalidShard(
            PorError::MissingReputationBlockShardId,
        ));
    }
    if shard_id.len() > MAX_REPUTATION_BLOCK_SHARD_ID_LEN {
        return Err(PorRuntimeError::InvalidShard(
            PorError::ReputationBlockShardIdTooLong,
        ));
    }
    Ok(())
}
