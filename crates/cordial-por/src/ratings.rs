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
