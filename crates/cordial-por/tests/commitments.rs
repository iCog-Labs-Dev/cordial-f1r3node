use cordial_miners_core::NodeId;
use cordial_por::{
    MissingEntryPolicy, PorConfig, RatingRecord, ReputationBlockContext, ReputationEntry,
    ReputationList, build_rating_batch, build_reputation_block, config_commitment,
    rating_batch_commitment, reputation_block_hash, reputation_list_commitment,
};

const SHARD_ID: &[u8] = b"root";
const CONFIG_COMMITMENT_V2: [u8; 32] = [
    215, 114, 12, 110, 6, 248, 14, 172, 138, 85, 23, 233, 230, 91, 205, 243, 115, 161, 172, 220,
    92, 241, 223, 203, 53, 114, 214, 24, 232, 92, 43, 176,
];
const RATING_BATCH_COMMITMENT_V1: [u8; 32] = [
    232, 164, 6, 130, 63, 164, 232, 78, 78, 171, 192, 221, 14, 49, 252, 205, 233, 0, 62, 152, 3,
    226, 213, 35, 88, 151, 210, 50, 138, 221, 88, 149,
];
const REPUTATION_LIST_COMMITMENT_V2: [u8; 32] = [
    122, 243, 61, 252, 112, 160, 205, 116, 184, 213, 90, 130, 129, 87, 185, 137, 77, 107, 130, 66,
    170, 233, 162, 75, 171, 154, 124, 253, 227, 110, 28, 247,
];
const REPUTATION_BLOCK_HASH_WITH_CONFIG_V2: [u8; 32] = [
    233, 142, 240, 128, 29, 148, 240, 184, 226, 12, 23, 151, 43, 83, 19, 255, 180, 93, 139, 124,
    244, 184, 56, 12, 1, 61, 124, 111, 154, 152, 107, 221,
];

fn config() -> PorConfig {
    PorConfig {
        scale: 100,
        initial_reputation: 20,
        liquid_rank_alpha: 60,
        minimum_rating: 10,
        maximum_rating: 100,
        missing_entry_policy: MissingEntryPolicy::CarryForward,
        ..PorConfig::new(100, 20)
    }
}

fn rating(rater: u8, recipient: u8, score: u64, signature: u8) -> RatingRecord {
    RatingRecord {
        round: 1,
        rater: NodeId(vec![rater]),
        recipient: NodeId(vec![recipient]),
        score,
        signature: vec![signature],
        interaction_ref: Some(vec![recipient; 32]),
    }
}

fn ratings() -> cordial_por::RatingBatch {
    build_rating_batch(
        1,
        vec![rating(2, 1, 80, 0xa2), rating(1, 2, 70, 0xa1)],
        &config(),
    )
    .unwrap()
}

fn reputation_list() -> ReputationList {
    ReputationList {
        round: 1,
        entries: vec![
            ReputationEntry::new(NodeId(vec![1]), 90),
            ReputationEntry::ejected(NodeId(vec![2])),
        ],
    }
}

#[test]
fn commitments_with_config_v2_match_golden_vectors() {
    let config = config();
    let ratings = ratings();
    let list = reputation_list();
    let block = build_reputation_block(
        ReputationBlockContext {
            shard_id: SHARD_ID,
            source_finalized_wave: 0,
            previous_block: None,
        },
        &ratings,
        list.clone(),
        &config,
    )
    .unwrap();

    assert_eq!(config_commitment(&config), CONFIG_COMMITMENT_V2);
    assert_eq!(
        rating_batch_commitment(&ratings, &config).unwrap(),
        RATING_BATCH_COMMITMENT_V1
    );
    assert_eq!(
        reputation_list_commitment(&list).unwrap(),
        REPUTATION_LIST_COMMITMENT_V2
    );
    assert_eq!(
        reputation_block_hash(&block).unwrap(),
        REPUTATION_BLOCK_HASH_WITH_CONFIG_V2
    );
}

#[test]
fn rating_batch_commitment_is_arrival_order_independent() {
    let mut reversed = ratings();
    reversed.ratings.reverse();

    assert_eq!(
        rating_batch_commitment(&ratings(), &config()).unwrap(),
        rating_batch_commitment(&reversed, &config()).unwrap()
    );
}

#[test]
fn commitments_cover_signatures_configuration_and_exclusion() {
    let config = config();
    let original_ratings = ratings();
    let original_list = reputation_list();

    let mut changed_ratings = original_ratings.clone();
    changed_ratings.ratings[0].signature[0] ^= 1;
    assert_ne!(
        rating_batch_commitment(&original_ratings, &config).unwrap(),
        rating_batch_commitment(&changed_ratings, &config).unwrap()
    );

    let changed_config = PorConfig {
        missing_entry_policy: MissingEntryPolicy::Neutral,
        ..config.clone()
    };
    assert_ne!(
        config_commitment(&config),
        config_commitment(&changed_config)
    );

    let mut changed_list = original_list.clone();
    changed_list.entries[1].is_excluded = false;
    assert_ne!(
        reputation_list_commitment(&original_list).unwrap(),
        reputation_list_commitment(&changed_list).unwrap()
    );
}

#[test]
fn commitment_covers_every_penalty_parameter() {
    let original = config();
    for field in 0..3 {
        let mut changed = original.clone();
        match field {
            0 => changed.correlation_threshold += 1,
            1 => changed.base_slash_penalty += 1,
            _ => changed.inactivity_decay_gamma += 1,
        }
        assert_ne!(config_commitment(&original), config_commitment(&changed));
    }
}
