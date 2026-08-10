use cordial_miners_core::NodeId;
use cordial_por::{
    NormalizedRatingEntry, NormalizedRatingMatrix, PorConfig, PorError, ReputationEntry,
    ReputationVector, compute_liquid_rank_contribution,
};

fn cfg(scale: u64) -> PorConfig {
    PorConfig {
        scale,
        initial_reputation: 0,
        liquid_rank_alpha: 0,
        minimum_rating: 0,
        maximum_rating: scale,
    }
}

fn entry(node: u8, reputation: u64) -> ReputationEntry {
    ReputationEntry::new(NodeId(vec![node]), reputation)
}

fn normalized_rating(
    rater: u8,
    recipient: u8,
    score: u64,
    normalized_score: u64,
) -> NormalizedRatingEntry {
    NormalizedRatingEntry {
        rater: NodeId(vec![rater]),
        recipient: NodeId(vec![recipient]),
        score,
        normalized_score,
    }
}

#[test]
fn computes_single_recipient_contribution_from_rater_reputation() {
    let matrix = NormalizedRatingMatrix {
        round: 7,
        ratings: vec![normalized_rating(1, 2, 50, 50)],
    };
    let previous = ReputationVector {
        round: 6,
        values: vec![entry(1, 80)],
    };

    let contribution = compute_liquid_rank_contribution(&matrix, &previous, &cfg(100)).unwrap();

    assert_eq!(contribution.round, 7);
    assert_eq!(contribution.values, vec![entry(2, 40)]);
}

#[test]
fn sums_multiple_rater_contributions_for_the_same_recipient() {
    let matrix = NormalizedRatingMatrix {
        round: 8,
        ratings: vec![
            normalized_rating(1, 9, 40, 50),
            normalized_rating(2, 9, 70, 100),
        ],
    };
    let previous = ReputationVector {
        round: 7,
        values: vec![entry(1, 80), entry(2, 20)],
    };

    let contribution = compute_liquid_rank_contribution(&matrix, &previous, &cfg(100)).unwrap();

    assert_eq!(contribution.values, vec![entry(9, 60)]);
}

#[test]
fn preserves_canonical_recipient_order_from_the_normalized_matrix() {
    let matrix = NormalizedRatingMatrix {
        round: 9,
        ratings: vec![
            normalized_rating(1, 2, 30, 50),
            normalized_rating(3, 2, 80, 100),
            normalized_rating(1, 9, 60, 75),
        ],
    };
    let previous = ReputationVector {
        round: 8,
        values: vec![entry(1, 80), entry(3, 40)],
    };

    let contribution = compute_liquid_rank_contribution(&matrix, &previous, &cfg(100)).unwrap();

    assert_eq!(contribution.values, vec![entry(2, 80), entry(9, 60)]);
}

#[test]
fn uses_rater_reputation_not_recipient_reputation() {
    let matrix = NormalizedRatingMatrix {
        round: 10,
        ratings: vec![normalized_rating(1, 2, 70, 100)],
    };
    let previous = ReputationVector {
        round: 9,
        values: vec![entry(1, 20), entry(2, 1_000)],
    };

    let contribution = compute_liquid_rank_contribution(&matrix, &previous, &cfg(100)).unwrap();

    assert_eq!(contribution.values, vec![entry(2, 20)]);
}

#[test]
fn accumulates_before_dividing_to_preserve_fixed_point_precision() {
    let matrix = NormalizedRatingMatrix {
        round: 12,
        ratings: vec![
            normalized_rating(1, 2, 50, 50),
            normalized_rating(3, 2, 50, 50),
        ],
    };
    let previous = ReputationVector {
        round: 11,
        values: vec![entry(1, 1), entry(3, 1)],
    };

    let contribution = compute_liquid_rank_contribution(&matrix, &previous, &cfg(100)).unwrap();

    assert_eq!(contribution.values, vec![entry(2, 1)]);
}

#[test]
fn empty_matrix_returns_empty_contribution_vector_and_preserves_round() {
    let matrix = NormalizedRatingMatrix {
        round: 11,
        ratings: Vec::new(),
    };
    let previous = ReputationVector {
        round: 10,
        values: Vec::new(),
    };

    let contribution = compute_liquid_rank_contribution(&matrix, &previous, &cfg(100)).unwrap();

    assert_eq!(contribution.round, 11);
    assert!(contribution.values.is_empty());
}

#[test]
fn unsorted_previous_reputation_entries_are_rejected() {
    let matrix = NormalizedRatingMatrix {
        round: 13,
        ratings: vec![normalized_rating(1, 2, 40, 50)],
    };
    let previous = ReputationVector {
        round: 12,
        values: vec![entry(3, 80), entry(1, 70)],
    };

    assert!(matches!(
        compute_liquid_rank_contribution(&matrix, &previous, &cfg(100)),
        Err(PorError::UnsortedReputationVector)
    ));
}

#[test]
fn missing_rater_reputation_is_rejected() {
    let matrix = NormalizedRatingMatrix {
        round: 12,
        ratings: vec![normalized_rating(1, 2, 40, 50)],
    };
    let previous = ReputationVector {
        round: 11,
        values: vec![entry(3, 80)],
    };

    assert!(matches!(
        compute_liquid_rank_contribution(&matrix, &previous, &cfg(100)),
        Err(PorError::MissingRaterReputation)
    ));
}

#[test]
fn duplicate_previous_reputation_entries_are_rejected() {
    let matrix = NormalizedRatingMatrix {
        round: 13,
        ratings: vec![normalized_rating(1, 2, 40, 50)],
    };
    let previous = ReputationVector {
        round: 12,
        values: vec![entry(1, 80), entry(1, 70)],
    };

    assert!(matches!(
        compute_liquid_rank_contribution(&matrix, &previous, &cfg(100)),
        Err(PorError::DuplicateReputationEntry)
    ));
}

#[test]
fn zero_scale_is_rejected() {
    let matrix = NormalizedRatingMatrix {
        round: 14,
        ratings: vec![normalized_rating(1, 2, 0, 0)],
    };
    let previous = ReputationVector {
        round: 13,
        values: vec![entry(1, 80)],
    };

    assert!(matches!(
        compute_liquid_rank_contribution(&matrix, &previous, &cfg(0)),
        Err(PorError::InvalidLiquidRankScale)
    ));
}

#[test]
fn contribution_overflow_is_reported() {
    let matrix = NormalizedRatingMatrix {
        round: 15,
        ratings: vec![normalized_rating(1, 2, u64::MAX, u64::MAX)],
    };
    let previous = ReputationVector {
        round: 14,
        values: vec![entry(1, u64::MAX)],
    };

    assert!(matches!(
        compute_liquid_rank_contribution(&matrix, &previous, &cfg(1)),
        Err(PorError::LiquidRankOverflow)
    ));
}
