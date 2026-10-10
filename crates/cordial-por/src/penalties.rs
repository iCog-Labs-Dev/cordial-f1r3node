//! Deterministic fixed-point penalty arithmetic. These helpers do not mutate
//! state or authenticate evidence. Audited transitions handle key ejection;
//! retained-capital transfer remains a separate authenticated host operation.

use crate::{config::PorConfig, error::PorError, types::ReputationWeight};

/// Return the base penalty at or below the correlation threshold, otherwise
/// a full slash. Zero total weight and invalid fractions return
/// `PorError::InvalidConfiguration` (the crate's configuration-error variant).
pub fn compute_slash_penalty(
    equivocating_weight: ReputationWeight,
    total_weight: ReputationWeight,
    config: &PorConfig,
) -> Result<ReputationWeight, PorError> {
    compute_slash_penalty_wide(
        u128::from(equivocating_weight),
        u128::from(total_weight),
        config,
    )
}

pub(crate) fn compute_slash_penalty_wide(
    equivocating_weight: u128,
    total_weight: u128,
    config: &PorConfig,
) -> Result<ReputationWeight, PorError> {
    config.validate()?;
    if total_weight == 0 || equivocating_weight > total_weight {
        return Err(PorError::InvalidConfiguration(
            "slash weights must satisfy equivocating weight <= total weight and total weight > 0"
                .into(),
        ));
    }
    // Compare exact ratios. Dividing first would round an above-threshold
    // event down into the base tier.
    let equivocating = equivocating_weight
        .checked_mul(u128::from(config.scale))
        .ok_or(PorError::SlashOverflow)?;
    let threshold = total_weight
        .checked_mul(u128::from(config.correlation_threshold))
        .ok_or(PorError::SlashOverflow)?;
    Ok(if equivocating <= threshold {
        config.base_slash_penalty
    } else {
        config.scale
    })
}

/// Apply `weight * (scale - penalty_ratio) / scale` with checked intermediates.
/// Reject fractions above scale instead of silently treating invalid input as
/// a full slash.
pub fn apply_slash_to_reputation(
    weight: ReputationWeight,
    penalty_ratio: ReputationWeight,
    config: &PorConfig,
) -> Result<ReputationWeight, PorError> {
    config.validate()?;
    let survival = config.scale.checked_sub(penalty_ratio).ok_or_else(|| {
        PorError::InvalidConfiguration("slash penalty must not exceed scale".into())
    })?;
    let value = u128::from(weight)
        .checked_mul(u128::from(survival))
        .ok_or(PorError::SlashOverflow)?
        / u128::from(config.scale);
    ReputationWeight::try_from(value).map_err(|_| PorError::SlashOverflow)
}

/// Apply one round of `weight * (scale - gamma) / scale`.
/// Gamma is a fixed-point fraction; zero disables decay and scale means 100%.
pub fn compute_inactivity_decay(
    weight: ReputationWeight,
    gamma: u64,
    config: &PorConfig,
) -> Result<ReputationWeight, PorError> {
    config.validate()?;
    let survival = config.scale.checked_sub(gamma).ok_or_else(|| {
        PorError::InvalidConfiguration("inactivity gamma must not exceed scale".into())
    })?;
    let value = u128::from(weight)
        .checked_mul(u128::from(survival))
        .ok_or(PorError::InactivityDecayOverflow)?
        / u128::from(config.scale);
    ReputationWeight::try_from(value).map_err(|_| PorError::InactivityDecayOverflow)
}

/// Convenience wrapper using the configured inactivity gamma.
pub fn apply_inactivity_decay(
    weight: ReputationWeight,
    config: &PorConfig,
) -> Result<ReputationWeight, PorError> {
    compute_inactivity_decay(weight, config.inactivity_decay_gamma, config)
}
