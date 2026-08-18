//! Reputation transition audit replay.
//!
//! The paper keeps reputation values open to all members so that any node can
//! audit them. This module implements that audit: it replays the deterministic
//! calculation pipeline from recorded ratings and the previous reputation
//! vector, then checks the result against a proposed `ReputationBlock`.
//!
//! Replay is read-only. It does not mutate `ReputationState`, publish blocks,
//! or perform networking.

use crate::{
    block::validate_reputation_block,
    clamp::clamp_reputation_vector,
    config::PorConfig,
    error::PorError,
    liquid_rank::compute_liquid_rank_contribution,
    matrix::build_rating_matrix,
    normalization::normalize_rating_matrix,
    ratings::build_rating_batch,
    transition::blend_reputation_transition,
    types::{RatingRecord, ReputationBlock, ReputationList, ReputationRound, ReputationVector},
};

/// Replay the deterministic reputation transition for a single round.
///
/// Runs the whole calculation path — batch, matrix, normalization, Liquid-Rank
/// contribution, alpha blend, sigmoid clamp — and returns the expected
/// reputation list. Every rating record must belong to `round`, and `round`
/// must immediately follow `previous_reputation.round`. Input ratings may
/// arrive in any order because batching sorts them canonically.
pub fn replay_reputation_transition(
    previous_reputation: &ReputationVector,
    ratings: &[RatingRecord],
    round: ReputationRound,
    config: &PorConfig,
) -> Result<ReputationList, PorError> {
    let batch = build_rating_batch(round, ratings.to_vec(), config)?;
    let matrix = build_rating_matrix(&batch)?;
    let normalized = normalize_rating_matrix(&matrix, config)?;
    let contribution = compute_liquid_rank_contribution(&normalized, previous_reputation, config)?;
    let blended = blend_reputation_transition(&contribution, previous_reputation, config)?;
    let clamped = clamp_reputation_vector(&blended, config)?;

    Ok(ReputationList {
        round: clamped.round,
        entries: clamped.values,
    })
}

/// Verify that a proposed reputation block matches a deterministic replay.
///
/// The block is first put through `validate_reputation_block`, so an audited
/// block is held to exactly the rules a constructed one satisfies: matching
/// header and list rounds, non-empty hash fields, and a canonically ordered
/// reputation list. The replayed list is then compared entry for entry, so a
/// validator accepts the block only when the recorded ratings and the previous
/// reputation actually produce it.
pub fn verify_reputation_transition(
    previous_reputation: &ReputationVector,
    ratings: &[RatingRecord],
    proposed_block: &ReputationBlock,
    config: &PorConfig,
) -> Result<(), PorError> {
    validate_reputation_block(proposed_block)?;

    let proposed = &proposed_block.reputation_list;
    let expected =
        replay_reputation_transition(previous_reputation, ratings, proposed.round, config)?;

    compare_reputation_lists(&expected, proposed)
}

fn compare_reputation_lists(
    expected: &ReputationList,
    proposed: &ReputationList,
) -> Result<(), PorError> {
    let mut expected_entries = expected.entries.iter().peekable();
    let mut proposed_entries = proposed.entries.iter().peekable();

    loop {
        match (expected_entries.peek(), proposed_entries.peek()) {
            (Some(expected_entry), Some(proposed_entry)) => {
                match expected_entry.node_id.cmp(&proposed_entry.node_id) {
                    std::cmp::Ordering::Less => return Err(PorError::MissingReputationBlockEntry),
                    std::cmp::Ordering::Equal => {
                        if expected_entry.reputation != proposed_entry.reputation {
                            return Err(PorError::ReputationValueMismatch);
                        }

                        expected_entries.next();
                        proposed_entries.next();
                    }
                    std::cmp::Ordering::Greater => {
                        return Err(PorError::UnexpectedReputationBlockEntry);
                    }
                }
            }
            (Some(_), None) => return Err(PorError::MissingReputationBlockEntry),
            (None, Some(_)) => return Err(PorError::UnexpectedReputationBlockEntry),
            (None, None) => return Ok(()),
        }
    }
}
