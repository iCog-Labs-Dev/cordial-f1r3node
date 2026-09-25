use cordial_miners_core::NodeId;
use cordial_por::{
    MissingEntryPolicy, PorConfig, PorError, RatingRecord, ReputationBlock, ReputationBlockContext,
    ReputationEntry, ReputationList, ReputationVector, build_rating_batch, build_reputation_block,
    clamp_reputation_value, replay_reputation_transition, reputation_list_commitment,
    verify_reputation_transition,
};

const ROUND: u64 = 7;
const SOURCE_WAVE: u64 = ROUND - 1;
const SHARD_ID: &[u8] = b"root";

/// Small fixed-point scale so the expected values below stay hand-checkable.
fn config() -> PorConfig {
    PorConfig {
        scale: 100,
        initial_reputation: 0,
        liquid_rank_alpha: 50,
        minimum_rating: 0,
        maximum_rating: 100,
        missing_entry_policy: MissingEntryPolicy::default(),
    }
}

fn config_with_initial_reputation(initial_reputation: u64) -> PorConfig {
    PorConfig {
        initial_reputation,
        ..config()
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

fn context() -> ReputationBlockContext<'static> {
    ReputationBlockContext {
        shard_id: SHARD_ID,
        source_finalized_wave: SOURCE_WAVE,
        previous_block: None,
    }
}

fn block_for(
    ratings: &[RatingRecord],
    entries: Vec<ReputationEntry>,
    config: &PorConfig,
) -> ReputationBlock {
    let batch = build_rating_batch(ROUND, ratings.to_vec(), config).unwrap();
    build_reputation_block(
        context(),
        &batch,
        ReputationList {
            round: ROUND,
            entries,
        },
        config,
    )
    .unwrap()
}

fn block(round: u64, entries: Vec<ReputationEntry>) -> ReputationBlock {
    assert_eq!(round, ROUND);
    block_for(&ratings(), entries, &config())
}

fn proposed_block() -> ReputationBlock {
    block(ROUND, expected_entries())
}

fn verify(block: &ReputationBlock) -> Result<(), PorError> {
    verify_reputation_transition(
        &previous_reputation(),
        &ratings(),
        block,
        context(),
        &config(),
    )
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
fn rejects_list_round_that_differs_from_the_header_round() {
    let mut block = proposed_block();
    block.reputation_list.round = ROUND + 1;

    assert_eq!(verify(&block), Err(PorError::InvalidReputationBlockRound));
}

#[test]
fn rejects_a_round_that_does_not_follow_the_previous_reputation() {
    let stale_previous = ReputationVector {
        round: ROUND - 3,
        ..previous_reputation()
    };

    assert_eq!(
        verify_reputation_transition(
            &stale_previous,
            &ratings(),
            &proposed_block(),
            context(),
            &config()
        ),
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
            context(),
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
            context(),
            &config()
        ),
        Err(PorError::SelfRating)
    );
}

#[test]
fn rejects_a_tampered_rating_batch_commitment() {
    let mut block = proposed_block();
    block.header.ratings_hash[0] ^= 1;

    assert_eq!(
        verify(&block),
        Err(PorError::ReputationBlockRatingsHashMismatch)
    );
}

#[test]
fn rejects_a_tampered_reputation_root() {
    let mut block = proposed_block();
    block.header.reputation_root[0] ^= 1;

    assert_eq!(verify(&block), Err(PorError::ReputationBlockRootMismatch));
}

#[test]
fn rejects_tampered_context_and_configuration_commitments() {
    let block = proposed_block();
    let wrong_context = ReputationBlockContext {
        shard_id: b"other-shard",
        ..context()
    };
    assert_eq!(
        verify_reputation_transition(
            &previous_reputation(),
            &ratings(),
            &block,
            wrong_context,
            &config(),
        ),
        Err(PorError::ReputationBlockShardMismatch)
    );

    let wrong_wave_context = ReputationBlockContext {
        source_finalized_wave: SOURCE_WAVE + 1,
        ..context()
    };
    assert_eq!(
        verify_reputation_transition(
            &previous_reputation(),
            &ratings(),
            &block,
            wrong_wave_context,
            &config(),
        ),
        Err(PorError::ReputationBlockSourceWaveMismatch)
    );

    let changed_config = PorConfig {
        liquid_rank_alpha: 40,
        ..config()
    };
    assert_eq!(
        verify_reputation_transition(
            &previous_reputation(),
            &ratings(),
            &block,
            context(),
            &changed_config,
        ),
        Err(PorError::ReputationBlockConfigHashMismatch)
    );
}

#[test]
fn rejects_a_tampered_previous_block_commitment() {
    let config = config();
    let previous_block = build_reputation_block(
        ReputationBlockContext {
            shard_id: SHARD_ID,
            source_finalized_wave: SOURCE_WAVE - 1,
            previous_block: None,
        },
        &cordial_por::RatingBatch {
            round: ROUND - 1,
            ratings: Vec::new(),
        },
        ReputationList {
            round: ROUND - 1,
            entries: previous_reputation().values,
        },
        &config,
    )
    .unwrap();
    let batch = build_rating_batch(ROUND, ratings(), &config).unwrap();
    let mut block = build_reputation_block(
        ReputationBlockContext {
            shard_id: SHARD_ID,
            source_finalized_wave: SOURCE_WAVE,
            previous_block: Some(&previous_block),
        },
        &batch,
        ReputationList {
            round: ROUND,
            entries: expected_entries(),
        },
        &config,
    )
    .unwrap();
    block.header.previous_reputation_hash.as_mut().unwrap()[0] ^= 1;

    assert_eq!(
        verify_reputation_transition(
            &previous_reputation(),
            &ratings(),
            &block,
            ReputationBlockContext {
                shard_id: SHARD_ID,
                source_finalized_wave: SOURCE_WAVE,
                previous_block: Some(&previous_block),
            },
            &config,
        ),
        Err(PorError::ReputationBlockPreviousHashMismatch)
    );
}

#[test]
fn rejects_unsorted_reputation_list() {
    let mut block = proposed_block();
    block.reputation_list.entries.swap(0, 1);

    assert_eq!(verify(&block), Err(PorError::UnsortedReputationVector));
}

#[test]
fn rejects_duplicate_reputation_entries() {
    let mut block = proposed_block();
    block.reputation_list.entries[1].node_id = node(1);

    assert_eq!(verify(&block), Err(PorError::DuplicateReputationEntry));
}

#[test]
fn rejects_a_self_consistent_but_incorrect_exclusion_flag() {
    let mut block = proposed_block();
    block.reputation_list.entries[0].is_excluded = true;
    block.header.reputation_root = reputation_list_commitment(&block.reputation_list).unwrap();

    assert_eq!(verify(&block), Err(PorError::ReputationExclusionMismatch));
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
            context(),
            &config()
        ),
        Ok(())
    );
}

/// Round 7 with no ratings at all for node 3.
fn sparse_ratings() -> Vec<RatingRecord> {
    vec![
        rating(2, 1, 80),
        rating(3, 1, 40),
        rating(1, 2, 60),
        rating(3, 2, 100),
    ]
}

#[test]
fn replays_a_sparse_round_and_carries_the_unrated_node_forward() {
    let replayed =
        replay_reputation_transition(&previous_reputation(), &sparse_ratings(), ROUND, &config())
            .unwrap();

    // Nodes 1 and 2 move as in the fully rated round; node 3 received no
    // ratings, so CarryForward copies its previous finalized reputation.
    assert_eq!(
        replayed.entries,
        vec![entry(1, 64), entry(2, 61), entry(3, 20)]
    );
}

#[test]
fn verifies_a_block_built_from_a_sparse_round() {
    let sparse_ratings = sparse_ratings();
    let block = block_for(
        &sparse_ratings,
        vec![entry(1, 64), entry(2, 61), entry(3, 20)],
        &config(),
    );

    assert_eq!(
        verify_reputation_transition(
            &previous_reputation(),
            &sparse_ratings,
            &block,
            context(),
            &config()
        ),
        Ok(())
    );
}

#[test]
fn sparse_round_replay_is_order_independent() {
    let mut shuffled = sparse_ratings();
    shuffled.reverse();

    assert_eq!(
        replay_reputation_transition(&previous_reputation(), &shuffled, ROUND, &config()).unwrap(),
        replay_reputation_transition(&previous_reputation(), &sparse_ratings(), ROUND, &config())
            .unwrap()
    );
}

/// Round 7 where node 4 is rated by nodes 1 and 2 but rates nobody itself, so
/// it reaches the transition with a contribution and no previous reputation.
fn ratings_with_new_node() -> Vec<RatingRecord> {
    let mut ratings = ratings();
    ratings.push(rating(1, 4, 100));
    ratings.push(rating(2, 4, 60));
    ratings
}

#[test]
fn seeds_a_new_node_from_initial_reputation() {
    let config = config_with_initial_reputation(20);

    let replayed = replay_reputation_transition(
        &previous_reputation(),
        &ratings_with_new_node(),
        ROUND,
        &config,
    )
    .unwrap();

    assert_eq!(
        replayed.entries,
        vec![entry(1, 64), entry(2, 61), entry(3, 61), entry(4, 57)]
    );
}

#[test]
fn a_new_node_that_also_rates_is_still_rejected_by_liquid_rank() {
    let mut ratings = ratings_with_new_node();
    ratings.push(rating(4, 1, 50));

    assert_eq!(
        replay_reputation_transition(
            &previous_reputation(),
            &ratings,
            ROUND,
            &config_with_initial_reputation(20)
        ),
        Err(PorError::MissingRaterReputation)
    );
}

/// Production defaults: scale 1_000_000_000, initial 200_000_000. Node 1 rates
/// node 2; nobody rates node 1.
fn production_sparse_ratings() -> Vec<RatingRecord> {
    vec![RatingRecord::new(
        ROUND,
        node(1),
        node(2),
        PorConfig::DEFAULT_SCALE,
        vec![0x01],
    )]
}

fn production_previous() -> ReputationVector {
    ReputationVector {
        round: ROUND - 1,
        values: vec![
            entry(1, PorConfig::DEFAULT_INITIAL_REPUTATION),
            entry(2, PorConfig::DEFAULT_INITIAL_REPUTATION),
        ],
    }
}

#[test]
fn carry_forward_does_not_decay_under_clamp_at_production_scale() {
    let config = PorConfig::default();
    let replayed = replay_reputation_transition(
        &production_previous(),
        &production_sparse_ratings(),
        ROUND,
        &config,
    )
    .unwrap();

    // Naive clamp of the carried-forward 200_000_000 would yield 196_116_135.
    assert_eq!(
        clamp_reputation_value(
            PorConfig::DEFAULT_INITIAL_REPUTATION,
            PorConfig::DEFAULT_SCALE
        )
        .unwrap(),
        196_116_135
    );
    assert_eq!(
        replayed.entries[0],
        entry(1, PorConfig::DEFAULT_INITIAL_REPUTATION)
    );
    assert_ne!(
        replayed.entries[1].reputation,
        PorConfig::DEFAULT_INITIAL_REPUTATION
    );
}

#[test]
fn neutral_still_clamps_an_unrated_node_at_production_scale() {
    let config = PorConfig {
        missing_entry_policy: MissingEntryPolicy::Neutral,
        ..PorConfig::default()
    };
    let replayed = replay_reputation_transition(
        &production_previous(),
        &production_sparse_ratings(),
        ROUND,
        &config,
    )
    .unwrap();

    assert_ne!(
        replayed.entries[0].reputation,
        PorConfig::DEFAULT_INITIAL_REPUTATION
    );
}
