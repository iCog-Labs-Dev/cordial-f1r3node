use cordial_miners_core::NodeId;
use cordial_por::{
    PorConfig, PorError, RatingMatrix, RatingRecord, build_rating_batch, build_rating_matrix,
    normalize_rating_matrix,
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

fn rating(round: u64, rater: u8, recipient: u8, score: u64) -> RatingRecord {
    RatingRecord::new(
        round,
        NodeId(vec![rater]),
        NodeId(vec![recipient]),
        score,
        vec![rater, recipient],
    )
}

#[test]
fn normalizes_matrix_values_by_recipient_using_modified_paper_formula() {
    let config = cfg(100);
    let batch = build_rating_batch(
        7,
        vec![
            rating(7, 3, 2, 100),
            rating(7, 1, 2, 0),
            rating(7, 2, 9, 60),
            rating(7, 1, 9, 40),
        ],
        &config,
    )
    .unwrap();
    let matrix = build_rating_matrix(&batch).unwrap();

    let normalized = normalize_rating_matrix(&matrix, &config).unwrap();

    assert_eq!(normalized.round, 7);
    assert_eq!(normalized.ratings.len(), 4);

    assert_eq!(normalized.ratings[0].recipient, NodeId(vec![2]));
    assert_eq!(normalized.ratings[0].rater, NodeId(vec![1]));
    assert_eq!(normalized.ratings[0].score, 0);
    assert_eq!(normalized.ratings[0].normalized_score, 50);

    assert_eq!(normalized.ratings[1].recipient, NodeId(vec![2]));
    assert_eq!(normalized.ratings[1].rater, NodeId(vec![3]));
    assert_eq!(normalized.ratings[1].score, 100);
    assert_eq!(normalized.ratings[1].normalized_score, 100);

    assert_eq!(normalized.ratings[2].recipient, NodeId(vec![9]));
    assert_eq!(normalized.ratings[2].rater, NodeId(vec![1]));
    assert_eq!(normalized.ratings[2].score, 40);
    assert_eq!(normalized.ratings[2].normalized_score, 83);

    assert_eq!(normalized.ratings[3].recipient, NodeId(vec![9]));
    assert_eq!(normalized.ratings[3].rater, NodeId(vec![2]));
    assert_eq!(normalized.ratings[3].score, 60);
    assert_eq!(normalized.ratings[3].normalized_score, 100);
}

#[test]
fn normalization_uses_recipient_groups_not_global_min_and_max() {
    let config = cfg(100);
    let matrix = RatingMatrix {
        round: 3,
        ratings: vec![
            rating(3, 1, 8, 40),
            rating(3, 2, 8, 60),
            rating(3, 1, 9, 0),
            rating(3, 2, 9, 100),
        ],
    };

    let normalized = normalize_rating_matrix(&matrix, &config).unwrap();

    assert_eq!(normalized.ratings[0].recipient, NodeId(vec![8]));
    assert_eq!(normalized.ratings[0].normalized_score, 83);
    assert_eq!(normalized.ratings[1].recipient, NodeId(vec![8]));
    assert_eq!(normalized.ratings[1].normalized_score, 100);
    assert_eq!(normalized.ratings[2].recipient, NodeId(vec![9]));
    assert_eq!(normalized.ratings[2].normalized_score, 50);
    assert_eq!(normalized.ratings[3].recipient, NodeId(vec![9]));
    assert_eq!(normalized.ratings[3].normalized_score, 100);
}

#[test]
fn normalization_preserves_canonical_matrix_order() {
    let config = cfg(100);
    let batch = build_rating_batch(
        4,
        vec![
            rating(4, 9, 8, 80),
            rating(4, 4, 2, 30),
            rating(4, 1, 2, 10),
        ],
        &config,
    )
    .unwrap();
    let matrix = build_rating_matrix(&batch).unwrap();

    let normalized = normalize_rating_matrix(&matrix, &config).unwrap();
    let order: Vec<_> = normalized
        .ratings
        .iter()
        .map(|rating| (rating.recipient.clone(), rating.rater.clone()))
        .collect();

    assert_eq!(
        order,
        vec![
            (NodeId(vec![2]), NodeId(vec![1])),
            (NodeId(vec![2]), NodeId(vec![4])),
            (NodeId(vec![8]), NodeId(vec![9])),
        ]
    );
}

#[test]
fn single_rating_recipient_group_normalizes_to_scale() {
    let config = cfg(100);
    let matrix = RatingMatrix {
        round: 5,
        ratings: vec![rating(5, 1, 2, 75)],
    };

    let normalized = normalize_rating_matrix(&matrix, &config).unwrap();

    assert_eq!(normalized.ratings[0].score, 75);
    assert_eq!(normalized.ratings[0].normalized_score, 100);
}

#[test]
fn empty_matrix_stays_empty_and_preserves_round() {
    let config = cfg(100);
    let matrix = RatingMatrix {
        round: 6,
        ratings: Vec::new(),
    };

    let normalized = normalize_rating_matrix(&matrix, &config).unwrap();

    assert_eq!(normalized.round, 6);
    assert!(normalized.ratings.is_empty());
}

#[test]
fn zero_scale_is_rejected() {
    let config = cfg(0);
    let matrix = RatingMatrix {
        round: 7,
        ratings: vec![rating(7, 1, 2, 0)],
    };

    assert!(matches!(
        normalize_rating_matrix(&matrix, &config),
        Err(PorError::InvalidNormalizationScale)
    ));
}

#[test]
fn normalization_overflow_is_reported() {
    let config = cfg(u64::MAX);
    let matrix = RatingMatrix {
        round: 8,
        ratings: vec![rating(8, 1, 2, 0), rating(8, 3, 2, u64::MAX)],
    };

    assert!(matches!(
        normalize_rating_matrix(&matrix, &config),
        Err(PorError::NormalizationOverflow)
    ));
}
