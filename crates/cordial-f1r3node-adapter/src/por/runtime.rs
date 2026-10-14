//! Production lifecycle owner for PoR state and Cordial weight activation.
//!
//! Construction restores the durable reputation state and the previously
//! activated projection, recomputes the Cordial finalized view under those
//! weights. A newer committed projection may wait for the host to replay its
//! source finalized wave before activation. Completed local or attested rounds
//! commit state first and immediately attempt weight activation.

use std::path::Path;

use cordial_por::{MAX_REPUTATION_BLOCK_SHARD_ID_LEN, PorConfig, PorError, ReputationState};
use thiserror::Error;

use crate::{
    live_ingress::{LiveIngress, PorWeightActivationError},
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
/// Startup restores the last activated projection before computing finality.
/// If a newer committed round needs blocks that are not yet replayed, the host
/// must publish those blocks and call `retry_weight_activation` before using
/// the ingress to process another PoR round.
pub struct PorRuntime<A> {
    ingress: LiveIngress<A>,
    state: DurablePorState,
    config: PorConfig,
    shard_id: Vec<u8>,
    wavelength: u64,
    startup_activation: PorWeightActivationOutcome,
}

impl<A> PorRuntime<A> {
    /// Restore one shard's PoR state and its last activated weights.
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
            (true, Some(restored)) => match state.activate_weights(&mut ingress) {
                Ok(activated) => activated,
                Err(DurablePorStateError::Activation(
                    PorWeightActivationError::MissingFinalizedOutput { .. }
                    | PorWeightActivationError::FinalizedWaveBehind { .. },
                )) => restored,
                Err(error) => return Err(error.into()),
            },
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
    /// An outer error means the durable state transition did not publish in
    /// memory (and may require startup recovery after an ambiguous storage
    /// failure). A successful return always contains the activation attempt;
    /// an activation error leaves the committed round available for retry.
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
