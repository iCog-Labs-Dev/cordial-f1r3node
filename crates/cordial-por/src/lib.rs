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
pub mod evidence;
pub mod interactions;
pub mod liquid_rank;
pub mod matrix;
pub mod normalization;
pub mod penalties;
pub mod ratings;
pub mod snapshot;
pub mod state;
pub mod transition;
pub mod types;
pub mod weights;

pub use audit::{
    replay_reputation_transition, replay_reputation_transition_with_penalties,
    verify_checkpoint_reputation_transition,
    verify_checkpoint_reputation_transition_with_penalties, verify_reputation_transition,
    verify_reputation_transition_with_penalties,
};
pub use commitments::{
    MAX_PENALTY_EVIDENCE_LEN, POR_CHECKPOINT_RATING_BATCH_COMMITMENT_DOMAIN,
    POR_CHECKPOINT_REPUTATION_BLOCK_COMMITMENT_DOMAIN, POR_CONFIG_COMMITMENT_DOMAIN,
    POR_PENALTY_EVENTS_COMMITMENT_DOMAIN, POR_RATING_BATCH_COMMITMENT_DOMAIN,
    POR_REPUTATION_BLOCK_COMMITMENT_DOMAIN, POR_REPUTATION_LIST_COMMITMENT_DOMAIN,
    checkpoint_rating_batch_commitment, config_commitment, penalty_events_commitment,
    rating_batch_commitment, reputation_block_hash, reputation_list_commitment,
};
pub use config::{MissingEntryPolicy, PorConfig};
pub use error::PorError;
pub use evidence::{
    CheckpointEquivocationAuthorizationContext, CheckpointEquivocationAuthorizationPolicy,
    EquivocationAuthorizationContext, EquivocationAuthorizationPolicy,
    POR_CHECKPOINT_EQUIVOCATION_AUTHORIZATION_DOMAIN, POR_EQUIVOCATION_AUTHORIZATION_DOMAIN,
    checkpoint_equivocation_authorization_payload,
    decode_checkpoint_equivocation_authorization_payload, equivocation_authorization_payload,
};
pub use interactions::{
    AdmittedCheckpointInteraction, AdmittedInteraction, CheckpointInteractionEvidence,
    InteractionEvidence, InteractionKind, admit_checkpoint_interaction_evidence,
    admit_interaction_evidence, build_rating_from_interaction,
    score_admitted_checkpoint_interaction, score_admitted_interaction, validate_checkpoint_rater,
};
pub use liquid_rank::compute_liquid_rank_contribution;
pub use matrix::build_rating_matrix;
pub use normalization::normalize_rating_matrix;
pub use penalties::{
    apply_inactivity_decay, apply_slash_to_reputation, compute_inactivity_decay,
    compute_slash_penalty,
};
pub use ratings::{
    CHECKPOINT_RATING_SIGNING_DOMAIN, POR_FINALIZED_BLOCKS_PER_CHECKPOINT, RATING_SIGNING_DOMAIN,
    RatingSourceCheckpoint, build_rating_batch, canonical_checkpoint_rating_payload,
    canonical_rating_payload, rating_round_from_finalized_wave, validate_rating,
};
pub use snapshot::{
    MAX_REPUTATION_STATE_ENTRIES, MAX_REPUTATION_STATE_NODE_ID_LEN,
    MAX_REPUTATION_STATE_SNAPSHOT_LEN, POR_CHECKPOINT_STATE_SNAPSHOT_VERSION,
    POR_STATE_SNAPSHOT_MAGIC, POR_STATE_SNAPSHOT_VERSION, decode_reputation_state_snapshot,
    encode_reputation_state_snapshot,
};
pub use state::ReputationState;

pub use block::{
    CHECKPOINT_REPUTATION_BLOCK_VERSION, CheckpointReputationBlockContext,
    MAX_REPUTATION_BLOCK_ENTRIES, MAX_REPUTATION_BLOCK_NODE_ID_LEN,
    MAX_REPUTATION_BLOCK_SHARD_ID_LEN, MAX_REPUTATION_BLOCK_WIRE_LEN,
    POR_CHECKPOINT_REPUTATION_BLOCK_WIRE_VERSION, POR_REPUTATION_BLOCK_MAGIC,
    POR_REPUTATION_BLOCK_WIRE_VERSION, REPUTATION_BLOCK_VERSION, ReputationBlockContext,
    build_checkpoint_reputation_block, build_checkpoint_reputation_block_with_penalties,
    build_reputation_block, build_reputation_block_with_penalties, decode_reputation_block,
    encode_reputation_block, validate_reputation_block,
};
pub use types::{
    EquivocationPenalty, InactivityPenalty, NormalizedRatingEntry, NormalizedRatingMatrix,
    RatingBatch, RatingMatrix, RatingRecord, RatingScore, ReputationBlock, ReputationBlockHeader,
    ReputationCommitment, ReputationEntry, ReputationList, ReputationPenaltyEvents,
    ReputationRound, ReputationVector, ReputationWeight,
};

pub use clamp::{clamp_reputation_transition, clamp_reputation_value, clamp_reputation_vector};
pub use transition::blend_reputation_transition;
pub use weights::{authorized_validator_weights, reputation_weights};
