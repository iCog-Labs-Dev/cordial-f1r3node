use cordial_miners_core::NodeId;

use crate::types::{
    RatingRecord, ReputationBlock, ReputationEntry, ReputationList, ReputationRound,
    ReputationWeight,
};

/// Local Proof-of-Reputation state.
///
/// This stores PoR data only.
/// It does not calculate reputation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReputationState {
    current_round: ReputationRound,

    reputation_list: ReputationList,

    pending_ratings: Vec<RatingRecord>,

    latest_block: Option<ReputationBlock>,
}

impl ReputationState {
    pub fn new(round: ReputationRound) -> Self {
        Self {
            current_round: round,

            reputation_list: ReputationList {
                round,
                entries: Vec::new(),
            },

            pending_ratings: Vec::new(),

            latest_block: None,
        }
    }

    pub fn round(&self) -> ReputationRound {
        self.current_round
    }

    pub fn reputation_list(&self) -> &ReputationList {
        &self.reputation_list
    }

    pub fn reputation_list_mut(&mut self) -> &mut ReputationList {
        &mut self.reputation_list
    }

    pub fn pending_ratings(&self) -> &[RatingRecord] {
        &self.pending_ratings
    }

    pub fn latest_block(&self) -> Option<&ReputationBlock> {
        self.latest_block.as_ref()
    }

    pub fn add_rating(&mut self, rating: RatingRecord) {
        self.pending_ratings.push(rating);
    }

    pub fn set_reputation(&mut self, node_id: NodeId, reputation: ReputationWeight) {
        if let Some(entry) = self
            .reputation_list
            .entries
            .iter_mut()
            .find(|entry| entry.node_id == node_id)
        {
            entry.reputation = reputation;
        } else {
            self.reputation_list
                .entries
                .push(ReputationEntry::new(node_id, reputation));

            self.reputation_list
                .entries
                .sort_by(|a, b| a.node_id.cmp(&b.node_id));
        }
    }
}

impl Default for ReputationState {
    fn default() -> Self {
        Self::new(0)
    }
}
