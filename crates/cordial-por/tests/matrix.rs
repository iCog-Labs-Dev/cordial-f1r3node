use cordial_miners_core::NodeId;
use cordial_por::{
    PorConfig, PorError, RatingBatch, RatingRecord, build_rating_batch, build_rating_matrix,
};

fn cfg() -> PorConfig {
    PorConfig::default()
}

fn rating(round: u64, rater: u8, recipient: u8, score: u64, signature: Vec<u8>) -> RatingRecord {
    RatingRecord::new(
        round,
        NodeId(vec![rater]),
        NodeId(vec![recipient]),
        score,
        signature,
    )
}

#[test]
fn builds_a_rating_matrix_from_a_valid_batch() {
    let batch = build_rating_batch(
        7,
        vec![
            rating(7, 2, 3, 40, vec![1, 2, 3]),
            rating(7, 1, 2, 50, vec![4, 5, 6]),
        ],
        &cfg(),
    )
    .unwrap();

    let matrix = build_rating_matrix(&batch).unwrap();

    assert_eq!(matrix.round, 7);
    assert_eq!(matrix.ratings.len(), 2);
}

#[test]
fn preserves_the_round_and_rating_fields() {
    let batch = build_rating_batch(
        9,
        vec![
            rating(9, 4, 1, 15, vec![9, 9, 9]),
            rating(9, 2, 5, 30, vec![8, 8, 8]),
        ],
        &cfg(),
    )
    .unwrap();

    let matrix = build_rating_matrix(&batch).unwrap();

    assert_eq!(matrix.round, 9);
    assert_eq!(matrix.ratings[0].rater, NodeId(vec![4]));
    assert_eq!(matrix.ratings[0].recipient, NodeId(vec![1]));
    assert_eq!(matrix.ratings[0].score, 15);
    assert_eq!(matrix.ratings[1].rater, NodeId(vec![2]));
    assert_eq!(matrix.ratings[1].recipient, NodeId(vec![5]));
    assert_eq!(matrix.ratings[1].score, 30);
}

#[test]
fn different_insertion_order_produces_identical_sorted_output() {
    let input = vec![
        rating(5, 2, 3, 40, vec![1]),
        rating(5, 1, 4, 80, vec![2]),
        rating(5, 1, 2, 60, vec![3]),
    ];

    let batch = build_rating_batch(5, input, &cfg()).unwrap();
    let matrix = build_rating_matrix(&batch).unwrap();

    let expected = vec![
        rating(5, 1, 2, 60, vec![3]),
        rating(5, 2, 3, 40, vec![1]),
        rating(5, 1, 4, 80, vec![2]),
    ];

    assert_eq!(matrix.ratings, expected);
}

#[test]
fn grouping_by_recipient_is_visible_in_sorted_order() {
    let batch = build_rating_batch(
        6,
        vec![
            rating(6, 3, 9, 20, vec![1]),
            rating(6, 1, 2, 70, vec![2]),
            rating(6, 4, 2, 90, vec![3]),
        ],
        &cfg(),
    )
    .unwrap();

    let matrix = build_rating_matrix(&batch).unwrap();
    let recipients: Vec<_> = matrix.ratings.iter().map(|r| r.recipient.clone()).collect();

    assert_eq!(
        recipients,
        vec![NodeId(vec![2]), NodeId(vec![2]), NodeId(vec![9])]
    );
}

#[test]
fn building_the_matrix_twice_from_the_same_batch_yields_identical_results() {
    let batch = build_rating_batch(
        8,
        vec![
            rating(8, 2, 1, 25, vec![1]),
            rating(8, 1, 4, 55, vec![2]),
            rating(8, 5, 3, 10, vec![3]),
        ],
        &cfg(),
    )
    .unwrap();

    let first = build_rating_matrix(&batch).unwrap();
    let second = build_rating_matrix(&batch).unwrap();

    assert_eq!(first, second);
}

#[test]
fn empty_batch_is_allowed_and_produces_a_correctly_rounded_empty_matrix() {
    let batch = RatingBatch {
        round: 11,
        ratings: vec![],
    };

    let matrix = build_rating_matrix(&batch).unwrap();

    assert_eq!(matrix.round, 11);
    assert!(matrix.ratings.is_empty());
}

#[test]
fn invariant_violation_duplicate_after_validation_is_caught_defensively() {
    let duplicate_batch = RatingBatch {
        round: 12,
        ratings: vec![rating(12, 1, 2, 50, vec![1]), rating(12, 1, 2, 60, vec![2])],
    };

    assert!(matches!(
        build_rating_matrix(&duplicate_batch),
        Err(PorError::DuplicateMatrixEntry)
    ));
}

#[test]
fn invariant_violation_round_mismatch_is_caught_defensively() {
    let mismatched_batch = RatingBatch {
        round: 14,
        ratings: vec![rating(15, 1, 2, 50, vec![1])],
    };

    assert!(matches!(
        build_rating_matrix(&mismatched_batch),
        Err(PorError::InvalidRatingRound)
    ));
}

#[test]
fn empty_signatures_are_not_confused_with_empty_batches() {
    let invalid_batch = build_rating_batch(13, vec![rating(13, 1, 2, 50, vec![])], &cfg());
    assert!(invalid_batch.is_err());

    let empty_batch = build_rating_batch(13, vec![], &cfg()).unwrap();
    assert!(empty_batch.ratings.is_empty());

    let matrix = build_rating_matrix(&empty_batch).unwrap();
    assert_eq!(matrix.round, 13);
    assert!(matrix.ratings.is_empty());
}
