use cordial_miners_core::NodeId;

use cordial_por::{PorConfig, RatingRecord, ReputationState, reputation_weights};

#[test]
fn creates_rating_record() {
    let rater = NodeId(vec![1]);
    let recipient = NodeId(vec![2]);

    let rating = RatingRecord::new(1, rater, recipient, 500);

    assert_eq!(rating.round, 1);
    assert_eq!(rating.score, 500);
}

#[test]
fn stores_reputation_snapshot() {
    let node = NodeId(vec![1]);

    let mut state = ReputationState::new(0);

    state.set_reputation(node.clone(), 42);

    assert_eq!(state.reputation_list().entries.len(), 1);
}

#[test]
fn exports_reputation_weights() {
    let validator = NodeId(vec![1]);

    let mut state = ReputationState::new(0);

    state.set_reputation(validator.clone(), 42);

    let weights = reputation_weights(&state);

    assert_eq!(weights.get(&validator), Some(&42));
}

#[test]
fn default_config_has_positive_initial_reputation() {
    let config = PorConfig::default();

    assert!(config.scale > 0);

    assert!(config.initial_reputation > 0);

    assert!(config.initial_reputation <= config.scale);
}
