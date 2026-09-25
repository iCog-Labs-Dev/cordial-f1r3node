//! Atomic handoff from a completed adapter rating round into PoR state.
//!
//! This module connects lifecycle-owned rating collection to the deterministic
//! calculation and audit APIs owned by `cordial-por`. It does not define the
//! peer publication transport for reputation blocks.

use std::collections::HashMap;

use cordial_miners_core::NodeId;
use cordial_por::{
    PorConfig, PorError, ReputationBlock, ReputationBlockContext, ReputationState,
    ReputationVector, ReputationWeight, build_reputation_block, replay_reputation_transition,
    reputation_weights,
};

use super::lifecycle::{CompletedPorRatingRound, PorRatingRoundCloseReason};

/// Result of a successfully applied reputation round.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppliedPorReputationRound {
    pub close_reason: PorRatingRoundCloseReason,
    pub block: ReputationBlock,
    pub weights: HashMap<NodeId, ReputationWeight>,
}

/// Replay, construct, audit, and atomically apply one completed PoR round.
///
/// All fallible work is performed against a cloned state. The caller's state
/// is replaced only after the completed rating batch has produced a valid
/// reputation block, audit replay has accepted it, and Cordial weights have
/// been exported. Any error leaves `state` unchanged.
pub fn apply_completed_reputation_round(
    completed: &CompletedPorRatingRound,
    state: &mut ReputationState,
    config: &PorConfig,
    shard_id: &[u8],
) -> Result<AppliedPorReputationRound, PorError> {
    let (staged, applied) = stage_completed_reputation_round(completed, state, config, shard_id)?;
    *state = staged;
    Ok(applied)
}

/// Build and audit the next state without changing the live state.
///
/// The persistence boundary uses this helper to durably commit `staged`
/// before making it visible as the live reputation state.
pub(super) fn stage_completed_reputation_round(
    completed: &CompletedPorRatingRound,
    state: &ReputationState,
    config: &PorConfig,
    shard_id: &[u8],
) -> Result<(ReputationState, AppliedPorReputationRound), PorError> {
    let previous = ReputationVector {
        round: state.round(),
        values: state.reputation_list().entries.clone(),
    };
    let batch = completed.batch();
    let reputation_list =
        replay_reputation_transition(&previous, &batch.ratings, batch.round, config)?;
    let block = build_reputation_block(
        ReputationBlockContext {
            shard_id,
            source_finalized_wave: completed.opened().finalized_wave,
            previous_block: state.latest_block(),
        },
        batch,
        reputation_list,
        config,
    )?;

    let mut staged = state.clone();
    staged.apply_reputation_block(
        shard_id,
        completed.opened().finalized_wave,
        &batch.ratings,
        block.clone(),
        config,
    )?;
    let weights = reputation_weights(&staged);

    Ok((
        staged,
        AppliedPorReputationRound {
            close_reason: completed.close_reason(),
            block,
            weights,
        },
    ))
}
