//! Rating validation and deterministic round batching.
//!
//! This module is intentionally narrow: it validates incoming `RatingRecord`
//! values against the configured bounds and assembles a single-round batch with a
//! deterministic ordering. It does not compute reputation, Liquid Rank, or any
//! future consensus-state transitions.

use crate::{
    config::PorConfig,
    error::PorError,
    types::{RatingBatch, RatingRecord, ReputationRound},
};

/// Domain separator prepended to every canonical PoR rating payload.
///
/// The version is part of the domain so future encoding changes cannot make a
/// signature valid under both the old and new protocols.
pub const RATING_SIGNING_DOMAIN: &[u8] = b"cordial-por:rating:v1";

/// Domain for ratings bound to a finalized-prefix checkpoint.
pub const CHECKPOINT_RATING_SIGNING_DOMAIN: &[u8] = b"cordial-por:rating:v2";

/// Source identity for one checkpoint-driven PoR round.
///
/// The prefix hash commits every ordered block identity through the boundary.
/// The fixed 16-block cadence is part of this version of the protocol.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RatingSourceCheckpoint {
    pub round: ReputationRound,
    pub finalized_prefix_len: u64,
    pub finalized_prefix_hash: [u8; 32],
}

impl RatingSourceCheckpoint {
    /// Check the fixed source boundary and a nonzero consecutive PoR round.
    pub fn validate(&self) -> Result<(), PorError> {
        if self.round == 0
            || self.round.checked_mul(POR_FINALIZED_BLOCKS_PER_CHECKPOINT)
                != Some(self.finalized_prefix_len)
        {
            return Err(PorError::InvalidRatingSourceCheckpoint);
        }
        Ok(())
    }
}

pub const POR_FINALIZED_BLOCKS_PER_CHECKPOINT: u64 = 16;

/// Encode all signed fields of a rating using the canonical v1 wire layout.
///
/// The signature itself is deliberately excluded. The byte layout is:
///
/// ```text
/// domain
/// round                     u64 big-endian
/// rater                     u64 big-endian length || bytes
/// recipient                 u64 big-endian length || bytes
/// score                     u64 big-endian
/// interaction_ref presence  0x00, or 0x01 || u64 big-endian length || bytes
/// ```
///
/// Length prefixes make the variable-width `NodeId` and interaction reference
/// fields unambiguous. Production interaction ratings require a non-empty
/// `interaction_ref`; that semantic check is performed by the signing and
/// verification boundary rather than by this pure encoder.
pub fn canonical_rating_payload(rating: &RatingRecord) -> Vec<u8> {
    let mut payload = Vec::with_capacity(
        RATING_SIGNING_DOMAIN.len()
            + 8
            + 8
            + rating.rater.0.len()
            + 8
            + rating.recipient.0.len()
            + 8
            + 1
            + rating
                .interaction_ref
                .as_ref()
                .map_or(0, |interaction_ref| 8 + interaction_ref.len()),
    );

    payload.extend_from_slice(RATING_SIGNING_DOMAIN);
    payload.extend_from_slice(&rating.round.to_be_bytes());
    put_bytes(&mut payload, &rating.rater.0);
    put_bytes(&mut payload, &rating.recipient.0);
    payload.extend_from_slice(&rating.score.to_be_bytes());

    match &rating.interaction_ref {
        Some(interaction_ref) => {
            payload.push(1);
            put_bytes(&mut payload, interaction_ref);
        }
        None => payload.push(0),
    }

    payload
}

/// Encode a v2 signed rating bound to its exact finalized-prefix checkpoint.
///
/// The layout is `v2 domain || round || prefix length || prefix hash ||`
/// `rater || recipient || score || interaction reference`, using the same
/// length-prefixed fields as v1. A v1 signature cannot verify against this
/// payload, and two source prefixes cannot share a valid signature.
pub fn canonical_checkpoint_rating_payload(
    rating: &RatingRecord,
    source: &RatingSourceCheckpoint,
) -> Result<Vec<u8>, PorError> {
    if rating.round != source.round {
        return Err(PorError::InvalidRatingRound);
    }
    source.validate()?;

    let mut payload = Vec::with_capacity(
        CHECKPOINT_RATING_SIGNING_DOMAIN.len()
            + 8
            + 8
            + 32
            + 8
            + rating.rater.0.len()
            + 8
            + rating.recipient.0.len()
            + 8
            + 1
            + rating
                .interaction_ref
                .as_ref()
                .map_or(0, |interaction_ref| 8 + interaction_ref.len()),
    );
    payload.extend_from_slice(CHECKPOINT_RATING_SIGNING_DOMAIN);
    payload.extend_from_slice(&source.round.to_be_bytes());
    payload.extend_from_slice(&source.finalized_prefix_len.to_be_bytes());
    payload.extend_from_slice(&source.finalized_prefix_hash);
    put_bytes(&mut payload, &rating.rater.0);
    put_bytes(&mut payload, &rating.recipient.0);
    payload.extend_from_slice(&rating.score.to_be_bytes());
    match &rating.interaction_ref {
        Some(interaction_ref) => {
            payload.push(1);
            put_bytes(&mut payload, interaction_ref);
        }
        None => payload.push(0),
    }
    Ok(payload)
}

fn put_bytes(output: &mut Vec<u8>, bytes: &[u8]) {
    output.extend_from_slice(&(bytes.len() as u64).to_be_bytes());
    output.extend_from_slice(bytes);
}

/// Return the PoR round opened by a finalized Cordial Miners wave.
///
/// Wave `k` is finalized using reputation state `R_k`. Its admitted
/// interactions are then processed in rating round `k + 1`, producing
/// reputation state `R_(k+1)` for subsequent consensus waves. The caller must
/// invoke this only after Cordial Miners has established finality for `wave`.
pub fn rating_round_from_finalized_wave(wave: u64) -> Result<ReputationRound, PorError> {
    wave.checked_add(1).ok_or(PorError::RatingRoundOverflow)
}

/// Validate a single rating record against the protocol configuration.
///
/// This checks the record-level constraints that do not depend on the target
/// batch round; `build_rating_batch` performs the round-level check for the
/// specific batch being assembled.
pub fn validate_rating(rating: &RatingRecord, config: &PorConfig) -> Result<(), PorError> {
    if rating.rater == rating.recipient {
        return Err(PorError::SelfRating);
    }

    if rating.score < config.minimum_rating {
        return Err(PorError::RatingBelowMinimum);
    }

    if rating.score > config.maximum_rating {
        return Err(PorError::RatingAboveMaximum);
    }

    if rating.signature.is_empty() {
        return Err(PorError::MissingRatingSignature);
    }

    Ok(())
}

/// Build a validated rating batch for a single reputation round.
///
/// The output is deterministically ordered by `(recipient, rater)` to match the
/// matrix stage and the paper's `S = [s_ij]` layout (row = recipient,
/// column = rater). This keeps the batch ordering identical to the later
/// matrix ordering for pipeline consistency and rejects invalid, duplicate, or
/// mismatched-round ratings before returning the batch.
pub fn build_rating_batch(
    round: ReputationRound,
    ratings: Vec<RatingRecord>,
    config: &PorConfig,
) -> Result<RatingBatch, PorError> {
    let mut validated = Vec::with_capacity(ratings.len());

    for rating in ratings {
        if rating.round != round {
            return Err(PorError::InvalidRatingRound);
        }

        validate_rating(&rating, config)?;
        validated.push(rating);
    }

    validated.sort_by(|a, b| {
        a.recipient
            .cmp(&b.recipient)
            .then_with(|| a.rater.cmp(&b.rater))
    });

    for window in validated.windows(2) {
        let previous = &window[0];
        let current = &window[1];

        if previous.recipient == current.recipient && previous.rater == current.rater {
            return Err(PorError::DuplicateRating);
        }
    }

    Ok(RatingBatch {
        round,
        ratings: validated,
    })
}
