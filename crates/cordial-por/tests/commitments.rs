use cordial_miners_core::NodeId;
use cordial_por::{
    MissingEntryPolicy, PorConfig, RatingRecord, ReputationBlockContext, ReputationEntry,
    ReputationList, build_rating_batch, build_reputation_block, config_commitment,
    rating_batch_commitment, reputation_block_hash, reputation_list_commitment,
};

const SHARD_ID: &[u8] = b"root";
const CONFIG_COMMITMENT_V1: [u8; 32] = [
    224, 122, 115, 222, 167, 17, 235, 116, 112, 65, 106, 207, 115, 129, 219, 14, 15, 244, 20, 3,
    187, 176, 229, 78, 206, 253, 87, 138, 57, 245, 205, 222,
];
const RATING_BATCH_COMMITMENT_V1: [u8; 32] = [
    232, 164, 6, 130, 63, 164, 232, 78, 78, 171, 192, 221, 14, 49, 252, 205, 233, 0, 62, 152, 3,
    226, 213, 35, 88, 151, 210, 50, 138, 221, 88, 149,
];
const REPUTATION_LIST_COMMITMENT_V1: [u8; 32] = [
    64, 226, 122, 114, 81, 63, 78, 61, 60, 226, 229, 17, 115, 133, 207, 56, 78, 143, 227, 21, 185,
    46, 90, 98, 102, 68, 116, 12, 182, 211, 28, 219,
];
const REPUTATION_BLOCK_HASH_V1: [u8; 32] = [
    104, 96, 94, 216, 223, 236, 231, 172, 101, 160, 26, 188, 186, 160, 28, 130, 129, 165, 168, 153,
    190, 44, 14, 202, 10, 153, 178, 43, 214, 160, 118, 77,
];

fn config() -> PorConfig {
    PorConfig {
        scale: 100,
        initial_reputation: 20,
        liquid_rank_alpha: 60,
        minimum_rating: 10,
        maximum_rating: 100,
        missing_entry_policy: MissingEntryPolicy::CarryForward,
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
fn v1_commitments_match_golden_vectors() {
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

    assert_eq!(config_commitment(&config), CONFIG_COMMITMENT_V1);
    assert_eq!(
        rating_batch_commitment(&ratings, &config).unwrap(),
        RATING_BATCH_COMMITMENT_V1
    );
    assert_eq!(
        reputation_list_commitment(&list).unwrap(),
        REPUTATION_LIST_COMMITMENT_V1
    );
    assert_eq!(
        reputation_block_hash(&block).unwrap(),
        REPUTATION_BLOCK_HASH_V1
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
