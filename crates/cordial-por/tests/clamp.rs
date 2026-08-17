use cordial_miners_core::NodeId;
use cordial_por::{
    PorConfig, PorError, ReputationEntry, ReputationVector, clamp_reputation_value,
    clamp_reputation_vector,
};

// This integration test file consolidates all clamp-related tests which were
// previously embedded in the library source. It preserves coverage for:
// - zero value
// - small value (regression constant)
// - value equal to scale
// - value greater than scale
// - monotonic behavior
// - zero scale error
// - vector behavior (preserve round, ordering and clamped values)
// - explicit overflow policy when intermediate arithmetic overflows

const S: u64 = PorConfig::DEFAULT_SCALE;

#[test]
fn clamp_zero_value() {
    assert_eq!(clamp_reputation_value(0, S).unwrap(), 0);
}

#[test]
fn clamp_small_value_behavior() {
    // 0.2 * scale
    let v = 200_000_000u64;
    let out = clamp_reputation_value(v, S).unwrap();
    // independent regression constant for scale = 1_000_000_000:
    let expected = 196_116_135u64;
    assert_eq!(
        out, expected,
        "small-value clamp should match regression constant"
    );
}

#[test]
fn clamp_equal_scale() {
    let out = clamp_reputation_value(S, S).unwrap();
    // Known constant: round(1/sqrt(2) * S) = 707_106_781
    assert_eq!(out, 707_106_781u64);
}

#[test]
fn clamp_greater_than_scale() {
    let two_s = S.saturating_mul(2);
    let out = clamp_reputation_value(two_s, S).unwrap();
    // Known constant for 2.0: round(2 / sqrt(5) * S) = 894_427_191
    assert_eq!(out, 894_427_191u64);
}

#[test]
fn clamp_monotonic() {
    let a = 0u64;
    let b = S / 2;
    let c = S;
    let d = S.saturating_mul(2);
    let va = clamp_reputation_value(a, S).unwrap();
    let vb = clamp_reputation_value(b, S).unwrap();
    let vc = clamp_reputation_value(c, S).unwrap();
    let vd = clamp_reputation_value(d, S).unwrap();
    assert!(va <= vb && vb <= vc && vc <= vd);
}

#[test]
fn clamp_zero_scale_error() {
    let cfg = PorConfig::new(0, PorConfig::DEFAULT_INITIAL_REPUTATION);
    let rv = ReputationVector {
        round: 1,
        values: vec![],
    };
    match clamp_reputation_vector(&rv, &cfg) {
        Err(PorError::InvalidClampScale) => {}
        other => panic!("expected InvalidClampScale, got {other:?}"),
    }
}

#[test]
fn clamp_vector_preserves_order_and_round_and_values() {
    let cfg = PorConfig::default();
    let entries = vec![
        ReputationEntry::new(NodeId(b"a".to_vec()), 0),
        ReputationEntry::new(NodeId(b"b".to_vec()), S),
        ReputationEntry::new(NodeId(b"c".to_vec()), S.saturating_mul(2)),
    ];
    let rv = ReputationVector {
        round: 42,
        values: entries.clone(),
    };
    let out = clamp_reputation_vector(&rv, &cfg).unwrap();
    assert_eq!(out.round, 42);
    assert_eq!(out.values.len(), entries.len());
    for (i, e) in out.values.iter().enumerate() {
        assert_eq!(e.node_id, entries[i].node_id);
    }
    // verify reputations were actually clamped to expected known constants
    assert_eq!(out.values[0].reputation, 0);
    assert_eq!(out.values[1].reputation, 707_106_781u64);
    assert_eq!(out.values[2].reputation, 894_427_191u64);
}

// Additional boundary tests to exercise the integer-sqrt code paths indirectly
// (without accessing private functions). Use small integers to construct
// perfect-square sums so sqrt correctness and rounding can be validated.
#[test]
fn clamp_perfect_square_sum_behavior() {
    // choose s=3, r=4 => s^2 + r^2 = 9 + 16 = 25, sqrt = 5
    // numerator = r*s = 12, result = round(12/5) = (12 + 2)/5 = 14/5 = 2
    let out = clamp_reputation_value(4, 3).unwrap();
    assert_eq!(out, 2u64);
}

// Preserve the explicit overflow test (already present)
#[test]
fn overflow_on_intermediate_addition_returns_error() {
    let big = u64::MAX;
    // direct scalar clamp should return Err(PorError::ClampOverflow)
    match clamp_reputation_value(big, big) {
        Err(PorError::ClampOverflow) => {}
        other => panic!("expected ClampOverflow, got {other:?}"),
    }

    // vector-level call should also return the same error and not panic
    let cfg = PorConfig::new(big, big);
    let entry = ReputationEntry::new(NodeId(b"x".to_vec()), big);
    let rv = ReputationVector {
        round: 1,
        values: vec![entry],
    };
    match clamp_reputation_vector(&rv, &cfg) {
        Err(PorError::ClampOverflow) => {}
        other => panic!("expected ClampOverflow from vector clamp, got {other:?}"),
    }
}
