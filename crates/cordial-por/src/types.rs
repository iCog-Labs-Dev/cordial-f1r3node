//! Minimal Proof-of-Reputation placeholder types.
//!
//! TODO: define ratings, reputation snapshots, reputation blocks, penalties,
//! committee members, and leader-selection policy in the next design issue.

use cordial_miners_core::NodeId;

pub type ReputationRound = u64;
pub type ReputationWeight = u64;

/// A validator reputation entry in a PoR state snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReputationEntry {
    pub validator: NodeId,
    pub reputation: ReputationWeight,
}

impl ReputationEntry {
    pub fn new(validator: NodeId, reputation: ReputationWeight) -> Self {
        Self {
            validator,
            reputation,
        }
    }
}
