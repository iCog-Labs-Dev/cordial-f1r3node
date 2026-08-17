use cordial_miners_core::NodeId;
use cordial_por::{PorError, ReputationEntry, ReputationState, ReputationVector};

fn entry(node: u8, reputation: u64) -> ReputationEntry {
    ReputationEntry::new(NodeId(vec![node]), reputation)
}

fn vector(round: u64, values: Vec<ReputationEntry>) -> ReputationVector {
    ReputationVector { round, values }
}

#[test]
fn set_reputation_inserts_entries_in_node_id_order() {
    let mut state = ReputationState::new(0);

    state.set_reputation(NodeId(vec![3]), 30);
    state.set_reputation(NodeId(vec![1]), 10);
    state.set_reputation(NodeId(vec![2]), 20);

    let entries = &state.reputation_list().entries;

    assert_eq!(entries[0].node_id, NodeId(vec![1]));
    assert_eq!(entries[1].node_id, NodeId(vec![2]));
    assert_eq!(entries[2].node_id, NodeId(vec![3]));
}

#[test]
fn set_reputation_updates_existing_entry_without_duplicate() {
    let mut state = ReputationState::new(0);

    state.set_reputation(NodeId(vec![2]), 20);
    state.set_reputation(NodeId(vec![1]), 10);
    state.set_reputation(NodeId(vec![2]), 99);

    let entries = &state.reputation_list().entries;

    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].node_id, NodeId(vec![1]));
    assert_eq!(entries[0].reputation, 10);
    assert_eq!(entries[1].node_id, NodeId(vec![2]));
    assert_eq!(entries[1].reputation, 99);
}

#[test]
fn apply_reputation_vector_replaces_state_snapshot_and_round() {
    let mut state = ReputationState::new(1);
    state.set_reputation(NodeId(vec![1]), 10);
    state.set_reputation(NodeId(vec![2]), 20);

    state
        .apply_reputation_vector(&vector(7, vec![entry(1, 90), entry(3, 30)]))
        .unwrap();

    assert_eq!(state.round(), 7);
    assert_eq!(state.reputation_list().round, 7);
    assert_eq!(
        state.reputation_list().entries,
        vec![entry(1, 90), entry(3, 30)]
    );
}

#[test]
fn apply_empty_reputation_vector_updates_round_and_clears_entries() {
    let mut state = ReputationState::new(1);
    state.set_reputation(NodeId(vec![1]), 10);

    state
        .apply_reputation_vector(&vector(8, Vec::new()))
        .unwrap();

    assert_eq!(state.round(), 8);
    assert_eq!(state.reputation_list().round, 8);
    assert!(state.reputation_list().entries.is_empty());
}

#[test]
fn apply_reputation_vector_preserves_canonical_vector_order() {
    let mut state = ReputationState::new(0);

    state
        .apply_reputation_vector(&vector(9, vec![entry(1, 10), entry(2, 20), entry(3, 30)]))
        .unwrap();

    assert_eq!(
        state.reputation_list().entries,
        vec![entry(1, 10), entry(2, 20), entry(3, 30)]
    );
}

#[test]
fn apply_reputation_vector_rejects_duplicate_entries_without_mutating_state() {
    let mut state = ReputationState::new(1);
    state.set_reputation(NodeId(vec![1]), 10);
    let before = state.clone();

    let result = state.apply_reputation_vector(&vector(2, vec![entry(1, 20), entry(1, 30)]));

    assert_eq!(result, Err(PorError::DuplicateReputationEntry));
    assert_eq!(state, before);
}

#[test]
fn apply_reputation_vector_rejects_unsorted_entries_without_mutating_state() {
    let mut state = ReputationState::new(1);
    state.set_reputation(NodeId(vec![1]), 10);
    let before = state.clone();

    let result = state.apply_reputation_vector(&vector(2, vec![entry(2, 20), entry(1, 10)]));

    assert_eq!(result, Err(PorError::UnsortedReputationVector));
    assert_eq!(state, before);
}
