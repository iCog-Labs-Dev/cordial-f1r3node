use cordial_miners_core::NodeId;
use cordial_por::ReputationState;

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
