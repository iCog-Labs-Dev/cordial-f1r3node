//! Alpha-blended reputation transition.
//!
//! This module blends the current round's Liquid-Rank contribution with the
//! previous reputation vector. Inputs must cover the same canonical node set;
//! no no-rating fallback is applied. It does not clamp values, mutate
//! reputation state, or construct a reputation block.

use crate::{
    config::PorConfig,
    error::PorError,
    types::{ReputationEntry, ReputationVector, ReputationWeight},
};

/// Blend a Liquid-Rank contribution with the previous reputation vector.
///
/// For every contribution entry, this computes:
///
/// `R_next = (alpha * contribution + (scale - alpha) * previous) / scale`
///
/// Both vectors must contain the same canonically ordered node set, and the
/// contribution round must immediately follow the previous reputation round.
/// The caller must pass the same previous vector used to compute the Liquid-Rank
/// contribution because that provenance is not encoded in `ReputationVector`.
pub fn blend_reputation_transition(
    contribution: &ReputationVector,
    previous_reputation: &ReputationVector,
    config: &PorConfig,
) -> Result<ReputationVector, PorError> {
    if config.scale == 0 {
        return Err(PorError::InvalidTransitionScale);
    }

    if config.liquid_rank_alpha > config.scale {
        return Err(PorError::InvalidLiquidRankAlpha);
    }

    if previous_reputation.round.checked_add(1) != Some(contribution.round) {
        return Err(PorError::InvalidTransitionRound);
    }

    validate_reputation_order(contribution)?;
    validate_reputation_order(previous_reputation)?;
    validate_matching_node_sets(contribution, previous_reputation)?;

    let alpha = u128::from(config.liquid_rank_alpha);
    let previous_weight = u128::from(config.scale - config.liquid_rank_alpha);
    let scale = u128::from(config.scale);
    let mut values = Vec::with_capacity(contribution.values.len());

    for (contribution_entry, previous_entry) in
        contribution.values.iter().zip(&previous_reputation.values)
    {
        let contribution_term = alpha
            .checked_mul(u128::from(contribution_entry.reputation))
            .ok_or(PorError::ReputationTransitionOverflow)?;
        let previous_term = previous_weight
            .checked_mul(u128::from(previous_entry.reputation))
            .ok_or(PorError::ReputationTransitionOverflow)?;
        let blended = contribution_term
            .checked_add(previous_term)
            .ok_or(PorError::ReputationTransitionOverflow)?
            / scale;

        let reputation = ReputationWeight::try_from(blended)
            .map_err(|_| PorError::ReputationTransitionOverflow)?;

        values.push(ReputationEntry::new(
            contribution_entry.node_id.clone(),
            reputation,
        ));
    }

    Ok(ReputationVector {
        round: contribution.round,
        values,
    })
}

fn validate_reputation_order(vector: &ReputationVector) -> Result<(), PorError> {
    for entries in vector.values.windows(2) {
        match entries[0].node_id.cmp(&entries[1].node_id) {
            std::cmp::Ordering::Less => {}
            std::cmp::Ordering::Equal => return Err(PorError::DuplicateReputationEntry),
            std::cmp::Ordering::Greater => return Err(PorError::UnsortedReputationVector),
        }
    }

    Ok(())
}

fn validate_matching_node_sets(
    contribution: &ReputationVector,
    previous_reputation: &ReputationVector,
) -> Result<(), PorError> {
    let mut contribution_entries = contribution.values.iter().peekable();
    let mut previous_entries = previous_reputation.values.iter().peekable();

    loop {
        match (contribution_entries.peek(), previous_entries.peek()) {
            (Some(contribution_entry), Some(previous_entry)) => {
                match contribution_entry.node_id.cmp(&previous_entry.node_id) {
                    std::cmp::Ordering::Less => return Err(PorError::MissingPreviousReputation),
                    std::cmp::Ordering::Equal => {
                        contribution_entries.next();
                        previous_entries.next();
                    }
                    std::cmp::Ordering::Greater => return Err(PorError::MissingContributionEntry),
                }
            }
            (Some(_), None) => return Err(PorError::MissingPreviousReputation),
            (None, Some(_)) => return Err(PorError::MissingContributionEntry),
            (None, None) => return Ok(()),
        }
    }
}
