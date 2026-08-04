//! Rating validation and deterministic round batching.
//!
//! This module is intentionally narrow: it validates incoming `RatingRecord`
//! values against the configured bounds and assembles a single-round batch with a
//! deterministic ordering. It does not compute reputation, Liquid Rank, or any
//! future consensus-state transitions.

use std::collections::HashSet;

use cordial_miners_core::NodeId;

use crate::{
    config::PorConfig,
    error::PorError,
    types::{RatingBatch, RatingRecord, ReputationRound},
};

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
    let mut seen: HashSet<(ReputationRound, NodeId, NodeId)> = HashSet::new();
    let mut validated = Vec::with_capacity(ratings.len());

    for rating in ratings {
        if rating.round != round {
            return Err(PorError::InvalidRatingRound);
        }

        validate_rating(&rating, config)?;

        let key = (rating.round, rating.rater.clone(), rating.recipient.clone());
        if !seen.insert(key) {
            return Err(PorError::DuplicateRating);
        }

        validated.push(rating);
    }

    validated.sort_by_key(|rating| (rating.recipient.clone(), rating.rater.clone()));

    Ok(RatingBatch {
        round,
        ratings: validated,
    })
}
