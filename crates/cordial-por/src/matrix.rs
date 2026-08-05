//! Rating matrix construction from a validated round batch.
//!
//! This module is intentionally narrow: it converts a single-round
//! `RatingBatch` into a deterministic `RatingMatrix` representation for later
//! PoR stages. It does not compute weights, normalize values, or update
//! reputation state.

use std::collections::HashSet;

use cordial_miners_core::NodeId;

use crate::{
    error::PorError,
    types::{RatingBatch, RatingMatrix, ReputationRound},
};

/// Convert a validated `RatingBatch` into a canonical matrix representation.
pub fn build_rating_matrix(batch: &RatingBatch) -> Result<RatingMatrix, PorError> {
    let mut seen: HashSet<(ReputationRound, NodeId, NodeId)> =
        HashSet::with_capacity(batch.ratings.len());
    let mut ordered = batch.ratings.clone();

    for rating in &ordered {
        if rating.round != batch.round {
            return Err(PorError::InvalidRatingRound);
        }

        let key = (rating.round, rating.rater.clone(), rating.recipient.clone());
        if !seen.insert(key) {
            return Err(PorError::DuplicateMatrixEntry);
        }
    }

    ordered.sort_by(|a, b| (&a.recipient, &a.rater).cmp(&(&b.recipient, &b.rater)));

    Ok(RatingMatrix {
        round: batch.round,
        ratings: ordered,
    })
}
