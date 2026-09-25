//! Proof-of-Reputation state and data model layer.
//!
//! This crate owns PoR vocabulary and reputation-derived weights.
//!
//! Consensus logic remains inside cordial-miners-core.

pub mod audit;
pub mod block;
pub mod clamp;
pub mod commitments;
pub mod config;
pub mod error;
pub mod interactions;
pub mod liquid_rank;
pub mod matrix;
pub mod normalization;
pub mod ratings;
pub mod state;
pub mod transition;
pub mod types;
pub mod weights;

pub use audit::{replay_reputation_transition, verify_reputation_transition};
pub use commitments::{
    POR_CONFIG_COMMITMENT_DOMAIN, POR_RATING_BATCH_COMMITMENT_DOMAIN,
    POR_REPUTATION_BLOCK_COMMITMENT_DOMAIN, POR_REPUTATION_LIST_COMMITMENT_DOMAIN,
    config_commitment, rating_batch_commitment, reputation_block_hash, reputation_list_commitment,
};
pub use config::{MissingEntryPolicy, PorConfig};
pub use error::PorError;
pub use interactions::{
    AdmittedInteraction, InteractionEvidence, InteractionKind, admit_interaction_evidence,
    build_rating_from_interaction, score_admitted_interaction,
};
pub use liquid_rank::compute_liquid_rank_contribution;
pub use matrix::build_rating_matrix;
pub use normalization::normalize_rating_matrix;
pub use ratings::{
    RATING_SIGNING_DOMAIN, build_rating_batch, canonical_rating_payload,
    rating_round_from_finalized_wave, validate_rating,
};
pub use state::ReputationState;

pub use block::{
    MAX_REPUTATION_BLOCK_SHARD_ID_LEN, REPUTATION_BLOCK_VERSION, ReputationBlockContext,
    build_reputation_block, validate_reputation_block,
};
pub use types::{
    EquivocationPenalty, InactivityPenalty, NormalizedRatingEntry, NormalizedRatingMatrix,
    RatingBatch, RatingMatrix, RatingRecord, RatingScore, ReputationBlock, ReputationBlockHeader,
    ReputationCommitment, ReputationEntry, ReputationList, ReputationRound, ReputationVector,
    ReputationWeight,
};

pub use clamp::{clamp_reputation_transition, clamp_reputation_value, clamp_reputation_vector};
pub use transition::blend_reputation_transition;
pub use weights::{reputation_weights, selected_validator_weights};
