use std::collections::BTreeMap;

use cordial_miners_core::NodeId;

use crate::types::{ReputationRound, ReputationWeight};

/// Minimal in-memory reputation state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReputationState {
    round: ReputationRound,
    reputations: BTreeMap<NodeId, ReputationWeight>,
}

impl ReputationState {
    pub fn new(round: ReputationRound) -> Self {
        Self {
            round,
            reputations: BTreeMap::new(),
        }
    }

    pub fn round(&self) -> ReputationRound {
        self.round
    }

    pub fn reputations(&self) -> &BTreeMap<NodeId, ReputationWeight> {
        &self.reputations
    }

    pub fn reputation_of(&self, validator: &NodeId) -> Option<ReputationWeight> {
        self.reputations.get(validator).copied()
    }

    pub fn set_reputation(
        &mut self,
        validator: NodeId,
        reputation: ReputationWeight,
    ) -> Option<ReputationWeight> {
        self.reputations.insert(validator, reputation)
    }
}

impl Default for ReputationState {
    fn default() -> Self {
        Self::new(0)
    }
}
