//! Atomic handoff from a completed adapter rating round into PoR state.
//!
//! This module connects lifecycle-owned rating collection to the deterministic
//! calculation and audit APIs owned by `cordial-por`. Authentication and
//! weighted peer-checkpoint attestation remain separate adapter boundaries.

use std::collections::HashMap;

use cordial_miners_core::NodeId;
use cordial_por::{
    CheckpointReputationBlockContext, PorConfig, PorError, ReputationBlock, ReputationBlockContext,
    ReputationState, ReputationVector, ReputationWeight,
    build_checkpoint_reputation_block_with_penalties, build_reputation_block_with_penalties,
    replay_reputation_transition_with_penalties, reputation_weights,
};

use super::lifecycle::{
    CompletedPorRatingRound, PorRatingRoundCloseReason, checkpoint::CompletedCheckpointRatingRound,
};

/// Result of a successfully applied reputation round.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppliedPorReputationRound {
    pub close_reason: PorRatingRoundCloseReason,
    pub block: ReputationBlock,
    pub reputation_weights: HashMap<NodeId, ReputationWeight>,
}

/// Replay, construct, audit, and atomically apply one completed PoR round.
///
/// All fallible work is performed against a cloned state. The caller's state
/// is replaced only after the completed rating batch has produced a valid
/// reputation block, audit replay has accepted it, and raw reputation
/// weights have been calculated. Any error leaves `state` unchanged.
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
    completed.validate_complete_authorizations(shard_id)?;
    let previous = ReputationVector {
        round: state.round(),
        values: state.reputation_list().entries.clone(),
    };
    let batch = completed.batch();
    let reputation_list = replay_reputation_transition_with_penalties(
        &previous,
        &batch.ratings,
        batch.round,
        config,
        completed.penalties(),
    )?;
    let block = build_reputation_block_with_penalties(
        ReputationBlockContext {
            shard_id,
            source_finalized_wave: completed.opened().finalized_wave,
            previous_block: state.latest_block(),
        },
        batch,
        reputation_list,
        config,
        completed.penalties(),
    )?;

    stage_reputation_block(completed, state, config, shard_id, block)
}

/// Replay and audit a complete checkpoint round without changing live state.
///
/// The resulting v3 reputation block stores the finalized-prefix source, so
/// the durable snapshot and history also retain the committed source cursor.
pub(super) fn stage_completed_checkpoint_round(
    completed: &CompletedCheckpointRatingRound,
    state: &ReputationState,
    config: &PorConfig,
    shard_id: &[u8],
) -> Result<(ReputationState, AppliedPorReputationRound), PorError> {
    completed.validate_preceding_state(state)?;
    completed.validate_complete_authorizations(shard_id)?;
    let source = completed.source();
    let previous = ReputationVector {
        round: state.round(),
        values: state.reputation_list().entries.clone(),
    };
    let batch = completed.batch();
    let reputation_list = replay_reputation_transition_with_penalties(
        &previous,
        &batch.ratings,
        batch.round,
        config,
        completed.penalties(),
    )?;
    let block = build_checkpoint_reputation_block_with_penalties(
        CheckpointReputationBlockContext {
            shard_id,
            source,
            previous_block: state.latest_block(),
        },
        batch,
        reputation_list,
        config,
        completed.penalties(),
    )?;
    let mut staged = state.clone();
    staged.apply_checkpoint_reputation_block_with_penalties(
        shard_id,
        source,
        &batch.ratings,
        block.clone(),
        config,
        completed.penalties(),
    )?;
    let weights = reputation_weights(&staged);
    Ok((
        staged,
        AppliedPorReputationRound {
            close_reason: PorRatingRoundCloseReason::AllAuthorizedRaters,
            block,
            reputation_weights: weights,
        },
    ))
}

/// Re-audit an attested peer block against current state without publishing it.
///
/// The durable boundary calls this after checkpoint attestation so an attestation
/// created against stale state, ratings, configuration, or shard context cannot
/// be committed.
pub(super) fn stage_attested_checkpoint(
    completed: &CompletedPorRatingRound,
    state: &ReputationState,
    config: &PorConfig,
    shard_id: &[u8],
    block: &ReputationBlock,
) -> Result<(ReputationState, AppliedPorReputationRound), PorError> {
    stage_reputation_block(completed, state, config, shard_id, block.clone())
}

fn stage_reputation_block(
    completed: &CompletedPorRatingRound,
    state: &ReputationState,
    config: &PorConfig,
    shard_id: &[u8],
    block: ReputationBlock,
) -> Result<(ReputationState, AppliedPorReputationRound), PorError> {
    completed.validate_complete_authorizations(shard_id)?;
    let batch = completed.batch();
    let mut staged = state.clone();
    staged.apply_reputation_block_with_penalties(
        shard_id,
        completed.opened().finalized_wave,
        &batch.ratings,
        block.clone(),
        config,
        completed.penalties(),
    )?;
    let weights = reputation_weights(&staged);

    Ok((
        staged,
        AppliedPorReputationRound {
            close_reason: completed.close_reason(),
            block,
            reputation_weights: weights,
        },
    ))
}
