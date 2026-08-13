use cordial_miners_core::NodeId;
use cordial_por::{
    PorConfig, PorError, ReputationEntry, ReputationVector, blend_reputation_transition,
};

fn cfg(scale: u64, alpha: u64) -> PorConfig {
    PorConfig {
        scale,
        initial_reputation: 0,
        liquid_rank_alpha: alpha,
        minimum_rating: 0,
        maximum_rating: scale,
    }
}

fn entry(node: u8, reputation: u64) -> ReputationEntry {
    ReputationEntry::new(NodeId(vec![node]), reputation)
}

fn vector(round: u64, values: Vec<ReputationEntry>) -> ReputationVector {
    ReputationVector { round, values }
}

#[test]
fn blends_contribution_with_previous_reputation() {
    let contribution = vector(7, vec![entry(1, 90), entry(2, 20)]);
    let previous = vector(6, vec![entry(1, 50), entry(2, 80)]);

    let next = blend_reputation_transition(&contribution, &previous, &cfg(100, 60)).unwrap();

    assert_eq!(next.round, 7);
    assert_eq!(next.values, vec![entry(1, 74), entry(2, 44)]);
}

#[test]
fn alpha_zero_preserves_previous_reputation() {
    let contribution = vector(7, vec![entry(1, 90)]);
    let previous = vector(6, vec![entry(1, 50)]);

    let next = blend_reputation_transition(&contribution, &previous, &cfg(100, 0)).unwrap();

    assert_eq!(next.values, vec![entry(1, 50)]);
}

#[test]
fn alpha_equal_to_scale_uses_only_contribution() {
    let contribution = vector(7, vec![entry(1, 90)]);
    let previous = vector(6, vec![entry(1, 50)]);

    let next = blend_reputation_transition(&contribution, &previous, &cfg(100, 100)).unwrap();

    assert_eq!(next.values, vec![entry(1, 90)]);
}

#[test]
fn missing_previous_reputation_is_rejected() {
    let contribution = vector(7, vec![entry(1, 90), entry(2, 20)]);
    let previous = vector(6, vec![entry(1, 50)]);

    assert_eq!(
        blend_reputation_transition(&contribution, &previous, &cfg(100, 60)),
        Err(PorError::MissingPreviousReputation)
    );
}

#[test]
fn alpha_greater_than_scale_is_rejected() {
    let contribution = vector(7, vec![entry(1, 90)]);
    let previous = vector(6, vec![entry(1, 50)]);

    assert_eq!(
        blend_reputation_transition(&contribution, &previous, &cfg(100, 101)),
        Err(PorError::InvalidLiquidRankAlpha)
    );
}

#[test]
fn zero_scale_is_rejected() {
    let contribution = vector(7, vec![entry(1, 90)]);
    let previous = vector(6, vec![entry(1, 50)]);

    assert_eq!(
        blend_reputation_transition(&contribution, &previous, &cfg(0, 0)),
        Err(PorError::InvalidTransitionScale)
    );
}

#[test]
fn arithmetic_overflow_is_rejected() {
    let contribution = vector(7, vec![entry(1, u64::MAX)]);
    let previous = vector(6, vec![entry(1, u64::MAX)]);

    assert_eq!(
        blend_reputation_transition(&contribution, &previous, &cfg(100, 60)),
        Err(PorError::ReputationTransitionOverflow)
    );
}

#[test]
fn preserves_canonical_contribution_order() {
    let contribution = vector(7, vec![entry(1, 90), entry(3, 40)]);
    let previous = vector(6, vec![entry(1, 50), entry(2, 70), entry(3, 20)]);

    let next = blend_reputation_transition(&contribution, &previous, &cfg(100, 50)).unwrap();

    assert_eq!(next.values, vec![entry(1, 70), entry(3, 30)]);
}

#[test]
fn unsorted_contribution_is_rejected() {
    let contribution = vector(7, vec![entry(3, 40), entry(1, 90)]);
    let previous = vector(6, vec![entry(1, 50), entry(3, 20)]);

    assert_eq!(
        blend_reputation_transition(&contribution, &previous, &cfg(100, 50)),
        Err(PorError::UnsortedReputationVector)
    );
}

#[test]
fn unsorted_previous_reputation_is_rejected() {
    let contribution = vector(7, vec![entry(1, 90), entry(3, 40)]);
    let previous = vector(6, vec![entry(3, 20), entry(1, 50)]);

    assert_eq!(
        blend_reputation_transition(&contribution, &previous, &cfg(100, 50)),
        Err(PorError::UnsortedReputationVector)
    );
}
