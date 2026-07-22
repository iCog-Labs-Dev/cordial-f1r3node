use std::collections::HashMap;

use cordial_miners_core::NodeId;

use crate::state::ReputationState;
use crate::types::ReputationWeight;

/// Export current reputation values as Cordial Miners weighted-path inputs.
///
/// TODO: future weight export may support policies such as reputation-only,
/// stake-times-reputation, capped stake, or committee-only weights.
pub fn reputation_weights(state: &ReputationState) -> HashMap<NodeId, ReputationWeight> {
    state
        .reputations()
        .iter()
        .map(|(validator, reputation)| (validator.clone(), *reputation))
        .collect()
}
