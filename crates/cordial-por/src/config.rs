use crate::types::{RatingScore, ReputationWeight};

/// Configuration parameters for PoR calculations and transitions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PorConfig {
    /// Fixed point scale.
    pub scale: ReputationWeight,

    /// Initial reputation.
    pub initial_reputation: ReputationWeight,

    /// Fixed-point alpha used to blend Liquid-Rank contribution with prior
    /// reputation.
    pub liquid_rank_alpha: ReputationWeight,

    /// Minimum accepted rating.
    pub minimum_rating: RatingScore,

    /// Maximum accepted rating.
    pub maximum_rating: RatingScore,
}

impl PorConfig {
    pub const DEFAULT_SCALE: ReputationWeight = 1_000_000_000;

    pub const DEFAULT_INITIAL_REPUTATION: ReputationWeight = 200_000_000;

    pub fn new(scale: ReputationWeight, initial_reputation: ReputationWeight) -> Self {
        Self {
            scale,
            initial_reputation,

            liquid_rank_alpha: 600_000_000,

            minimum_rating: 0,

            maximum_rating: scale,
        }
    }
}

impl Default for PorConfig {
    fn default() -> Self {
        Self::new(Self::DEFAULT_SCALE, Self::DEFAULT_INITIAL_REPUTATION)
    }
}
