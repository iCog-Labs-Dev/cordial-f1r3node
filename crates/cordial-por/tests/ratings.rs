use cordial_miners_core::NodeId;
use cordial_por::{PorConfig, PorError, RatingRecord, build_rating_batch, validate_rating};

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
fn accepts_a_valid_signed_rating() {
    let record = rating(7, 1, 2, 42, vec![1, 2, 3]);

    assert!(validate_rating(&record, &cfg()).is_ok());
}

#[test]
fn rejects_self_rating() {
    let record = rating(7, 1, 1, 42, vec![1, 2, 3]);

    assert!(matches!(
        validate_rating(&record, &cfg()),
        Err(PorError::SelfRating)
    ));
}

#[test]
fn rejects_empty_signature() {
    let record = rating(7, 1, 2, 42, Vec::new());

    assert!(matches!(
        validate_rating(&record, &cfg()),
        Err(PorError::MissingRatingSignature)
    ));
}

#[test]
fn rejects_score_below_minimum() {
    let mut config = cfg();
    config.minimum_rating = 10;
    let record = rating(7, 1, 2, 9, vec![1, 2, 3]);

    assert!(matches!(
        validate_rating(&record, &config),
        Err(PorError::RatingBelowMinimum)
    ));
}

#[test]
fn rejects_score_above_maximum() {
    let mut config = cfg();
    config.maximum_rating = 20;
    let record = rating(7, 1, 2, 21, vec![1, 2, 3]);

    assert!(matches!(
        validate_rating(&record, &config),
        Err(PorError::RatingAboveMaximum)
    ));
}

#[test]
fn rejects_rating_whose_round_does_not_match_batch_round() {
    let r1 = rating(9, 1, 2, 10, vec![1, 2, 3]);
    let result = build_rating_batch(10, vec![r1], &cfg());

    assert!(matches!(result, Err(PorError::InvalidRatingRound)));
}

#[test]
fn rejects_duplicate_round_rater_recipient_ratings() {
    let r1 = rating(7, 1, 2, 10, vec![1, 2, 3]);
    let r2 = rating(7, 1, 2, 11, vec![4, 5, 6]);

    let result = build_rating_batch(7, vec![r1, r2], &cfg());

    assert!(matches!(result, Err(PorError::DuplicateRating)));
}

#[test]
fn rejects_duplicate_ratings_after_sorting() {
    let r1 = rating(7, 1, 2, 10, vec![1, 2, 3]);
    let r2 = rating(7, 3, 9, 12, vec![4, 5, 6]);
    let r3 = rating(7, 1, 2, 14, vec![7, 8, 9]);

    let result = build_rating_batch(7, vec![r1, r2, r3], &cfg());

    assert!(matches!(result, Err(PorError::DuplicateRating)));
}

#[test]
fn returns_ratings_in_deterministic_order_even_when_input_is_shuffled() {
    let r1 = rating(5, 2, 3, 10, vec![1, 2, 3]);
    let r2 = rating(5, 1, 4, 12, vec![4, 5, 6]);
    let r3 = rating(5, 1, 2, 14, vec![7, 8, 9]);
    let batch = build_rating_batch(5, vec![r1.clone(), r2.clone(), r3.clone()], &cfg()).unwrap();

    let expected = vec![r3, r1, r2];
    assert_eq!(batch.ratings, expected);
}
