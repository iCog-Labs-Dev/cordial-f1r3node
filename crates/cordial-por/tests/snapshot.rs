use cordial_miners_core::{
    NodeId,
    crypto::{Blake2b256Hasher, Hasher},
};
use cordial_por::{
    MAX_REPUTATION_STATE_NODE_ID_LEN, MissingEntryPolicy, POR_STATE_SNAPSHOT_MAGIC, PorConfig,
    PorError, RatingRecord, ReputationBlockContext, ReputationEntry, ReputationState,
    ReputationVector, build_rating_batch, build_reputation_block, decode_reputation_state_snapshot,
    encode_reputation_state_snapshot, replay_reputation_transition,
};

const ROUND: u64 = 1;
const SHARD_ID: &[u8] = b"root";

fn node(id: u8) -> NodeId {
    NodeId(vec![id])
}

fn config() -> PorConfig {
    PorConfig {
        scale: 100,
        initial_reputation: 0,
        liquid_rank_alpha: 50,
        minimum_rating: 0,
        maximum_rating: 100,
        missing_entry_policy: MissingEntryPolicy::CarryForward,
    }
}

fn ratings() -> Vec<RatingRecord> {
    vec![
        RatingRecord::new(ROUND, node(1), node(2), 80, vec![0x01]),
        RatingRecord::new(ROUND, node(2), node(1), 60, vec![0x02]),
    ]
}

fn initial_state() -> ReputationState {
    let mut state = ReputationState::new(ROUND - 1);
    state.set_reputation(node(1), 40);
    state.set_reputation(node(2), 60);
    state
}

fn audited_state() -> ReputationState {
    let config = config();
    let ratings = ratings();
    let mut state = initial_state();
    let previous = ReputationVector {
        round: state.round(),
        values: state.reputation_list().entries.clone(),
    };
    let list = replay_reputation_transition(&previous, &ratings, ROUND, &config).unwrap();
    let batch = build_rating_batch(ROUND, ratings.clone(), &config).unwrap();
    let block = build_reputation_block(
        ReputationBlockContext {
            shard_id: SHARD_ID,
            source_finalized_wave: ROUND - 1,
            previous_block: None,
        },
        &batch,
        list,
        &config,
    )
    .unwrap();
    state
        .apply_reputation_block(SHARD_ID, ROUND - 1, &ratings, block, &config)
        .unwrap();
    state.eject_validator(&node(1)).unwrap();
    state
}

#[test]
fn v1_snapshot_round_trips_the_complete_finalized_state() {
    let state = audited_state();
    let encoded = encode_reputation_state_snapshot(&state).unwrap();
    let restored = decode_reputation_state_snapshot(&encoded).unwrap();

    assert_eq!(restored, state);
    assert_eq!(restored.latest_block(), state.latest_block());
    assert!(restored.is_ejected(&node(1)));
}

#[test]
fn v1_snapshot_encoding_is_deterministic() {
    let state = audited_state();
    let first = encode_reputation_state_snapshot(&state).unwrap();
    let second = encode_reputation_state_snapshot(&state).unwrap();

    assert_eq!(first, second);
    assert_eq!(
        Blake2b256Hasher.hash(&first),
        [
            68, 196, 208, 65, 40, 25, 73, 53, 178, 96, 174, 79, 73, 233, 160, 190, 99, 250, 31,
            184, 60, 75, 141, 242, 120, 247, 252, 22, 17, 135, 8, 18,
        ]
    );
}

#[test]
fn rejects_checksum_corruption_and_trailing_bytes() {
    let mut corrupted = encode_reputation_state_snapshot(&audited_state()).unwrap();
    let payload_start = POR_STATE_SNAPSHOT_MAGIC.len() + 2 + 8;
    corrupted[payload_start] ^= 1;
    assert_eq!(
        decode_reputation_state_snapshot(&corrupted),
        Err(PorError::ReputationStateSnapshotChecksumMismatch)
    );

    let mut trailing = encode_reputation_state_snapshot(&audited_state()).unwrap();
    trailing.push(0);
    assert_eq!(
        decode_reputation_state_snapshot(&trailing),
        Err(PorError::MalformedReputationStateSnapshot)
    );
}

#[test]
fn rejects_unsupported_or_truncated_snapshots() {
    let encoded = encode_reputation_state_snapshot(&audited_state()).unwrap();
    let mut unsupported = encoded.clone();
    let version_offset = POR_STATE_SNAPSHOT_MAGIC.len();
    unsupported[version_offset..version_offset + 2].copy_from_slice(&2_u16.to_be_bytes());
    assert_eq!(
        decode_reputation_state_snapshot(&unsupported),
        Err(PorError::UnsupportedReputationStateSnapshotVersion(2))
    );

    let truncated = &encoded[..encoded.len() - 1];
    assert_eq!(
        decode_reputation_state_snapshot(truncated),
        Err(PorError::MalformedReputationStateSnapshot)
    );
}

#[test]
fn refuses_to_silently_persist_pending_ratings() {
    let mut state = initial_state();
    state.add_rating(RatingRecord::new(ROUND, node(1), node(2), 50, vec![1]));

    assert_eq!(
        encode_reputation_state_snapshot(&state),
        Err(PorError::ReputationStateSnapshotHasPendingRatings)
    );
}

#[test]
fn bounds_persisted_node_identifiers() {
    let mut state = ReputationState::new(0);
    state.set_reputation(NodeId(vec![0; MAX_REPUTATION_STATE_NODE_ID_LEN + 1]), 10);

    assert_eq!(
        encode_reputation_state_snapshot(&state),
        Err(PorError::ReputationStateSnapshotTooLarge)
    );
}

#[test]
fn rejects_an_exclusion_flag_missing_from_the_permanent_registry() {
    let mut state = ReputationState::new(0);
    state
        .apply_reputation_vector(ReputationVector {
            round: 1,
            values: vec![ReputationEntry::ejected(node(1))],
        })
        .unwrap();

    assert_eq!(
        encode_reputation_state_snapshot(&state),
        Err(PorError::ReputationStateSnapshotExclusionMismatch)
    );
}

#[test]
fn rejects_a_latest_block_stale_after_direct_state_replacement() {
    let mut state = audited_state();
    state
        .apply_reputation_vector(ReputationVector {
            round: ROUND + 1,
            values: state.reputation_list().entries.clone(),
        })
        .unwrap();

    assert_eq!(
        encode_reputation_state_snapshot(&state),
        Err(PorError::ReputationStateSnapshotBlockRoundMismatch)
    );
}
