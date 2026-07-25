use crate::types::ReputationWeight;

/// Configuration placeholders for future Proof-of-Reputation state updates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PorConfig {
    /// Fixed-point scale used by reputation values.
    pub scale: ReputationWeight,

    /// Reputation assigned to validators before any PoR history exists.
    pub initial_reputation: ReputationWeight,
}

impl PorConfig {
    pub const DEFAULT_SCALE: ReputationWeight = 1_000_000_000;
    pub const DEFAULT_INITIAL_REPUTATION: ReputationWeight = 200_000_000;

    pub fn new(scale: ReputationWeight, initial_reputation: ReputationWeight) -> Self {
        Self {
            scale,
            initial_reputation,
        }
    }
}

impl Default for PorConfig {
    fn default() -> Self {
        Self {
            scale: Self::DEFAULT_SCALE,
            initial_reputation: Self::DEFAULT_INITIAL_REPUTATION,
        }
    }
}
