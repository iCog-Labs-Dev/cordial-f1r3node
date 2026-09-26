use std::collections::HashMap;

use cordial_miners_core::NodeId;

use crate::error::PorError;
use crate::state::ReputationState;
use crate::types::ReputationWeight;

/// Export the active reputation-state map.
///
/// Permanently ejected entries (`is_excluded = true`) are omitted from the
/// returned map.
///
/// This is an active PoR-state export, not a validator-membership decision.
/// Runtime consensus integration must use [`authorized_validator_weights`]
/// so Cordial's existing authorized set remains authoritative.
pub fn reputation_weights(state: &ReputationState) -> HashMap<NodeId, ReputationWeight> {
    state
        .reputation_list()
        .entries
        .iter()
        .filter(|entry| !entry.is_excluded)
        .map(|entry| (entry.node_id.clone(), entry.reputation))
        .collect()
}

/// Project PoR weights onto Cordial's existing authorized validator set.
///
/// The returned map has exactly the distinct identities supplied by Cordial:
/// PoR cannot add validators, choose a committee, or remove membership. An
/// ejected identity remains present with zero weight. Missing reputations,
/// empty authorization, zero total weight, and arithmetic overflow fail closed.
pub fn authorized_validator_weights(
    state: &ReputationState,
    authorized_validators: &[NodeId],
) -> Result<HashMap<NodeId, ReputationWeight>, PorError> {
    if authorized_validators.is_empty() {
        return Err(PorError::EmptyAuthorizedValidatorSet);
    }

    let mut weights = HashMap::with_capacity(authorized_validators.len());
    let mut total_weight = 0u128;

    for validator in authorized_validators {
        if weights.contains_key(validator) {
            continue;
        }

        let entry = state
            .reputation_list()
            .entries
            .binary_search_by(|entry| entry.node_id.cmp(validator))
            .ok()
            .map(|index| &state.reputation_list().entries[index])
            .ok_or_else(|| PorError::MissingAuthorizedValidatorReputation(validator.clone()))?;
        let weight = if entry.is_excluded || state.is_ejected(validator) {
            0
        } else {
            entry.reputation
        };
        total_weight = total_weight
            .checked_add(u128::from(weight))
            .ok_or(PorError::AuthorizedValidatorWeightOverflow)?;
        weights.insert(validator.clone(), weight);
    }

    if total_weight == 0 {
        return Err(PorError::ZeroAuthorizedValidatorWeight);
    }

    Ok(weights)
}
