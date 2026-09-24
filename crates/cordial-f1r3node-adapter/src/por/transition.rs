//! Atomic handoff from a completed adapter rating round into PoR state.
//!
//! This module connects lifecycle-owned rating collection to the deterministic
//! calculation and audit APIs owned by `cordial-por`. It does not define the
//! publication format or hashing scheme for reputation blocks.

use std::collections::HashMap;

use cordial_miners_core::NodeId;
use cordial_por::{
    PorConfig, PorError, ReputationBlock, ReputationBlockHeader, ReputationState, ReputationVector,
    ReputationWeight, build_reputation_block, replay_reputation_transition, reputation_weights,
};

use super::lifecycle::{CompletedPorRatingRound, PorRatingRoundCloseReason};

/// Publication-layer commitments required to construct a reputation block.
///
/// The adapter deliberately accepts these bytes rather than choosing a
/// consensus hash encoding. `cordial-por` validates the fields required by its
/// current block contract, including non-empty rating and reputation hashes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PorReputationBlockCommitments {
    pub previous_reputation_hash: Option<Vec<u8>>,
    pub ratings_hash: Vec<u8>,
    pub reputation_root: Vec<u8>,
}

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
    commitments: PorReputationBlockCommitments,
) -> Result<AppliedPorReputationRound, PorError> {
    let previous = ReputationVector {
        round: state.round(),
        values: state.reputation_list().entries.clone(),
    };
    let batch = completed.batch();
    let reputation_list =
        replay_reputation_transition(&previous, &batch.ratings, batch.round, config)?;
    let block = build_reputation_block(
        ReputationBlockHeader {
            round: batch.round,
            previous_reputation_hash: commitments.previous_reputation_hash,
            ratings_hash: commitments.ratings_hash,
            reputation_root: commitments.reputation_root,
        },
        reputation_list,
    )?;

    let mut staged = state.clone();
    staged.apply_reputation_block(&batch.ratings, block.clone(), config)?;
    let weights = reputation_weights(&staged);

    *state = staged;
    Ok(AppliedPorReputationRound {
        close_reason: completed.close_reason(),
        block,
        weights,
    })
}
