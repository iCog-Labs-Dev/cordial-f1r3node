//! Proof-of-Reputation integration at the f1r3node adapter boundary.
//!
//! This facade groups finalized evidence, rating collection, optional checkpoint
//! attestation, persistence, and transport without moving those adapter-owned
//! responsibilities into the protocol-math crate.

pub mod activation;
pub mod checkpoint;
mod checkpoint_evidence;
pub mod checkpoint_ratings;
pub mod collector;
pub mod epoch_policy;
pub mod epoch_record;
pub mod evidence;
pub mod finality;
pub mod history;
pub mod interactions;
pub mod lifecycle;
pub mod local_ratings;
pub mod persistence;
pub mod rating_key;
pub mod ratings;
pub mod runtime;
pub mod shadow;
pub mod transition;
pub mod transport;

pub use activation::{
    POR_WEIGHT_ACTIVATION_FILE_NAME, PorWeightActivationOutcome, PorWeightActivationRecord,
    PorWeightActivationStore, PorWeightActivationStoreError,
};
pub use checkpoint::{
    AttestedPorCheckpoint, DEFAULT_POR_CHECKPOINT_THRESHOLD_DENOMINATOR,
    DEFAULT_POR_CHECKPOINT_THRESHOLD_NUMERATOR, PorCheckpointCollector,
    PorCheckpointConflictEvidence, PorCheckpointError, PorCheckpointObservation,
    PorCheckpointPolicy, PorCheckpointProgress,
};
pub use checkpoint_ratings::{
    close_checkpoint_rating_exchange_if_complete, stage_local_checkpoint_rating_batch,
};
pub use collector::{
    BlockProductionRatingCollector, CheckpointBlockProductionRatingCollector,
    PorRatingCollectorError,
};
pub use epoch_policy::{PorWeightEpochPolicy, PorWeightEpochPolicyError};
pub use epoch_record::{
    POR_WEIGHT_EPOCH_FILE_NAME, PorWeightEpochRecord, PorWeightEpochRecordError,
    PorWeightEpochStore,
};
pub use evidence::FinalizedEquivocationAuthorization;
pub use finality::{
    FinalizedPrefixCheckpoint, FinalizedRatingRound, POR_FINALIZED_BLOCKS_PER_CHECKPOINT,
    PorFinalityError, PorFinalityTracker, finalized_prefix_checkpoint,
    validate_finalized_prefix_checkpoint,
};
pub use history::{
    POR_REPUTATION_BLOCK_HISTORY_DIRECTORY, PorReputationBlockAppendOutcome,
    PorReputationBlockHistory, PorReputationBlockHistoryError,
};
pub use interactions::{
    PorInteractionError, admit_checkpoint_block_production_interactions,
    admit_finalized_block_production_interactions, extract_block_production_evidence,
    extract_checkpoint_block_production_evidence, validate_finalized_checkpoint_window,
    validate_finalized_rating_round,
};
pub use lifecycle::checkpoint::CompletedCheckpointRatingRound;
pub use lifecycle::cutoff::{
    DEFAULT_RATING_CUTOFF_WAVE_LAG, PorRatingCutoffError, PorRatingRoundCutoffPolicy,
};
pub use lifecycle::quorum::{
    DEFAULT_RATING_QUORUM_DENOMINATOR, DEFAULT_RATING_QUORUM_NUMERATOR, PorRatingQuorumError,
    PorRatingQuorumProgress, PorRatingRoundClosurePolicy,
};
pub use lifecycle::{
    CompletedPorRatingRound, PorRatingRoundCloseReason, PorRatingRoundCoordinator,
    PorRatingRoundError, PorRatingRoundStatus,
};
pub use persistence::{
    DurablePorState, DurablePorStateError, POR_STATE_DIRECTORY, POR_STATE_FILE_NAME, PorStateStore,
    PorStateStoreError,
};
pub use ratings::{
    PorRatingError, build_finalized_block_production_rating_batch,
    build_finalized_checkpoint_rating_batch, build_verified_checkpoint_rating_batch,
    build_verified_rating_batch, checkpoint_rating_signing_hash, rating_signing_hash,
    sign_admitted_interaction, sign_checkpoint_rating, validate_checkpoint_signed_rating,
    validate_signed_rating, verify_checkpoint_rating_signature, verify_rating_signature,
};
pub use runtime::{CommittedPorRound, PorRuntime, PorRuntimeError};
pub use transition::{AppliedPorReputationRound, apply_completed_reputation_round};
pub use transport::channel::{
    ChannelRatingEnvelopeBroadcaster, ChannelRatingEnvelopeReceiver, PorRatingChannelError,
    RatingEnvelopeReceiveOutcome, bounded_rating_envelope_channel,
};
pub use transport::reputation_block::{
    MAX_REPUTATION_BLOCK_PUBLICATION_LEN, PorReputationBlockPublicationError,
    PorReputationBlockTransportError, REPUTATION_BLOCK_PUBLICATION_DOMAIN,
    REPUTATION_BLOCK_PUBLICATION_VERSION, ReputationBlockEnvelopeBroadcaster,
    ReputationBlockPublicationV1, broadcast_reputation_block,
    broadcast_reputation_block_publication, receive_reputation_block_envelope,
    reputation_block_publication_signing_hash,
};
pub use transport::reputation_block_channel::{
    ChannelReputationBlockEnvelopeBroadcaster, ChannelReputationBlockEnvelopeReceiver,
    PorReputationBlockChannelError, ReputationBlockReceiveOutcome,
    bounded_reputation_block_envelope_channel,
};
pub use transport::wire::{
    BLOCK_PRODUCTION_RATING_ENVELOPE_DOMAIN, BLOCK_PRODUCTION_RATING_ENVELOPE_VERSION,
    BlockProductionRatingEnvelopeV1, BlockProductionRatingEnvelopeV2,
    CHECKPOINT_RATING_ENVELOPE_VERSION, MAX_BLOCK_PRODUCTION_RATING_ENVELOPE_LEN,
    MAX_CHECKPOINT_RATING_ENVELOPE_LEN, PorRatingWireError,
};
pub use transport::{
    FnRatingEnvelopeBroadcaster, PorRatingTransportError, RatingEnvelopeBroadcaster,
    broadcast_rating_batch, encode_checkpoint_rating_batch, encode_rating_batch,
    receive_checkpoint_rating_envelope, receive_rating_envelope,
};
