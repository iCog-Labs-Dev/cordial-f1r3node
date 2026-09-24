//! Proof-of-Reputation integration at the f1r3node adapter boundary.
//!
//! This facade groups finalized evidence, rating signing and collection,
//! lifecycle policy, and transport implementations without moving those
//! adapter-owned responsibilities into the protocol-math crate.

pub mod collector;
pub mod finality;
pub mod interactions;
pub mod lifecycle;
pub mod ratings;
pub mod transport;

pub use collector::{BlockProductionRatingCollector, PorRatingCollectorError};
pub use finality::{FinalizedRatingRound, PorFinalityError, PorFinalityTracker};
pub use interactions::{
    PorInteractionError, admit_finalized_block_production_interactions,
    extract_block_production_evidence, validate_finalized_rating_round,
};
pub use lifecycle::quorum::{
    DEFAULT_RATING_QUORUM_DENOMINATOR, DEFAULT_RATING_QUORUM_NUMERATOR, PorRatingQuorumError,
    PorRatingQuorumProgress, PorRatingRoundClosurePolicy,
};
pub use lifecycle::{PorRatingRoundCoordinator, PorRatingRoundError, PorRatingRoundStatus};
pub use ratings::{
    PorRatingError, build_finalized_block_production_rating_batch, build_verified_rating_batch,
    rating_signing_hash, sign_admitted_interaction, validate_signed_rating,
    verify_rating_signature,
};
pub use transport::channel::{
    ChannelRatingEnvelopeBroadcaster, ChannelRatingEnvelopeReceiver, PorRatingChannelError,
    RatingEnvelopeReceiveOutcome, bounded_rating_envelope_channel,
};
pub use transport::wire::{
    BLOCK_PRODUCTION_RATING_ENVELOPE_DOMAIN, BLOCK_PRODUCTION_RATING_ENVELOPE_VERSION,
    BlockProductionRatingEnvelopeV1, MAX_BLOCK_PRODUCTION_RATING_ENVELOPE_LEN, PorRatingWireError,
};
pub use transport::{
    FnRatingEnvelopeBroadcaster, PorRatingTransportError, RatingEnvelopeBroadcaster,
    broadcast_rating_batch, encode_rating_batch, receive_rating_envelope,
};
