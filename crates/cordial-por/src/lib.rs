//! Proof-of-Reputation state and data model layer.
//!
//! This crate owns PoR vocabulary and reputation-derived weights.
//!
//! Consensus logic remains inside cordial-miners-core.

pub mod config;
pub mod error;
pub mod liquid_rank;
pub mod matrix;
pub mod normalization;
pub mod ratings;
pub mod state;
pub mod transition;
pub mod types;
pub mod weights;

pub use config::PorConfig;
pub use error::PorError;
pub use liquid_rank::compute_liquid_rank_contribution;
pub use matrix::build_rating_matrix;
pub use normalization::normalize_rating_matrix;
pub use ratings::{build_rating_batch, validate_rating};
pub use state::ReputationState;

pub use types::{
    EquivocationPenalty, InactivityPenalty, NormalizedRatingEntry, NormalizedRatingMatrix,
    RatingBatch, RatingMatrix, RatingRecord, RatingScore, ReputationBlock, ReputationBlockHeader,
    ReputationEntry, ReputationList, ReputationRound, ReputationVector, ReputationWeight,
};

pub use transition::blend_reputation_transition;
pub use weights::reputation_weights;
