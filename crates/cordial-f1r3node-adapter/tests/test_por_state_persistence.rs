use cordial_f1r3node_adapter::por::{PorStateStore, PorStateStoreError};
use cordial_miners_core::NodeId;
use cordial_por::{
    MissingEntryPolicy, PorConfig, PorError, RatingRecord, ReputationBlockContext, ReputationState,
    ReputationVector, build_rating_batch, build_reputation_block, replay_reputation_transition,
};
use tempfile::tempdir;

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
fn fresh_store_has_no_reputation_state() {
    let directory = tempdir().unwrap();
    let store = PorStateStore::open(directory.path()).unwrap();

    assert_eq!(store.restore().unwrap(), None);
}

#[test]
fn complete_state_and_latest_block_survive_reopen() {
    let directory = tempdir().unwrap();
    let expected = audited_state();

    {
        let store = PorStateStore::open(directory.path()).unwrap();
        store.persist(&expected).unwrap();
        assert!(store.snapshot_path().is_file());
    }

    let reopened = PorStateStore::open(directory.path()).unwrap();
    let restored = reopened.restore().unwrap().unwrap();
    assert_eq!(restored, expected);
    assert_eq!(restored.latest_block(), expected.latest_block());
    assert!(restored.is_ejected(&node(1)));
}

#[test]
fn newer_snapshot_atomically_replaces_the_previous_state() {
    let directory = tempdir().unwrap();
    let store = PorStateStore::open(directory.path()).unwrap();
    store.persist(&initial_state()).unwrap();

    let expected = audited_state();
    store.persist(&expected).unwrap();

    assert_eq!(store.restore().unwrap(), Some(expected));
}

#[test]
fn failed_replacement_preserves_the_last_committed_snapshot() {
    let directory = tempdir().unwrap();
    let store = PorStateStore::open(directory.path()).unwrap();
    let expected = audited_state();
    store.persist(&expected).unwrap();

    let mut invalid_replacement = initial_state();
    invalid_replacement.add_rating(RatingRecord::new(ROUND, node(1), node(2), 50, vec![1]));
    assert!(matches!(
        store.persist(&invalid_replacement),
        Err(PorStateStoreError::InvalidSnapshot(
            PorError::ReputationStateSnapshotHasPendingRatings
        ))
    ));

    assert_eq!(store.restore().unwrap(), Some(expected));
}

#[test]
fn corrupt_committed_snapshot_is_an_error_not_a_fresh_boot() {
    let directory = tempdir().unwrap();
    let store = PorStateStore::open(directory.path()).unwrap();
    store.persist(&audited_state()).unwrap();

    let mut bytes = std::fs::read(store.snapshot_path()).unwrap();
    let midpoint = bytes.len() / 2;
    bytes[midpoint] ^= 1;
    std::fs::write(store.snapshot_path(), bytes).unwrap();

    assert!(matches!(
        store.restore(),
        Err(PorStateStoreError::InvalidSnapshot(
            PorError::ReputationStateSnapshotChecksumMismatch
        ))
    ));
}

#[test]
fn interrupted_temporary_file_is_ignored_on_restart() {
    let directory = tempdir().unwrap();
    let expected = audited_state();
    let store = PorStateStore::open(directory.path()).unwrap();
    store.persist(&expected).unwrap();

    let temporary_path = store
        .snapshot_path()
        .parent()
        .unwrap()
        .join(".reputation-state.bin.tmp");
    std::fs::write(temporary_path, b"interrupted replacement").unwrap();
    drop(store);

    let reopened = PorStateStore::open(directory.path()).unwrap();
    assert_eq!(reopened.restore().unwrap(), Some(expected));
}
