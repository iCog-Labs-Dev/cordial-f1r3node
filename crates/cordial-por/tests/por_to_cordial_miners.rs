use std::collections::HashSet;

use cordial_miners_core::{
    Block, BlockContent, BlockIdentity, NodeId,
    consensus::{is_supermajority, is_weighted_supermajority},
};
use cordial_por::{
    MissingEntryPolicy, PorConfig, RatingRecord, ReputationEntry, ReputationState,
    ReputationVector, blend_reputation_transition, build_rating_batch, build_rating_matrix,
    clamp_reputation_transition, compute_liquid_rank_contribution, normalize_rating_matrix,
    reputation_weights,
};

const PREVIOUS_ROUND: u64 = 0;
const RATING_ROUND: u64 = 1;
const SCALE: u64 = 1_000;

fn node(id: u8) -> NodeId {
    NodeId(vec![id])
}

fn entry(id: u8, reputation: u64) -> ReputationEntry {
    ReputationEntry::new(node(id), reputation)
}

fn rating(rater: u8, recipient: u8, score: u64) -> RatingRecord {
    let mut record = RatingRecord::new(
        RATING_ROUND,
        node(rater),
        node(recipient),
        score,
        vec![rater, recipient],
    );
    record.interaction_ref = Some(vec![RATING_ROUND as u8, rater, recipient]);
    record
}

fn config() -> PorConfig {
    PorConfig {
        scale: SCALE,
        initial_reputation: SCALE,
        liquid_rank_alpha: SCALE,
        minimum_rating: 0,
        maximum_rating: SCALE,
        missing_entry_policy: MissingEntryPolicy::CarryForward,
    }
}

fn previous_reputation() -> ReputationVector {
    ReputationVector {
        round: PREVIOUS_ROUND,
        values: vec![
            entry(1, SCALE),
            entry(2, SCALE),
            entry(3, SCALE),
            entry(4, SCALE),
        ],
    }
}

fn support_block(creator: u8, tag: u8) -> Block {
    let mut content_hash = [0u8; 32];
    content_hash[0] = creator;
    content_hash[1] = tag;

    Block {
        identity: BlockIdentity {
            content_hash,
            creator: node(creator),
            signature: vec![tag],
        },
        content: BlockContent {
            payload: vec![tag],
            predecessors: HashSet::new(),
        },
    }
}

#[test]
fn por_reputation_weights_drive_cordial_miners_weighted_supermajority() {
    let config = config();
    let previous = previous_reputation();
    let ratings = vec![
        rating(2, 1, 1_000),
        rating(3, 1, 900),
        rating(4, 1, 800),
        rating(1, 2, 1_000),
        rating(3, 2, 900),
        rating(1, 3, 1_000),
    ];

    let batch = build_rating_batch(RATING_ROUND, ratings, &config).unwrap();
    let matrix = build_rating_matrix(&batch).unwrap();
    let normalized = normalize_rating_matrix(&matrix, &config).unwrap();
    let contribution = compute_liquid_rank_contribution(&normalized, &previous, &config).unwrap();
    let blended = blend_reputation_transition(&contribution, &previous, &config).unwrap();
    let clamped = clamp_reputation_transition(&blended, &previous, &contribution, &config).unwrap();

    let mut state = ReputationState::new(PREVIOUS_ROUND);
    state.apply_reputation_vector(clamped).unwrap();
    state.eject_validator(&node(4)).unwrap();

    let weights = reputation_weights(&state);

    assert_eq!(weights.get(&node(1)), Some(&940));
    assert_eq!(weights.get(&node(2)), Some(&886));
    assert_eq!(weights.get(&node(3)), Some(&707));
    assert!(!weights.contains_key(&node(4)));

    let weighted_support = HashSet::from([node(1), node(2)]);
    assert!(is_weighted_supermajority(&weighted_support, &weights));

    let unweighted_support = HashSet::from([support_block(1, 1), support_block(2, 2)]);
    assert!(!is_supermajority(&unweighted_support, 4, 1));
}
