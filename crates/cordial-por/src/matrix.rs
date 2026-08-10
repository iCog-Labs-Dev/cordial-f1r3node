//! Rating matrix construction from a validated round batch.
//!
//! This module is intentionally narrow: it converts a single-round
//! `RatingBatch` into a deterministic `RatingMatrix` representation for later
//! PoR stages. It does not compute weights, normalize values, or update
//! reputation state.

use crate::{
    error::PorError,
    types::{RatingBatch, RatingMatrix},
};

/// Convert a validated `RatingBatch` into a canonical matrix representation.
pub fn build_rating_matrix(batch: &RatingBatch) -> Result<RatingMatrix, PorError> {
    let mut ordered = batch.ratings.clone();

    for rating in &ordered {
        if rating.round != batch.round {
            return Err(PorError::InvalidRatingRound);
        }
    }

    ordered.sort_by(|a, b| {
        a.recipient
            .cmp(&b.recipient)
            .then_with(|| a.rater.cmp(&b.rater))
    });

    for window in ordered.windows(2) {
        let previous = &window[0];
        let current = &window[1];

        if previous.recipient == current.recipient && previous.rater == current.rater {
            return Err(PorError::DuplicateMatrixEntry);
        }
    }

    Ok(RatingMatrix {
        round: batch.round,
        ratings: ordered,
    })
}
