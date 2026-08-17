//! Deterministic fixed-point reputation clamping.
//!
//! Implements the paper sigmoid-style clamp:
//!
//! R_clamped = R / sqrt(1 + R^2)
//!
//! with all values represented as fixed-point integers using the crate's
//! `scale` (e.g. 1.0 -> 1_000_000_000). No floating-point arithmetic is used.
//!
//! Overflow and error policy (explicit):
//! - `scale == 0` => returns `Err(PorError::InvalidClampScale)`
//! - any intermediate arithmetic overflow (checked operations) => returns
//!   `Err(PorError::ClampOverflow)` (no panic)
//! - the implementation does not silently convert intermediate arithmetic
//!   overflow into a saturated value. A defensive final saturation on the
//!   conversion back to `u64` remains, but intermediate overflows are treated
//!   as errors per the policy above.

use crate::config::PorConfig;
use crate::error::PorError;
use crate::types::{ReputationEntry, ReputationVector, ReputationWeight};

/// Integer square root for u128 returning floor(sqrt(n)).
fn integer_sqrt_u128(n: u128) -> u128 {
    // Binary search over 0..=2^64 because (2^64)^2 = 2^128 which covers u128 range.
    let mut low: u128 = 0;
    let mut high: u128 = (1u128 << 64) as u128; // exclusive upper bound
    while low + 1 < high {
        let mid = (low + high) / 2;
        let mid_sq = mid.saturating_mul(mid);
        if mid_sq == n {
            return mid;
        }
        if mid_sq < n {
            low = mid;
        } else {
            high = mid;
        }
    }
    // ensure correct by adjusting low if needed
    while (low + 1).saturating_mul(low + 1) <= n {
        low = low + 1;
    }
    while low.saturating_mul(low) > n {
        low = low - 1;
    }
    low
}

/// Clamp single fixed-point reputation value using integer-only arithmetic.
///
/// Formula (fixed-point derivation):
/// clamp_fixed = round( (r * S) / sqrt(S^2 + r^2) )
/// where `r` is the fixed-point reputation and `S` is the fixed-point scale.
pub fn clamp_reputation_value(
    value: ReputationWeight,
    scale: ReputationWeight,
) -> Result<ReputationWeight, PorError> {
    if scale == 0 {
        return Err(PorError::InvalidClampScale);
    }
    if value == 0 {
        return Ok(0);
    }

    let r = value as u128;
    let s = scale as u128;

    // Compute s^2 and r^2 with checked/saturating arithmetic using u128
    let s2 = s.checked_mul(s).ok_or(PorError::ClampOverflow)?;
    let r2 = r.checked_mul(r).ok_or(PorError::ClampOverflow)?;

    let sum = s2.checked_add(r2).ok_or(PorError::ClampOverflow)?;

    // denom = sqrt(s^2 + r^2)
    let denom = integer_sqrt_u128(sum);
    if denom == 0 {
        return Err(PorError::ClampOverflow);
    }

    // numerator = r * s
    let numerator = r.checked_mul(s).ok_or(PorError::ClampOverflow)?;

    // rounding: (numerator + denom/2) / denom
    let half = denom / 2;
    let numerator_rounded = numerator.checked_add(half).ok_or(PorError::ClampOverflow)?;

    let result = numerator_rounded / denom;

    // result should be <= scale, but be defensive and saturate to u64::MAX if needed
    let result_u64 = if result > (ReputationWeight::MAX as u128) {
        ReputationWeight::MAX
    } else {
        result as ReputationWeight
    };

    Ok(result_u64)
}

/// Clamp an entire ReputationVector, preserving round and entry ordering.
///
/// Does not mutate the input and returns a new ReputationVector with clamped
/// reputation values. Uses PorConfig.scale for the fixed-point scale.
pub fn clamp_reputation_vector(
    reputation: &ReputationVector,
    config: &PorConfig,
) -> Result<ReputationVector, PorError> {
    let scale = config.scale;
    if scale == 0 {
        return Err(PorError::InvalidClampScale);
    }

    let mut values = Vec::with_capacity(reputation.values.len());
    for entry in &reputation.values {
        let clamped = clamp_reputation_value(entry.reputation, scale)?;
        values.push(ReputationEntry::new(entry.node_id.clone(), clamped));
    }

    Ok(ReputationVector {
        round: reputation.round,
        values,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::PorConfig;
    use crate::error::PorError;
    use crate::types::{ReputationEntry, ReputationVector};
    use cordial_miners_core::NodeId;

    const S: ReputationWeight = PorConfig::DEFAULT_SCALE;

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
        // expected approximate clamp(0.2) = 196_116_135
        let expected = 196_116_135u64;
        assert_eq!(out, expected, "small-value clamp should match regression constant");
    }

    #[test]
    fn clamp_equal_scale() {
        // r = S -> expected floor(round(S*S / sqrt(2*S^2))) = round(S / sqrt(2))
        let out = clamp_reputation_value(S, S).unwrap();
        // Known constant: floor(round(1/sqrt(2) * S)) = 707_106_781
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
    fn clamp_vector_preserves_order_and_round() {
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
        // for scale = 1_000_000_000: 0 -> 0, 1.0 -> 707_106_781, 2.0 -> 894_427_191
        assert_eq!(out.values[0].reputation, 0);
        assert_eq!(out.values[1].reputation, 707_106_781u64);
        assert_eq!(out.values[2].reputation, 894_427_191u64);
    }
}
