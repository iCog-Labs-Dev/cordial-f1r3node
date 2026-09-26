//! Acceptance tests for exporting externally authorized validator weights.

use cordial_miners_core::NodeId;
use cordial_por::{PorError, ReputationState, authorized_validator_weights};

#[test]
fn empty_authorized_set_fails_closed() {
    let mut state = ReputationState::new(1);
    state.set_reputation(NodeId(vec![1]), 750);

    let error = authorized_validator_weights(&state, &[]).unwrap_err();

    assert_eq!(error, PorError::EmptyAuthorizedValidatorSet);
}

#[test]
fn single_authorized_validator_exports_exact_weight() {
    let node = NodeId(vec![1]);
    let mut state = ReputationState::new(1);
    state.set_reputation(node.clone(), 750);

    let weights = authorized_validator_weights(&state, std::slice::from_ref(&node)).unwrap();

    assert_eq!(weights.len(), 1);
    assert_eq!(weights.get(&node), Some(&750));
}

#[test]
fn multiple_authorized_validators_export_exact_weights() {
    let node_a = NodeId(vec![1]);
    let node_b = NodeId(vec![2]);
    let node_c = NodeId(vec![3]);
    let mut state = ReputationState::new(1);
    state.set_reputation(node_a.clone(), 900);
    state.set_reputation(node_b.clone(), 600);
    state.set_reputation(node_c.clone(), 300);

    let weights =
        authorized_validator_weights(&state, &[node_a.clone(), node_b.clone(), node_c.clone()])
            .unwrap();

    assert_eq!(weights.len(), 3);
    assert_eq!(weights.get(&node_a), Some(&900));
    assert_eq!(weights.get(&node_b), Some(&600));
    assert_eq!(weights.get(&node_c), Some(&300));
}

#[test]
fn excludes_unauthorized_validators() {
    let authorized = NodeId(vec![1]);
    let unauthorized = NodeId(vec![2]);
    let mut state = ReputationState::new(1);
    state.set_reputation(authorized.clone(), 700);
    state.set_reputation(unauthorized.clone(), 400);

    let weights = authorized_validator_weights(&state, std::slice::from_ref(&authorized)).unwrap();

    assert!(weights.contains_key(&authorized));
    assert!(!weights.contains_key(&unauthorized));
}

#[test]
fn missing_authorized_validator_fails_closed() {
    let known = NodeId(vec![1]);
    let unknown = NodeId(vec![2]);
    let mut state = ReputationState::new(1);
    state.set_reputation(known.clone(), 700);

    let error = authorized_validator_weights(&state, &[known, unknown.clone()]).unwrap_err();

    assert_eq!(
        error,
        PorError::MissingAuthorizedValidatorReputation(unknown)
    );
}

#[test]
fn duplicate_authorization_does_not_change_exported_weight() {
    let node = NodeId(vec![9]);
    let mut state = ReputationState::new(7);
    state.set_reputation(node.clone(), 987_654_321);

    let weights = authorized_validator_weights(&state, &[node.clone(), node.clone()]).unwrap();

    assert_eq!(weights.len(), 1);
    assert_eq!(weights.get(&node), Some(&987_654_321));
}

#[test]
fn ejected_authorized_validator_keeps_membership_with_zero_weight() {
    let active = NodeId(vec![1]);
    let ejected = NodeId(vec![2]);
    let mut state = ReputationState::new(1);
    state.set_reputation(active.clone(), 700);
    state.set_reputation(ejected.clone(), 400);
    state.eject_validator(&ejected).unwrap();

    let weights = authorized_validator_weights(&state, &[active.clone(), ejected.clone()]).unwrap();

    assert_eq!(weights.len(), 2);
    assert_eq!(weights.get(&active), Some(&700));
    assert_eq!(weights.get(&ejected), Some(&0));
}

#[test]
fn zero_total_authorized_weight_fails_closed() {
    let node_a = NodeId(vec![1]);
    let node_b = NodeId(vec![2]);
    let mut state = ReputationState::new(1);
    state.set_reputation(node_a.clone(), 0);
    state.set_reputation(node_b.clone(), 0);

    let error = authorized_validator_weights(&state, &[node_a, node_b]).unwrap_err();

    assert_eq!(error, PorError::ZeroAuthorizedValidatorWeight);
}
