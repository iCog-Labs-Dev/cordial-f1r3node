// Alpha-blended reputation transition.
//
// This module blends the current round's Liquid-Rank contribution with the
// previous reputation vector. It does not clamp values, mutate reputation
// state, or construct a reputation block.

use cordial_miners_core::NodeId;

use crate::{
    config::PorConfig,
    error::PorError,
    types::{ReputationEntry, ReputationVector, ReputationWeight},
};

// Blend a Liquid-Rank contribution with the previous reputation vector.
//
// For every contribution entry, this computes:
//
// `R_next = (alpha * contribution + (scale - alpha) * previous) / scale`
//
// Output entries preserve the contribution vector's canonical `NodeId`
// ordering, and the output round is the contribution round.
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

    validate_reputation_order(contribution)?;
    validate_reputation_order(previous_reputation)?;

    let alpha = config.liquid_rank_alpha;
    let previous_weight = config.scale - config.liquid_rank_alpha;
    let mut values = Vec::with_capacity(contribution.values.len());

    for contribution_entry in &contribution.values {
        let previous = reputation_of(previous_reputation, &contribution_entry.node_id)?;

        let contribution_term = alpha
            .checked_mul(contribution_entry.reputation)
            .ok_or(PorError::ReputationTransitionOverflow)?;
        let previous_term = previous_weight
            .checked_mul(previous)
            .ok_or(PorError::ReputationTransitionOverflow)?;
        let reputation = contribution_term
            .checked_add(previous_term)
            .ok_or(PorError::ReputationTransitionOverflow)?
            / config.scale;

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

fn reputation_of(
    previous_reputation: &ReputationVector,
    node_id: &NodeId,
) -> Result<ReputationWeight, PorError> {
    let index = previous_reputation
        .values
        .binary_search_by(|entry| entry.node_id.cmp(node_id))
        .map_err(|_| PorError::MissingPreviousReputation)?;

    Ok(previous_reputation.values[index].reputation)
}
