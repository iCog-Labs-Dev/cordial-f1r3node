//! Rating matrix normalization.
//!
//! This module implements the paper-guided normalization step before Liquid
//! Rank. It does not compute `P = S * R`, blend reputation, or update state.

use crate::{
    config::PorConfig,
    error::PorError,
    types::{NormalizedRatingEntry, NormalizedRatingMatrix, RatingMatrix, RatingRecord},
};

/// Normalize a rating matrix using the modified Section 4.2 PoR formula.
///
/// The input is expected to be in the canonical `(recipient, rater)` order
/// produced by `build_rating_matrix`. Ratings are grouped by recipient node.
/// For each recipient group, scores are normalized using:
///
/// `((score - min) + scale) * scale / ((max - min) + scale)`
pub fn normalize_rating_matrix(
    matrix: &RatingMatrix,
    config: &PorConfig,
) -> Result<NormalizedRatingMatrix, PorError> {
    if config.scale == 0 {
        return Err(PorError::InvalidNormalizationScale);
    }

    let mut normalized = Vec::with_capacity(matrix.ratings.len());

    for group in matrix
        .ratings
        .chunk_by(|left, right| left.recipient == right.recipient)
    {
        normalize_recipient_group(group, config.scale, &mut normalized)?;
    }

    Ok(NormalizedRatingMatrix {
        round: matrix.round,
        ratings: normalized,
    })
}

fn normalize_recipient_group(
    group: &[RatingRecord],
    scale: u64,
    output: &mut Vec<NormalizedRatingEntry>,
) -> Result<(), PorError> {
    let Some(first) = group.first() else {
        return Ok(());
    };

    let (min, max) = group
        .iter()
        .skip(1)
        .fold((first.score, first.score), |(min, max), rating| {
            (min.min(rating.score), max.max(rating.score))
        });
    let range = max - min;
    let scale_u128 = u128::from(scale);
    let denominator = u128::from(range) + scale_u128;

    if denominator > u128::MAX / scale_u128 {
        return Err(PorError::NormalizationOverflow);
    }

    for rating in group {
        let score_delta = rating.score - min;
        let normalized_score = normalize_score(score_delta, scale_u128, denominator);

        output.push(NormalizedRatingEntry {
            rater: rating.rater.clone(),
            recipient: rating.recipient.clone(),
            score: rating.score,
            normalized_score,
        });
    }

    Ok(())
}

fn normalize_score(score_delta: u64, scale: u128, denominator: u128) -> u64 {
    let numerator_base = u128::from(score_delta) + scale;
    let numerator = numerator_base * scale;
    let normalized = numerator / denominator;

    debug_assert!(normalized <= scale);
    normalized as u64
}
