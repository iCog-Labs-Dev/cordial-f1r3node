use std::collections::{HashMap, HashSet};

use cordial_miners_core::NodeId;

use crate::state::ReputationState;
use crate::types::ReputationWeight;

/// Export reputation values as Cordial Miners weighted-path inputs.
///
/// Permanently ejected entries (`is_excluded = true`) are omitted from the
/// returned map. The consensus weighted path only receives active validators.
///
/// This crate computes weights.
/// Cordial Miners consumes them.
pub fn reputation_weights(state: &ReputationState) -> HashMap<NodeId, ReputationWeight> {
    state
        .reputation_list()
        .entries
        .iter()
        .filter(|entry| !entry.is_excluded)
        .map(|entry| (entry.node_id.clone(), entry.reputation))
        .collect()
}

/// Export weights for validators selected by the caller.
///
/// Selection remains outside Proof-of-Reputation. For every selected validator
/// present in `state`, its reputation is copied unchanged into the returned
/// map. Unknown validators are omitted. Permanently ejected validators
/// (`is_excluded = true`) are also omitted regardless of selection.
pub fn selected_validator_weights(
    state: &ReputationState,
    selected_validators: &[NodeId],
) -> HashMap<NodeId, ReputationWeight> {
    let selected: HashSet<&NodeId> = selected_validators.iter().collect();

    state
        .reputation_list()
        .entries
        .iter()
        .filter(|entry| !entry.is_excluded && selected.contains(&entry.node_id))
        .map(|entry| (entry.node_id.clone(), entry.reputation))
        .collect()
}
