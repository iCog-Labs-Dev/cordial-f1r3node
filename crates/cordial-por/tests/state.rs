use cordial_miners_core::NodeId;
use cordial_por::{
    PorError, ReputationEntry, ReputationState, ReputationVector, reputation_weights,
};

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
        .apply_reputation_vector(vector(7, vec![entry(1, 90), entry(3, 30)]))
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
        .apply_reputation_vector(vector(8, Vec::new()))
        .unwrap();

    assert_eq!(state.round(), 8);
    assert_eq!(state.reputation_list().round, 8);
    assert!(state.reputation_list().entries.is_empty());
}

#[test]
fn apply_reputation_vector_preserves_canonical_vector_order() {
    let mut state = ReputationState::new(0);

    state
        .apply_reputation_vector(vector(9, vec![entry(1, 10), entry(2, 20), entry(3, 30)]))
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

    let result = state.apply_reputation_vector(vector(2, vec![entry(1, 20), entry(1, 30)]));

    assert_eq!(result, Err(PorError::DuplicateReputationEntry));
    assert_eq!(state, before);
}

#[test]
fn apply_reputation_vector_rejects_unsorted_entries_without_mutating_state() {
    let mut state = ReputationState::new(1);
    state.set_reputation(NodeId(vec![1]), 10);
    let before = state.clone();

    let result = state.apply_reputation_vector(vector(2, vec![entry(2, 20), entry(1, 10)]));

    assert_eq!(result, Err(PorError::UnsortedReputationVector));
    assert_eq!(state, before);
}

// ============================================================
// Key ejection tests
// ============================================================

#[test]
fn eject_validator_returns_unknown_node_for_absent_node() {
    let mut state = ReputationState::new(0);
    state.set_reputation(NodeId(vec![1]), 100);

    let result = state.eject_validator(&NodeId(vec![99]));

    assert_eq!(result, Err(PorError::UnknownNode));
}

#[test]
fn eject_validator_sets_is_excluded_and_zeros_weight() {
    let mut state = ReputationState::new(0);
    state.set_reputation(NodeId(vec![1]), 1_000);
    state.set_reputation(NodeId(vec![2]), 2_000);

    state.eject_validator(&NodeId(vec![1])).unwrap();

    let entries = &state.reputation_list().entries;
    let ejected = entries
        .iter()
        .find(|e| e.node_id == NodeId(vec![1]))
        .unwrap();

    assert!(ejected.is_excluded);
    assert_eq!(ejected.reputation, 0);

    // Node 2 is unaffected.
    let active = entries
        .iter()
        .find(|e| e.node_id == NodeId(vec![2]))
        .unwrap();
    assert!(!active.is_excluded);
    assert_eq!(active.reputation, 2_000);
}

#[test]
fn eject_validator_is_idempotent_on_double_call() {
    let mut state = ReputationState::new(0);
    state.set_reputation(NodeId(vec![1]), 1_000);

    state.eject_validator(&NodeId(vec![1])).unwrap();
    // Second call on the same node must succeed without panicking.
    state.eject_validator(&NodeId(vec![1])).unwrap();

    let entry = state
        .reputation_list()
        .entries
        .iter()
        .find(|e| e.node_id == NodeId(vec![1]))
        .unwrap();

    assert!(entry.is_excluded);
    assert_eq!(entry.reputation, 0);
}

#[test]
fn set_reputation_silently_ignores_an_ejected_node() {
    let mut state = ReputationState::new(0);
    state.set_reputation(NodeId(vec![1]), 1_000);
    state.eject_validator(&NodeId(vec![1])).unwrap();

    // Attempt to restore reputation via set_reputation must be a no-op.
    state.set_reputation(NodeId(vec![1]), 9_999);

    let entry = state
        .reputation_list()
        .entries
        .iter()
        .find(|e| e.node_id == NodeId(vec![1]))
        .unwrap();

    assert!(entry.is_excluded);
    assert_eq!(entry.reputation, 0);
}

#[test]
fn apply_reputation_vector_preserves_ejection_across_rounds() {
    let mut state = ReputationState::new(0);
    state.set_reputation(NodeId(vec![1]), 500);
    state.set_reputation(NodeId(vec![2]), 500);
    state.eject_validator(&NodeId(vec![1])).unwrap();

    // A new vector arrives with node 1 carrying a non-zero weight — ejection
    // must be re-applied.
    state
        .apply_reputation_vector(vector(1, vec![entry(1, 999), entry(2, 800)]))
        .unwrap();

    let entries = &state.reputation_list().entries;
    let ejected = entries
        .iter()
        .find(|e| e.node_id == NodeId(vec![1]))
        .unwrap();
    assert!(ejected.is_excluded);
    assert_eq!(ejected.reputation, 0);

    let active = entries
        .iter()
        .find(|e| e.node_id == NodeId(vec![2]))
        .unwrap();
    assert!(!active.is_excluded);
    assert_eq!(active.reputation, 800);
}

#[test]
fn reputation_weights_omits_ejected_nodes() {
    let mut state = ReputationState::new(0);
    state.set_reputation(NodeId(vec![1]), 1_000);
    state.set_reputation(NodeId(vec![2]), 2_000);
    state.eject_validator(&NodeId(vec![1])).unwrap();

    let weights = reputation_weights(&state);

    assert!(!weights.contains_key(&NodeId(vec![1])));
    assert_eq!(weights[&NodeId(vec![2])], 2_000);
}

// ============================================================
// Regression tests for reviewer bugs (Johnnas12)
// ============================================================

/// Bug 1: ejected node absent from new vector could be resurrected.
///
/// If `apply_reputation_vector` is called with a vector that omits an ejected
/// node, and then `set_reputation` is called for that same node, the old
/// implementation would silently re-insert it as active because it was no
/// longer present in the list. The `excluded_keys` registry must prevent this.
#[test]
fn ejected_node_absent_from_new_vector_cannot_be_resurrected_via_set_reputation() {
    let mut state = ReputationState::new(0);
    state.set_reputation(NodeId(vec![1]), 1_000);
    state.set_reputation(NodeId(vec![2]), 2_000);
    state.eject_validator(&NodeId(vec![1])).unwrap();

    // New vector deliberately omits node 1.
    state
        .apply_reputation_vector(vector(1, vec![entry(2, 1_500)]))
        .unwrap();

    // Attempt to re-insert ejected node via set_reputation — must be a no-op.
    state.set_reputation(NodeId(vec![1]), 9_999);

    // The permanent registry must still mark node 1 as ejected.
    assert!(state.is_ejected(&NodeId(vec![1])));

    // reputation_weights must not expose node 1 with any weight.
    let weights = reputation_weights(&state);
    assert!(!weights.contains_key(&NodeId(vec![1])));
}

/// Bug 2: `is_ejected` consults the permanent registry, not the list flag.
///
/// Callers must be able to rely on `is_ejected` returning `true` even when
/// the `reputation_list` entry is absent or its `is_excluded` flag is somehow
/// inconsistent. The `excluded_keys` set is the sole source of truth.
#[test]
fn is_ejected_consults_permanent_registry_not_list_flag() {
    let mut state = ReputationState::new(0);
    state.set_reputation(NodeId(vec![1]), 500);
    state.eject_validator(&NodeId(vec![1])).unwrap();

    // Apply a vector that omits node 1 — its list entry disappears.
    state
        .apply_reputation_vector(vector(1, vec![entry(2, 800)]))
        .unwrap();

    // is_ejected must still return true via the permanent registry.
    assert!(state.is_ejected(&NodeId(vec![1])));
    // Node 2 must remain unaffected.
    assert!(!state.is_ejected(&NodeId(vec![2])));
}
