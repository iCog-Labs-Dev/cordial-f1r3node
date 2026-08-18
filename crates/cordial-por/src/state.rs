use cordial_miners_core::NodeId;

use crate::{
    error::PorError,
    types::{
        RatingRecord, ReputationBlock, ReputationEntry, ReputationList, ReputationRound,
        ReputationVector, ReputationWeight,
    },
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
        match self
            .reputation_list
            .entries
            .binary_search_by(|entry| entry.node_id.cmp(&node_id))
        {
            Ok(index) => {
                self.reputation_list.entries[index].reputation = reputation;
            }
            Err(index) => {
                self.reputation_list
                    .entries
                    .insert(index, ReputationEntry::new(node_id, reputation));
            }
        }
    }

    /// Apply a finalized reputation vector as the current state snapshot.
    ///
    /// The vector is expected to be the already-computed output of the
    /// calculation pipeline. This method validates canonical ordering, takes
    /// ownership of the vector entries, and replaces the state's reputation
    /// list; it does not recompute ratings, Liquid Rank, transition, or
    /// clamping.
    pub fn apply_reputation_vector(&mut self, vector: ReputationVector) -> Result<(), PorError> {
        validate_reputation_vector(&vector)?;

        self.current_round = vector.round;
        self.reputation_list = ReputationList {
            round: vector.round,
            entries: vector.values,
        };

        Ok(())
    }
}

impl Default for ReputationState {
    fn default() -> Self {
        Self::new(0)
    }
}

fn validate_reputation_vector(vector: &ReputationVector) -> Result<(), PorError> {
    for entries in vector.values.windows(2) {
        match entries[0].node_id.cmp(&entries[1].node_id) {
            std::cmp::Ordering::Less => {}
            std::cmp::Ordering::Equal => return Err(PorError::DuplicateReputationEntry),
            std::cmp::Ordering::Greater => return Err(PorError::UnsortedReputationVector),
        }
    }

    Ok(())
}
