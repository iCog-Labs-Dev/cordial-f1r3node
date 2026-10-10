use cordial_por::{
    PorConfig, PorError, apply_inactivity_decay, apply_slash_to_reputation, compute_slash_penalty,
};

// ============================================================
// Helpers
// ============================================================

fn cfg() -> PorConfig {
    PorConfig::default()
}

/// Build a config with all slashing/decay fields set to custom values but
/// keep the rest at defaults so we can test boundary conditions precisely.
fn cfg_custom(
    scale: u64,
    correlation_threshold: u64,
    base_slash_penalty: u64,
    inactivity_decay_gamma: u64,
) -> PorConfig {
    PorConfig {
        scale,
        initial_reputation: scale / 5,
        liquid_rank_alpha: PorConfig::new(scale, 0).liquid_rank_alpha,
        minimum_rating: 0,
        maximum_rating: scale,
        missing_entry_policy: cordial_por::MissingEntryPolicy::CarryForward,
        correlation_threshold,
        base_slash_penalty,
        inactivity_decay_gamma,
    }
}

// ============================================================
// compute_slash_penalty — step-function tier tests
// ============================================================

#[test]
fn isolated_fault_returns_base_penalty() {
    // 1 unit out of 10 total → 10% correlated ratio, well below 30% threshold
    let penalty = compute_slash_penalty(1, 10, &cfg()).unwrap();
    assert_eq!(penalty, PorConfig::DEFAULT_BASE_SLASH_PENALTY);
}

#[test]
fn coordinated_attack_returns_full_slash() {
    // 4 units out of 10 total → 40% correlated ratio, above 30% threshold
    let penalty = compute_slash_penalty(4, 10, &cfg()).unwrap();
    assert_eq!(penalty, PorConfig::DEFAULT_SCALE);
}

#[test]
fn at_exact_threshold_returns_base_penalty() {
    // Exactly 30% → should take the base penalty (≤ threshold, not >)
    let scale = 1_000;
    let config = cfg_custom(scale, 300, 250, 0);
    // equivocating = 3, total = 10 → ratio = 300/1000 = 300 == threshold
    let penalty = compute_slash_penalty(3, 10, &config).unwrap();
    assert_eq!(penalty, 250);
}

#[test]
fn one_above_threshold_returns_full_slash() {
    // ratio just above 30% → full slash
    let scale = 1_000;
    let config = cfg_custom(scale, 300, 250, 0);
    // equivocating = 31, total = 100 → ratio = 310/1000 = 310 > 300
    let penalty = compute_slash_penalty(31, 100, &config).unwrap();
    assert_eq!(penalty, scale);
}

#[test]
fn zero_equivocating_weight_returns_base_penalty() {
    // No one is actually equivocating — ratio is 0, below threshold
    let penalty = compute_slash_penalty(0, 1_000, &cfg()).unwrap();
    assert_eq!(penalty, PorConfig::DEFAULT_BASE_SLASH_PENALTY);
}

#[test]
fn full_validator_set_equivocating_returns_full_slash() {
    // 100% equivocating
    let penalty = compute_slash_penalty(1_000, 1_000, &cfg()).unwrap();
    assert_eq!(penalty, PorConfig::DEFAULT_SCALE);
}

#[test]
fn zero_total_weight_returns_invalid_slash_input() {
    assert!(matches!(
        compute_slash_penalty(0, 0, &cfg()),
        Err(PorError::InvalidConfiguration(_))
    ));
}

// ============================================================
// apply_slash_to_reputation — penalty application tests
// ============================================================

#[test]
fn base_penalty_reduces_weight_by_25_percent() {
    let config = cfg();
    // weight = 1_000_000_000, penalty = 25% → new = 750_000_000
    let new_weight =
        apply_slash_to_reputation(1_000_000_000, config.base_slash_penalty, &config).unwrap();
    assert_eq!(new_weight, 750_000_000);
}

#[test]
fn full_penalty_reduces_weight_to_zero() {
    let config = cfg();
    let new_weight = apply_slash_to_reputation(1_000_000_000, config.scale, &config).unwrap();
    assert_eq!(new_weight, 0);
}

#[test]
fn zero_penalty_ratio_leaves_weight_unchanged() {
    let config = cfg();
    let new_weight = apply_slash_to_reputation(500_000_000, 0, &config).unwrap();
    assert_eq!(new_weight, 500_000_000);
}

#[test]
fn penalty_exceeding_scale_is_rejected() {
    let config = cfg();
    assert!(matches!(
        apply_slash_to_reputation(1_000_000_000, config.scale + 1, &config),
        Err(PorError::InvalidConfiguration(_))
    ));
}

#[test]
fn slash_on_zero_weight_stays_zero() {
    let config = cfg();
    let new_weight = apply_slash_to_reputation(0, config.base_slash_penalty, &config).unwrap();
    assert_eq!(new_weight, 0);
}

// ============================================================
// apply_inactivity_decay — per-round decay tests
// ============================================================

#[test]
fn inactivity_decay_reduces_weight_by_gamma() {
    // Default gamma = 1% → 1_000_000_000 * 0.99 = 990_000_000
    let new_weight = apply_inactivity_decay(1_000_000_000, &cfg()).unwrap();
    assert_eq!(new_weight, 990_000_000);
}

#[test]
fn zero_gamma_leaves_weight_unchanged() {
    let config = cfg_custom(1_000, 300, 250, 0);
    let new_weight = apply_inactivity_decay(800, &config).unwrap();
    assert_eq!(new_weight, 800);
}

#[test]
fn full_gamma_reduces_weight_to_zero_in_one_round() {
    // gamma = scale → survival = 0 → weight = 0
    let scale = 1_000;
    let config = cfg_custom(scale, 300, 250, scale);
    let new_weight = apply_inactivity_decay(1_000, &config).unwrap();
    assert_eq!(new_weight, 0);
}

#[test]
fn inactivity_decay_on_zero_weight_stays_zero() {
    let new_weight = apply_inactivity_decay(0, &cfg()).unwrap();
    assert_eq!(new_weight, 0);
}

#[test]
fn two_consecutive_decay_rounds_compound_correctly() {
    // 1_000_000_000 * 0.99 = 990_000_000, then * 0.99 = 980_100_000
    let after_round_1 = apply_inactivity_decay(1_000_000_000, &cfg()).unwrap();
    let after_round_2 = apply_inactivity_decay(after_round_1, &cfg()).unwrap();
    assert_eq!(after_round_1, 990_000_000);
    assert_eq!(after_round_2, 980_100_000);
}

// ============================================================
// End-to-end: full isolated slash lifecycle
// ============================================================

#[test]
fn isolated_slash_lifecycle_25_percent() {
    let config = cfg();
    let initial_weight = 1_000_000_000_u64;

    // 1 validator out of 4 equivocates. Equal weights → 25% equivocating.
    // 25% < 30% threshold → base penalty.
    let total_weight = initial_weight * 4;
    let equivocating_weight = initial_weight;

    let penalty = compute_slash_penalty(equivocating_weight, total_weight, &config).unwrap();
    assert_eq!(penalty, config.base_slash_penalty); // 25%

    let new_weight = apply_slash_to_reputation(initial_weight, penalty, &config).unwrap();
    assert_eq!(new_weight, 750_000_000); // 75% survives
}

// ============================================================
// End-to-end: full coordinated attack lifecycle
// ============================================================

#[test]
fn coordinated_slash_lifecycle_100_percent() {
    let config = cfg();
    let initial_weight = 1_000_000_000_u64;

    // 3 out of 4 equal-weight validators equivocate → 75% equivocating.
    let total_weight = initial_weight * 4;
    let equivocating_weight = initial_weight * 3;

    let penalty = compute_slash_penalty(equivocating_weight, total_weight, &config).unwrap();
    assert_eq!(penalty, config.scale); // 100%

    let new_weight = apply_slash_to_reputation(initial_weight, penalty, &config).unwrap();
    assert_eq!(new_weight, 0); // fully slashed
}

#[test]
fn fractional_unit_above_threshold_is_a_full_slash() {
    let config = cfg();
    assert_eq!(
        compute_slash_penalty(3_000_000_001, 10_000_000_000, &config),
        Ok(config.scale)
    );
    assert_eq!(
        compute_slash_penalty(3_000_000_000, 10_000_000_000, &config),
        Ok(config.base_slash_penalty)
    );
}

#[test]
fn constructor_scales_all_fixed_point_defaults() {
    let config = PorConfig::new(1000, 200);
    assert_eq!(
        (
            config.correlation_threshold,
            config.base_slash_penalty,
            config.inactivity_decay_gamma
        ),
        (300, 250, 10)
    );
    let penalty = compute_slash_penalty(1, 10, &config).unwrap();
    assert_eq!(apply_slash_to_reputation(1000, penalty, &config), Ok(750));
    assert_eq!(apply_inactivity_decay(1000, &config), Ok(990));
    assert_eq!(compute_slash_penalty(4, 10, &config), Ok(1000));
}

#[test]
fn maximum_values_use_wide_intermediates() {
    let config = PorConfig::new(u64::MAX, 0);
    assert_eq!(
        config.liquid_rank_alpha,
        ((u128::from(u64::MAX) * 3) / 5) as u64
    );
    assert_eq!(
        compute_slash_penalty(u64::MAX, u64::MAX, &config),
        Ok(u64::MAX)
    );
    assert_eq!(
        apply_slash_to_reputation(u64::MAX, 0, &config),
        Ok(u64::MAX)
    );
    assert_eq!(
        cordial_por::compute_inactivity_decay(u64::MAX, 0, &config),
        Ok(u64::MAX)
    );
}

#[test]
fn invalid_scale_fractions_and_weights_are_rejected() {
    assert!(matches!(
        compute_slash_penalty(1, 1, &PorConfig::new(0, 0)),
        Err(PorError::InvalidConfiguration(_))
    ));
    assert!(matches!(
        compute_slash_penalty(2, 1, &cfg()),
        Err(PorError::InvalidConfiguration(_))
    ));
    assert!(matches!(
        cordial_por::compute_inactivity_decay(100, cfg().scale + 1, &cfg()),
        Err(PorError::InvalidConfiguration(_))
    ));
    for field in 0..3 {
        let mut config = cfg();
        match field {
            0 => config.correlation_threshold = config.scale + 1,
            1 => config.base_slash_penalty = config.scale + 1,
            _ => config.inactivity_decay_gamma = config.scale + 1,
        }
        assert!(matches!(
            compute_slash_penalty(1, 10, &config),
            Err(PorError::InvalidConfiguration(_))
        ));
        assert!(matches!(
            apply_inactivity_decay(100, &config),
            Err(PorError::InvalidConfiguration(_))
        ));
    }
}
