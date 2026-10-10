//! Alpha-blended reputation transition.
//!
//! This module blends the current round's Liquid-Rank contribution with the
//! previous reputation vector. Rounds are sparse in practice, so nodes covered
//! by only one of the two vectors are resolved through
//! `PorConfig::missing_entry_policy` rather than rejected outright. It does not
//! clamp values, mutate reputation state, or construct a reputation block.
//! CarryForward copies the previous reputation into the blend; the pipeline
//! clamp (`clamp_reputation_transition`) then restores those entries from
//! previous reputation so the sigmoid does not decay an already-finalized
//! value.

pub use crate::penalties::{
    apply_slash_to_reputation, compute_inactivity_decay, compute_slash_penalty,
};

use crate::{
    config::{MissingEntryPolicy, PorConfig},
    error::PorError,
    types::{ReputationEntry, ReputationVector, ReputationWeight},
};

/// Blend a Liquid-Rank contribution with the previous reputation vector.
///
/// For every node, this computes:
///
/// `R_next = (alpha * contribution + (scale - alpha) * previous) / scale`
///
/// Both vectors must be canonically ordered and the contribution round must
/// immediately follow the previous reputation round. The output covers the
/// union of the two node sets, in the same canonical order. A node missing from
/// either side is resolved through `config.missing_entry_policy`, so a sparse
/// round no longer rejects the whole transition.
///
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

    config.validate()?;

    if previous_reputation.round.checked_add(1) != Some(contribution.round) {
        return Err(PorError::InvalidTransitionRound);
    }

    validate_reputation_order(contribution)?;
    validate_reputation_order(previous_reputation)?;

    let contributions = &contribution.values;
    let previous_values = &previous_reputation.values;
    let mut contribution_index = 0;
    let mut previous_index = 0;
    let mut values = Vec::with_capacity(contributions.len().max(previous_values.len()));

    loop {
        // Which side the next node comes from: Equal means both, Less means the
        // contribution alone, Greater means the previous reputation alone.
        let side = match (
            contributions.get(contribution_index),
            previous_values.get(previous_index),
        ) {
            (Some(entry), Some(previous_entry)) => entry.node_id.cmp(&previous_entry.node_id),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => break,
        };

        let (node_id, contribution_value, previous_value) = match side {
            std::cmp::Ordering::Equal => {
                let entry = &contributions[contribution_index];
                let previous_entry = &previous_values[previous_index];

                contribution_index += 1;
                previous_index += 1;

                (&entry.node_id, entry.reputation, previous_entry.reputation)
            }
            std::cmp::Ordering::Less => {
                let entry = &contributions[contribution_index];

                contribution_index += 1;

                (
                    &entry.node_id,
                    entry.reputation,
                    previous_for_new_node(config)?,
                )
            }
            std::cmp::Ordering::Greater => {
                let previous_entry = &previous_values[previous_index];

                previous_index += 1;

                (
                    &previous_entry.node_id,
                    contribution_for_unrated_node(previous_entry.reputation, config)?,
                    previous_entry.reputation,
                )
            }
        };

        let reputation = blend_reputation(contribution_value, previous_value, config)?;

        values.push(ReputationEntry::new(node_id.clone(), reputation));
    }

    Ok(ReputationVector {
        round: contribution.round,
        values,
    })
}

/// Resolve the contribution to use for a node that received no ratings.
fn contribution_for_unrated_node(
    previous: ReputationWeight,
    config: &PorConfig,
) -> Result<ReputationWeight, PorError> {
    match config.missing_entry_policy {
        MissingEntryPolicy::Reject => Err(PorError::MissingContributionEntry),
        MissingEntryPolicy::CarryForward => Ok(previous),
        MissingEntryPolicy::Neutral => Ok(config.initial_reputation),
    }
}

/// Resolve the previous reputation to use for a node joining this round.
fn previous_for_new_node(config: &PorConfig) -> Result<ReputationWeight, PorError> {
    match config.missing_entry_policy {
        MissingEntryPolicy::Reject => Err(PorError::MissingPreviousReputation),
        MissingEntryPolicy::CarryForward | MissingEntryPolicy::Neutral => {
            Ok(config.initial_reputation)
        }
    }
}

fn blend_reputation(
    contribution: ReputationWeight,
    previous: ReputationWeight,
    config: &PorConfig,
) -> Result<ReputationWeight, PorError> {
    let alpha = u128::from(config.liquid_rank_alpha);
    let previous_weight = u128::from(config.scale - config.liquid_rank_alpha);
    let contribution_term = alpha
        .checked_mul(u128::from(contribution))
        .ok_or(PorError::ReputationTransitionOverflow)?;
    let previous_term = previous_weight
        .checked_mul(u128::from(previous))
        .ok_or(PorError::ReputationTransitionOverflow)?;
    let blended = contribution_term
        .checked_add(previous_term)
        .ok_or(PorError::ReputationTransitionOverflow)?
        / u128::from(config.scale);

    ReputationWeight::try_from(blended).map_err(|_| PorError::ReputationTransitionOverflow)
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

/// Apply externally finalized penalties after blend/clamp, before publishing
/// the snapshot. Offenders receive no rating reward in a fault/missed round:
/// deductions use their previous-round reputation. Equivocating keys are
/// permanently excluded with zero active weight; the surviving balance is
/// stored separately as retained reputation. Capital transfer is host-owned.
pub(crate) fn apply_reputation_penalties(
    next: &mut crate::types::ReputationList,
    previous: &ReputationVector,
    ratings: &[crate::types::RatingRecord],
    events: &crate::types::ReputationPenaltyEvents,
    config: &PorConfig,
) -> Result<(), PorError> {
    use crate::penalties::compute_slash_penalty_wide;
    use std::collections::BTreeSet;

    config.validate()?;
    if events.round != next.round || previous.round.checked_add(1) != Some(events.round) {
        return Err(PorError::InvalidPenaltyEvents(
            "event round must match the next snapshot".into(),
        ));
    }
    let mut seen = BTreeSet::new();
    let prior_entry = |offender: &cordial_miners_core::NodeId| {
        let index = previous
            .values
            .binary_search_by(|entry| entry.node_id.cmp(offender))
            .map_err(|_| PorError::InvalidPenaltyEvents("unknown penalty offender".into()))?;
        let entry = &previous.values[index];
        if entry.is_excluded {
            return Err(PorError::InvalidPenaltyEvents(
                "cannot penalize an already ejected key".into(),
            ));
        }
        Ok(entry)
    };
    let mut equivocating_weight = 0u128;
    for event in &events.equivocations {
        if !seen.insert(&event.offender) || event.evidence.is_empty() {
            return Err(PorError::InvalidPenaltyEvents(
                "duplicate offender or empty equivocation evidence".into(),
            ));
        }
        equivocating_weight = equivocating_weight
            .checked_add(u128::from(prior_entry(&event.offender)?.reputation))
            .ok_or(PorError::SlashOverflow)?;
    }
    for event in &events.inactivity {
        if !seen.insert(&event.offender) || event.missed_rounds != 1 {
            return Err(PorError::InvalidPenaltyEvents(
                "inactivity requires one missed round and a unique offender".into(),
            ));
        }
        prior_entry(&event.offender)?;
        if ratings
            .iter()
            .any(|rating| rating.rater == event.offender || rating.recipient == event.offender)
        {
            return Err(PorError::InvalidPenaltyEvents(
                "inactive offender participates in the rating batch".into(),
            ));
        }
    }
    // Validate all inputs before touching the caller's output.
    let mut staged = next.clone();
    if !events.equivocations.is_empty() {
        let total_weight = previous
            .values
            .iter()
            .filter(|entry| !entry.is_excluded)
            .try_fold(0u128, |sum, entry| {
                sum.checked_add(u128::from(entry.reputation))
                    .ok_or(PorError::SlashOverflow)
            })?;
        let penalty = compute_slash_penalty_wide(equivocating_weight, total_weight, config)?;
        for event in &events.equivocations {
            let index = staged
                .entries
                .binary_search_by(|entry| entry.node_id.cmp(&event.offender))
                .map_err(|_| PorError::MissingReputationBlockEntry)?;
            staged.entries[index].retained_reputation = apply_slash_to_reputation(
                prior_entry(&event.offender)?.reputation,
                penalty,
                config,
            )?;
            staged.entries[index].reputation = 0;
            staged.entries[index].is_excluded = true;
        }
    }
    for event in &events.inactivity {
        let index = staged
            .entries
            .binary_search_by(|entry| entry.node_id.cmp(&event.offender))
            .map_err(|_| PorError::MissingReputationBlockEntry)?;
        staged.entries[index].reputation = compute_inactivity_decay(
            prior_entry(&event.offender)?.reputation,
            config.inactivity_decay_gamma,
            config,
        )?;
    }
    *next = staged;
    Ok(())
}
