//! Proof-of-Reputation state and data model layer.
//!
//! This crate owns PoR vocabulary and reputation-derived weights.
//!
//! Consensus logic remains inside cordial-miners-core.

pub mod config;
pub mod error;
pub mod matrix;
pub mod ratings;
pub mod state;
pub mod types;
pub mod weights;

pub use config::PorConfig;
pub use error::PorError;
pub use matrix::build_rating_matrix;
pub use ratings::{build_rating_batch, validate_rating};
pub use state::ReputationState;

pub use types::{
    EquivocationPenalty, InactivityPenalty, RatingBatch, RatingRecord, RatingScore,
    ReputationBlock, ReputationBlockHeader, ReputationEntry, ReputationList, ReputationRound,
    ReputationVector, ReputationWeight,
};

pub use weights::reputation_weights;
