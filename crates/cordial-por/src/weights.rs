use std::collections::HashMap;

use cordial_miners_core::NodeId;

use crate::state::ReputationState;
use crate::types::ReputationWeight;

/// Export reputation values as Cordial Miners weighted-path inputs.
///
/// This crate computes weights.
/// Cordial Miners consumes them.
pub fn reputation_weights(state: &ReputationState) -> HashMap<NodeId, ReputationWeight> {
    state
        .reputation_list()
        .entries
        .iter()
        .map(|entry| (entry.node_id.clone(), entry.reputation))
        .collect()
}
