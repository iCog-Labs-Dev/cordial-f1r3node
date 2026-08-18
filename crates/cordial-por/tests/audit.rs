use cordial_miners_core::NodeId;
use cordial_por::{
    PorConfig, PorError, RatingRecord, ReputationBlock, ReputationBlockHeader, ReputationEntry,
    ReputationList, ReputationVector, replay_reputation_transition, verify_reputation_transition,
};

const ROUND: u64 = 7;

/// Small fixed-point scale so the expected values below stay hand-checkable.
fn config() -> PorConfig {
    PorConfig {
        scale: 100,
        initial_reputation: 0,
        liquid_rank_alpha: 50,
        minimum_rating: 0,
        maximum_rating: 100,
    }
}

fn node(id: u8) -> NodeId {
    NodeId(vec![id])
}

fn entry(id: u8, reputation: u64) -> ReputationEntry {
    ReputationEntry::new(node(id), reputation)
}

fn rating(rater: u8, recipient: u8, score: u64) -> RatingRecord {
    RatingRecord::new(ROUND, node(rater), node(recipient), score, vec![0x01])
}

fn previous_reputation() -> ReputationVector {
    ReputationVector {
        round: ROUND - 1,
        values: vec![entry(1, 50), entry(2, 100), entry(3, 20)],
    }
}

fn ratings() -> Vec<RatingRecord> {
    vec![
        rating(2, 1, 80),
        rating(3, 1, 40),
        rating(1, 2, 60),
        rating(3, 2, 100),
        rating(1, 3, 0),
        rating(2, 3, 50),
    ]
}

/// The same ratings in a different arrival order.
fn shuffled_ratings() -> Vec<RatingRecord> {
    vec![
        rating(2, 3, 50),
        rating(1, 2, 60),
        rating(3, 1, 40),
        rating(2, 1, 80),
        rating(1, 3, 0),
        rating(3, 2, 100),
    ]
}

/// Hand-computed result of the pipeline for the fixture above:
///
/// normalized S' -> Liquid-Rank P = [114, 55, 133] -> alpha blend = [82, 77, 76]
/// -> sigmoid clamp = [64, 61, 61].
fn expected_entries() -> Vec<ReputationEntry> {
    vec![entry(1, 64), entry(2, 61), entry(3, 61)]
}

fn block(round: u64, entries: Vec<ReputationEntry>) -> ReputationBlock {
    ReputationBlock {
        header: ReputationBlockHeader {
            round,
            previous_reputation_hash: Some(vec![0x01]),
            ratings_hash: vec![0x02],
            reputation_root: vec![0x03],
        },
        reputation_list: ReputationList { round, entries },
    }
}

fn proposed_block() -> ReputationBlock {
    block(ROUND, expected_entries())
}

fn verify(block: &ReputationBlock) -> Result<(), PorError> {
    verify_reputation_transition(&previous_reputation(), &ratings(), block, &config())
}

#[test]
fn replays_the_expected_reputation_list() {
    let replayed =
        replay_reputation_transition(&previous_reputation(), &ratings(), ROUND, &config()).unwrap();

    assert_eq!(replayed.round, ROUND);
    assert_eq!(replayed.entries, expected_entries());
}

#[test]
fn accepts_a_block_matching_the_replay() {
    assert_eq!(verify(&proposed_block()), Ok(()));
}

#[test]
fn rejects_mismatched_reputation_value() {
    let block = block(ROUND, vec![entry(1, 64), entry(2, 62), entry(3, 61)]);

    assert_eq!(verify(&block), Err(PorError::ReputationValueMismatch));
}

#[test]
fn rejects_missing_reputation_entry() {
    let block = block(ROUND, vec![entry(1, 64), entry(3, 61)]);

    assert_eq!(verify(&block), Err(PorError::MissingReputationBlockEntry));
}

#[test]
fn rejects_extra_reputation_entry() {
    let mut entries = expected_entries();
    entries.push(entry(4, 10));

    assert_eq!(
        verify(&block(ROUND, entries)),
        Err(PorError::UnexpectedReputationBlockEntry)
    );
}

#[test]
fn rejects_header_round_that_differs_from_the_list_round() {
    let mut block = proposed_block();
    block.header.round = ROUND + 1;

    assert_eq!(verify(&block), Err(PorError::InvalidReputationBlockRound));
}

#[test]
fn rejects_a_round_that_does_not_follow_the_previous_reputation() {
    let stale_previous = ReputationVector {
        round: ROUND - 3,
        ..previous_reputation()
    };

    assert_eq!(
        verify_reputation_transition(&stale_previous, &ratings(), &proposed_block(), &config()),
        Err(PorError::InvalidTransitionRound)
    );
}

#[test]
fn rejects_ratings_from_another_round() {
    let mut ratings = ratings();
    ratings[0].round = ROUND + 1;

    assert_eq!(
        verify_reputation_transition(
            &previous_reputation(),
            &ratings,
            &proposed_block(),
            &config()
        ),
        Err(PorError::InvalidRatingRound)
    );
}

#[test]
fn rejects_invalid_rating_input() {
    let mut ratings = ratings();
    ratings[0].rater = ratings[0].recipient.clone();

    assert_eq!(
        verify_reputation_transition(
            &previous_reputation(),
            &ratings,
            &proposed_block(),
            &config()
        ),
        Err(PorError::SelfRating)
    );
}

#[test]
fn rejects_block_with_empty_ratings_hash() {
    let mut block = proposed_block();
    block.header.ratings_hash.clear();

    assert_eq!(
        verify(&block),
        Err(PorError::MissingReputationBlockRatingsHash)
    );
}

#[test]
fn rejects_block_with_empty_reputation_root() {
    let mut block = proposed_block();
    block.header.reputation_root.clear();

    assert_eq!(verify(&block), Err(PorError::MissingReputationBlockRoot));
}

#[test]
fn rejects_unsorted_reputation_list() {
    let block = block(ROUND, vec![entry(2, 61), entry(1, 64), entry(3, 61)]);

    assert_eq!(verify(&block), Err(PorError::UnsortedReputationVector));
}

#[test]
fn rejects_duplicate_reputation_entries() {
    let block = block(ROUND, vec![entry(1, 64), entry(1, 64), entry(3, 61)]);

    assert_eq!(verify(&block), Err(PorError::DuplicateReputationEntry));
}

#[test]
fn replays_deterministically_from_shuffled_ratings() {
    let replayed = replay_reputation_transition(
        &previous_reputation(),
        &shuffled_ratings(),
        ROUND,
        &config(),
    )
    .unwrap();

    assert_eq!(replayed.entries, expected_entries());
    assert_eq!(
        verify_reputation_transition(
            &previous_reputation(),
            &shuffled_ratings(),
            &proposed_block(),
            &config()
        ),
        Ok(())
    );
}
