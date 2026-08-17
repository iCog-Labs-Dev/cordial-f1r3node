use cordial_por::{clamp_reputation_value, clamp_reputation_vector, PorConfig, PorError, ReputationEntry, ReputationVector};
use cordial_miners_core::NodeId;

// This integration test verifies that intermediate u128 arithmetic overflow
// is detected and returns Err(PorError::ClampOverflow). It uses the maximum
// u64 values for both scale and value. Individually scale^2 and value^2 fit
// into u128, but their sum overflows u128, which should trigger ClampOverflow.

#[test]
fn overflow_on_intermediate_addition_returns_error() {
    let big = std::u64::MAX;
    // direct scalar clamp should return Err(PorError::ClampOverflow)
    match clamp_reputation_value(big, big) {
        Err(PorError::ClampOverflow) => {}
        other => panic!("expected ClampOverflow, got {other:?}"),
    }

    // vector-level call should also return the same error and not panic
    let cfg = PorConfig::new(big, big);
    let entry = ReputationEntry::new(NodeId(b"x".to_vec()), big);
    let rv = ReputationVector { round: 1, values: vec![entry] };
    match clamp_reputation_vector(&rv, &cfg) {
        Err(PorError::ClampOverflow) => {}
        other => panic!("expected ClampOverflow from vector clamp, got {other:?}"),
    }
}
