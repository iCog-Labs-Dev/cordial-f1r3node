//! Liquid-Rank reputation contribution calculation.
//!
//! This module computes the paper-guided contribution vector:
//!
//! `P = S * R`
//!
//! where `S` is the normalized rating matrix and `R` is the previous
//! reputation vector. It does not alpha-blend, clamp, or mutate state.

use cordial_miners_core::NodeId;

use crate::{
    config::PorConfig,
    error::PorError,
    types::{
        NormalizedRatingEntry, NormalizedRatingMatrix, ReputationEntry, ReputationVector,
        ReputationWeight,
    },
};

/// Compute the Liquid-Rank contribution vector `P = S * R`.
///
/// The input matrix is expected to be in the canonical `(recipient, rater)`
/// order produced by the rating-matrix and normalization stages. Output values
/// preserve recipient order and use the normalized matrix round.
pub fn compute_liquid_rank_contribution(
    matrix: &NormalizedRatingMatrix,
    previous_reputation: &ReputationVector,
    config: &PorConfig,
) -> Result<ReputationVector, PorError> {
    if config.scale == 0 {
        return Err(PorError::InvalidLiquidRankScale);
    }

    validate_previous_reputation_order(previous_reputation)?;

    let mut values = Vec::with_capacity(recipient_group_count(matrix));

    for group in matrix
        .ratings
        .chunk_by(|left, right| left.recipient == right.recipient)
    {
        push_recipient_contribution(group, previous_reputation, config.scale, &mut values)?;
    }

    Ok(ReputationVector {
        round: matrix.round,
        values,
    })
}

fn validate_previous_reputation_order(
    previous_reputation: &ReputationVector,
) -> Result<(), PorError> {
    for entries in previous_reputation.values.windows(2) {
        match entries[0].node_id.cmp(&entries[1].node_id) {
            std::cmp::Ordering::Less => {}
            std::cmp::Ordering::Equal => return Err(PorError::DuplicateReputationEntry),
            std::cmp::Ordering::Greater => return Err(PorError::UnsortedReputationVector),
        }
    }

    Ok(())
}

fn recipient_group_count(matrix: &NormalizedRatingMatrix) -> usize {
    matrix
        .ratings
        .chunk_by(|left, right| left.recipient == right.recipient)
        .count()
}

fn push_recipient_contribution(
    group: &[NormalizedRatingEntry],
    previous_reputation: &ReputationVector,
    scale: ReputationWeight,
    output: &mut Vec<ReputationEntry>,
) -> Result<(), PorError> {
    let Some(first) = group.first() else {
        return Ok(());
    };

    let scale_u128 = u128::from(scale);
    let mut weighted_score_sum = 0u128;

    for rating in group {
        let rater_reputation = reputation_of(previous_reputation, &rating.rater)?;
        let weighted_score = u128::from(rating.normalized_score) * u128::from(rater_reputation);

        weighted_score_sum = weighted_score_sum
            .checked_add(weighted_score)
            .ok_or(PorError::LiquidRankOverflow)?;
    }

    let contribution_sum = weighted_score_sum / scale_u128;
    let reputation =
        ReputationWeight::try_from(contribution_sum).map_err(|_| PorError::LiquidRankOverflow)?;

    output.push(ReputationEntry::new(first.recipient.clone(), reputation));

    Ok(())
}

fn reputation_of(
    previous_reputation: &ReputationVector,
    node_id: &NodeId,
) -> Result<ReputationWeight, PorError> {
    let index = previous_reputation
        .values
        .binary_search_by(|entry| entry.node_id.cmp(node_id))
        .map_err(|_| PorError::MissingRaterReputation)?;

    Ok(previous_reputation.values[index].reputation)
}
