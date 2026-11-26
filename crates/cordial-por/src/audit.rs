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
    block::{
        CHECKPOINT_REPUTATION_BLOCK_VERSION, CheckpointReputationBlockContext,
        REPUTATION_BLOCK_VERSION, ReputationBlockContext, validate_reputation_block,
    },
    clamp::clamp_reputation_transition,
    commitments::{
        checkpoint_rating_batch_commitment, config_commitment, penalty_events_commitment,
        rating_batch_commitment, reputation_block_hash,
    },
    config::PorConfig,
    error::PorError,
    liquid_rank::compute_liquid_rank_contribution,
    matrix::build_rating_matrix,
    normalization::normalize_rating_matrix,
    ratings::build_rating_batch,
    transition::{apply_reputation_penalties, blend_reputation_transition},
    types::{
        RatingBatch, RatingRecord, ReputationBlock, ReputationList, ReputationPenaltyEvents,
        ReputationRound, ReputationVector,
    },
};

/// Replay the deterministic reputation transition for a single round.
///
/// Runs the whole calculation path — batch, matrix, normalization, Liquid-Rank
/// contribution, alpha blend, then the pipeline clamp
/// (`clamp_reputation_transition`) — and returns the expected reputation list.
/// Every rating record must belong to `round`, and `round` must immediately
/// follow `previous_reputation.round`. Input ratings may arrive in any order
/// because batching sorts them canonically.
pub fn replay_reputation_transition(
    previous_reputation: &ReputationVector,
    ratings: &[RatingRecord],
    round: ReputationRound,
    config: &PorConfig,
) -> Result<ReputationList, PorError> {
    replay_reputation_transition_with_penalties(previous_reputation, ratings, round, config, None)
}

/// Replay with optional penalty events for this round. Equivocations require
/// intact proof authorization; inactivity authentication remains caller-owned.
/// Penalties are applied after clamp, using previous-round weights. Missing
/// ratings alone do not trigger decay. `None` preserves the no-penalty path.
pub fn replay_reputation_transition_with_penalties(
    previous_reputation: &ReputationVector,
    ratings: &[RatingRecord],
    round: ReputationRound,
    config: &PorConfig,
    penalties: Option<&ReputationPenaltyEvents>,
) -> Result<ReputationList, PorError> {
    config.validate()?;
    crate::commitments::validate_reputation_vector(previous_reputation)?;
    penalty_events_commitment(round, penalties)?;
    let batch = build_rating_batch(round, ratings.to_vec(), config)?;
    let matrix = build_rating_matrix(&batch)?;
    let normalized = normalize_rating_matrix(&matrix, config)?;
    let contribution = compute_liquid_rank_contribution(&normalized, previous_reputation, config)?;
    let blended = blend_reputation_transition(&contribution, previous_reputation, config)?;
    let clamped =
        clamp_reputation_transition(&blended, previous_reputation, &contribution, config)?;

    let mut entries = clamped.values;
    for entry in &mut entries {
        if let Ok(index) = previous_reputation
            .values
            .binary_search_by(|previous| previous.node_id.cmp(&entry.node_id))
            && previous_reputation.values[index].is_excluded
        {
            entry.reputation = 0;
            entry.is_excluded = true;
            entry.retained_reputation = previous_reputation.values[index].retained_reputation;
        }
    }

    let mut next = ReputationList {
        round: clamped.round,
        entries,
    };
    if let Some(events) = penalties {
        apply_reputation_penalties(&mut next, previous_reputation, ratings, events, config)?;
    }
    Ok(next)
}

/// Verify that a proposed reputation block matches a deterministic replay.
///
/// The block is first put through `validate_reputation_block`, so an audited
/// block is held to exactly the structural rules a constructed one satisfies.
/// The external context then binds it to the expected shard, finalized wave,
/// previous block, configuration, and signed rating batch. Finally, the
/// replayed list is compared entry for entry, so a validator accepts the block
/// only when the recorded ratings and previous reputation actually produce it.
/// Rating signatures must be authenticated by the caller before admission.
pub fn verify_reputation_transition(
    previous_reputation: &ReputationVector,
    ratings: &[RatingRecord],
    proposed_block: &ReputationBlock,
    context: ReputationBlockContext<'_>,
    config: &PorConfig,
) -> Result<(), PorError> {
    verify_reputation_transition_with_penalties(
        previous_reputation,
        ratings,
        proposed_block,
        context,
        config,
        None,
    )
}

/// Verify a penalty-inclusive block against the same finalized events used by
/// its producer. Equivocation authorization is checked against this context;
/// the exact event set is bound by the block's penalties hash. Substitution or
/// omission fails even when the resulting reputation values are identical.
pub fn verify_reputation_transition_with_penalties(
    previous_reputation: &ReputationVector,
    ratings: &[RatingRecord],
    proposed_block: &ReputationBlock,
    context: ReputationBlockContext<'_>,
    config: &PorConfig,
    penalties: Option<&ReputationPenaltyEvents>,
) -> Result<(), PorError> {
    config.validate()?;
    validate_reputation_block(proposed_block)?;
    crate::evidence::validate_penalty_context(penalties, context)?;

    let header = &proposed_block.header;
    if header.version != REPUTATION_BLOCK_VERSION {
        return Err(PorError::UnsupportedReputationBlockAuditVersion(
            header.version,
        ));
    }
    if header.shard_id != context.shard_id {
        return Err(PorError::ReputationBlockShardMismatch);
    }
    if header.source_finalized_wave != context.source_finalized_wave {
        return Err(PorError::ReputationBlockSourceWaveMismatch);
    }

    let expected_previous_hash = match context.previous_block {
        Some(previous) => {
            if previous.header.shard_id != context.shard_id {
                return Err(PorError::PreviousReputationBlockShardMismatch);
            }
            if previous.header.round.checked_add(1) != Some(header.round) {
                return Err(PorError::InvalidPreviousReputationBlockRound);
            }
            Some(reputation_block_hash(previous)?)
        }
        None => None,
    };
    if header.previous_reputation_hash != expected_previous_hash {
        return Err(PorError::ReputationBlockPreviousHashMismatch);
    }
    if header.config_hash != config_commitment(config) {
        return Err(PorError::ReputationBlockConfigHashMismatch);
    }

    let rating_batch = RatingBatch {
        round: header.round,
        ratings: ratings.to_vec(),
    };
    if header.ratings_hash != rating_batch_commitment(&rating_batch, config)? {
        return Err(PorError::ReputationBlockRatingsHashMismatch);
    }

    if header.penalties_hash != penalty_events_commitment(header.round, penalties)? {
        return Err(PorError::ReputationBlockPenaltiesHashMismatch);
    }
    let proposed = &proposed_block.reputation_list;
    let expected = replay_reputation_transition_with_penalties(
        previous_reputation,
        ratings,
        proposed.round,
        config,
        penalties,
    )?;

    compare_reputation_lists(&expected, proposed)
}

/// Audit a v3 block against an externally verified finalized-prefix checkpoint.
///
/// The caller must establish that `context.source` is the canonical checkpoint
/// in Cordial's finalized order and authenticate the rating signatures before
/// using this result to apply state. This function binds the block and batch
/// commitments to that source and replays the reputation calculation.
pub fn verify_checkpoint_reputation_transition(
    previous_reputation: &ReputationVector,
    ratings: &[RatingRecord],
    proposed_block: &ReputationBlock,
    context: CheckpointReputationBlockContext<'_>,
    config: &PorConfig,
) -> Result<(), PorError> {
    verify_checkpoint_reputation_transition_with_penalties(
        previous_reputation,
        ratings,
        proposed_block,
        context,
        config,
        None,
    )
}

/// Audit the exact authorized penalty set against the finalized checkpoint.
pub fn verify_checkpoint_reputation_transition_with_penalties(
    previous_reputation: &ReputationVector,
    ratings: &[RatingRecord],
    proposed_block: &ReputationBlock,
    context: CheckpointReputationBlockContext<'_>,
    config: &PorConfig,
    penalties: Option<&crate::ReputationPenaltyEvents>,
) -> Result<(), PorError> {
    context.source.validate()?;
    crate::evidence::validate_checkpoint_penalty_context(penalties, context)?;
    validate_reputation_block(proposed_block)?;

    let header = &proposed_block.header;
    if header.version != CHECKPOINT_REPUTATION_BLOCK_VERSION {
        return Err(PorError::UnsupportedReputationBlockAuditVersion(
            header.version,
        ));
    }
    if header.shard_id != context.shard_id {
        return Err(PorError::ReputationBlockShardMismatch);
    }
    if header.source_checkpoint != Some(context.source) {
        return Err(PorError::ReputationBlockSourceCheckpointMismatch);
    }

    let expected_previous_hash = match context.previous_block {
        Some(previous) => {
            if previous.header.shard_id != context.shard_id {
                return Err(PorError::PreviousReputationBlockShardMismatch);
            }
            if previous.header.round.checked_add(1) != Some(header.round) {
                return Err(PorError::InvalidPreviousReputationBlockRound);
            }
            Some(reputation_block_hash(previous)?)
        }
        None => None,
    };
    if header.previous_reputation_hash != expected_previous_hash {
        return Err(PorError::ReputationBlockPreviousHashMismatch);
    }
    if header.config_hash != config_commitment(config) {
        return Err(PorError::ReputationBlockConfigHashMismatch);
    }

    let rating_batch = RatingBatch {
        round: header.round,
        ratings: ratings.to_vec(),
    };
    if header.ratings_hash
        != checkpoint_rating_batch_commitment(&rating_batch, &context.source, config)?
    {
        return Err(PorError::ReputationBlockRatingsHashMismatch);
    }
    if header.penalties_hash != penalty_events_commitment(header.round, penalties)? {
        return Err(PorError::ReputationBlockPenaltiesHashMismatch);
    }

    let expected = replay_reputation_transition_with_penalties(
        previous_reputation,
        ratings,
        header.round,
        config,
        penalties,
    )?;
    compare_reputation_lists(&expected, &proposed_block.reputation_list)
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
                        if expected_entry.is_excluded != proposed_entry.is_excluded {
                            return Err(PorError::ReputationExclusionMismatch);
                        }
                        if expected_entry.reputation != proposed_entry.reputation
                            || expected_entry.retained_reputation
                                != proposed_entry.retained_reputation
                        {
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
