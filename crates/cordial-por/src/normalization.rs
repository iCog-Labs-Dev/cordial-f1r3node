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

    let ordered = &matrix.ratings;
    let mut normalized = Vec::with_capacity(ordered.len());
    let mut group_start = 0;

    while group_start < ordered.len() {
        let recipient = &ordered[group_start].recipient;
        let mut group_end = group_start + 1;

        while group_end < ordered.len() && ordered[group_end].recipient == *recipient {
            group_end += 1;
        }

        normalize_recipient_group(
            &ordered[group_start..group_end],
            config.scale,
            &mut normalized,
        )?;

        group_start = group_end;
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
    let min = group
        .iter()
        .map(|rating| rating.score)
        .min()
        .ok_or(PorError::InvalidNormalizationScale)?;
    let max = group
        .iter()
        .map(|rating| rating.score)
        .max()
        .ok_or(PorError::InvalidNormalizationScale)?;
    let range = max - min;

    for rating in group {
        let score_delta = rating.score - min;
        let normalized_score = normalize_score(score_delta, range, scale)?;

        output.push(NormalizedRatingEntry {
            rater: rating.rater.clone(),
            recipient: rating.recipient.clone(),
            score: rating.score,
            normalized_score,
        });
    }

    Ok(())
}

fn normalize_score(score_delta: u64, range: u64, scale: u64) -> Result<u64, PorError> {
    let numerator_base = u128::from(score_delta) + u128::from(scale);
    let denominator = u128::from(range) + u128::from(scale);
    let numerator = numerator_base
        .checked_mul(u128::from(scale))
        .ok_or(PorError::NormalizationOverflow)?;
    let normalized = numerator / denominator;

    u64::try_from(normalized).map_err(|_| PorError::NormalizationOverflow)
}
