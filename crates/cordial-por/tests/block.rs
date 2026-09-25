use cordial_miners_core::NodeId;
use cordial_por::{
    MAX_REPUTATION_BLOCK_SHARD_ID_LEN, PorConfig, PorError, REPUTATION_BLOCK_VERSION, RatingBatch,
    RatingRecord, ReputationBlockContext, ReputationEntry, ReputationList, build_rating_batch,
    build_reputation_block, config_commitment, rating_batch_commitment, reputation_block_hash,
    reputation_list_commitment, validate_reputation_block,
};

const SHARD_ID: &[u8] = b"root";
const SOURCE_WAVE: u64 = 6;
const ROUND: u64 = SOURCE_WAVE + 1;

fn entry(node: u8, reputation: u64) -> ReputationEntry {
    ReputationEntry::new(NodeId(vec![node]), reputation)
}

fn list(round: u64, entries: Vec<ReputationEntry>) -> ReputationList {
    ReputationList { round, entries }
}

fn rating(round: u64, rater: u8, recipient: u8, signature: u8) -> RatingRecord {
    RatingRecord::new(
        round,
        NodeId(vec![rater]),
        NodeId(vec![recipient]),
        80,
        vec![signature],
    )
}

fn batch(round: u64) -> RatingBatch {
    build_rating_batch(
        round,
        vec![rating(round, 2, 1, 0x02), rating(round, 1, 2, 0x01)],
        &PorConfig::default(),
    )
    .unwrap()
}

fn context<'a>(
    source_finalized_wave: u64,
    previous_block: Option<&'a cordial_por::ReputationBlock>,
) -> ReputationBlockContext<'a> {
    ReputationBlockContext {
        shard_id: SHARD_ID,
        source_finalized_wave,
        previous_block,
    }
}

#[test]
fn builds_all_reputation_block_commitments_from_protocol_inputs() {
    let config = PorConfig::default();
    let ratings = batch(ROUND);
    let reputation_list = list(ROUND, vec![entry(1, 90), entry(3, 30)]);

    let block = build_reputation_block(
        context(SOURCE_WAVE, None),
        &ratings,
        reputation_list.clone(),
        &config,
    )
    .unwrap();

    assert_eq!(block.header.version, REPUTATION_BLOCK_VERSION);
    assert_eq!(block.header.shard_id, SHARD_ID);
    assert_eq!(block.header.source_finalized_wave, SOURCE_WAVE);
    assert_eq!(block.header.round, ROUND);
    assert_eq!(block.header.previous_reputation_hash, None);
    assert_eq!(block.header.config_hash, config_commitment(&config));
    assert_eq!(
        block.header.ratings_hash,
        rating_batch_commitment(&ratings, &config).unwrap()
    );
    assert_eq!(
        block.header.reputation_root,
        reputation_list_commitment(&reputation_list).unwrap()
    );
    assert_eq!(block.reputation_list, reputation_list);
    assert_eq!(validate_reputation_block(&block), Ok(()));
    assert!(reputation_block_hash(&block).is_ok());
}

#[test]
fn allows_an_empty_reputation_list() {
    let block = build_reputation_block(
        context(SOURCE_WAVE, None),
        &RatingBatch {
            round: ROUND,
            ratings: Vec::new(),
        },
        list(ROUND, Vec::new()),
        &PorConfig::default(),
    )
    .unwrap();

    assert!(block.reputation_list.entries.is_empty());
    assert_eq!(validate_reputation_block(&block), Ok(()));
}

#[test]
fn chains_to_the_canonical_previous_block_hash() {
    let config = PorConfig::default();
    let previous = build_reputation_block(
        context(SOURCE_WAVE - 1, None),
        &batch(ROUND - 1),
        list(ROUND - 1, vec![entry(1, 80), entry(2, 70)]),
        &config,
    )
    .unwrap();
    let expected_previous_hash = reputation_block_hash(&previous).unwrap();

    let next = build_reputation_block(
        context(SOURCE_WAVE, Some(&previous)),
        &batch(ROUND),
        list(ROUND, vec![entry(1, 90), entry(2, 60)]),
        &config,
    )
    .unwrap();

    assert_eq!(
        next.header.previous_reputation_hash,
        Some(expected_previous_hash)
    );
}

#[test]
fn rejects_a_previous_block_from_another_shard_or_round() {
    let config = PorConfig::default();
    let other_shard_previous = build_reputation_block(
        ReputationBlockContext {
            shard_id: b"other-shard",
            source_finalized_wave: SOURCE_WAVE - 1,
            previous_block: None,
        },
        &batch(ROUND - 1),
        list(ROUND - 1, vec![entry(1, 80)]),
        &config,
    )
    .unwrap();

    assert_eq!(
        build_reputation_block(
            context(SOURCE_WAVE, Some(&other_shard_previous)),
            &batch(ROUND),
            list(ROUND, vec![entry(1, 90)]),
            &config,
        ),
        Err(PorError::PreviousReputationBlockShardMismatch)
    );

    let stale_previous = build_reputation_block(
        context(SOURCE_WAVE - 2, None),
        &batch(ROUND - 2),
        list(ROUND - 2, vec![entry(1, 70)]),
        &config,
    )
    .unwrap();

    assert_eq!(
        build_reputation_block(
            context(SOURCE_WAVE, Some(&stale_previous)),
            &batch(ROUND),
            list(ROUND, vec![entry(1, 90)]),
            &config,
        ),
        Err(PorError::InvalidPreviousReputationBlockRound)
    );
}

#[test]
fn rejects_rating_or_list_round_mismatch() {
    let config = PorConfig::default();

    assert_eq!(
        build_reputation_block(
            context(SOURCE_WAVE, None),
            &batch(ROUND - 1),
            list(ROUND, vec![entry(1, 90)]),
            &config,
        ),
        Err(PorError::InvalidRatingRound)
    );
    assert_eq!(
        build_reputation_block(
            context(SOURCE_WAVE, None),
            &batch(ROUND),
            list(ROUND - 1, vec![entry(1, 90)]),
            &config,
        ),
        Err(PorError::InvalidReputationBlockRound)
    );
}

#[test]
fn rejects_duplicate_or_unsorted_reputation_entries() {
    let config = PorConfig::default();
    let ratings = batch(ROUND);

    assert_eq!(
        build_reputation_block(
            context(SOURCE_WAVE, None),
            &ratings,
            list(ROUND, vec![entry(1, 90), entry(1, 30)]),
            &config,
        ),
        Err(PorError::DuplicateReputationEntry)
    );
    assert_eq!(
        build_reputation_block(
            context(SOURCE_WAVE, None),
            &ratings,
            list(ROUND, vec![entry(2, 30), entry(1, 90)]),
            &config,
        ),
        Err(PorError::UnsortedReputationVector)
    );
}

#[test]
fn rejects_invalid_shard_context() {
    let config = PorConfig::default();
    let ratings = batch(ROUND);
    let reputation_list = list(ROUND, vec![entry(1, 90)]);

    assert_eq!(
        build_reputation_block(
            ReputationBlockContext {
                shard_id: b"",
                source_finalized_wave: SOURCE_WAVE,
                previous_block: None,
            },
            &ratings,
            reputation_list.clone(),
            &config,
        ),
        Err(PorError::MissingReputationBlockShardId)
    );
    let oversized_shard = vec![0; MAX_REPUTATION_BLOCK_SHARD_ID_LEN + 1];
    assert_eq!(
        build_reputation_block(
            ReputationBlockContext {
                shard_id: &oversized_shard,
                source_finalized_wave: SOURCE_WAVE,
                previous_block: None,
            },
            &ratings,
            reputation_list,
            &config,
        ),
        Err(PorError::ReputationBlockShardIdTooLong)
    );
}

#[test]
fn structural_validation_rejects_header_or_root_tampering() {
    let config = PorConfig::default();
    let mut block = build_reputation_block(
        context(SOURCE_WAVE, None),
        &batch(ROUND),
        list(ROUND, vec![entry(1, 90)]),
        &config,
    )
    .unwrap();

    block.header.version += 1;
    assert_eq!(
        validate_reputation_block(&block),
        Err(PorError::UnsupportedReputationBlockVersion(
            REPUTATION_BLOCK_VERSION + 1
        ))
    );

    block.header.version = REPUTATION_BLOCK_VERSION;
    block.reputation_list.entries[0].reputation += 1;
    assert_eq!(
        validate_reputation_block(&block),
        Err(PorError::ReputationBlockRootMismatch)
    );
}
