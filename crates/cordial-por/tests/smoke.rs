use cordial_miners_core::NodeId;
use cordial_por::{PorConfig, ReputationState, reputation_weights};

#[test]
fn exports_reputation_weights_for_core_node_ids() {
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
