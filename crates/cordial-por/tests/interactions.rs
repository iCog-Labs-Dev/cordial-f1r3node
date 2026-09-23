use cordial_miners_core::NodeId;
use cordial_por::{
    InteractionEvidence, InteractionKind, PorError, ReputationState, admit_interaction_evidence,
};

const FINALIZED_WAVE: u64 = 4;
const RATING_ROUND: u64 = 5;

fn node(id: u8) -> NodeId {
    NodeId(vec![id])
}

fn state() -> ReputationState {
    let mut state = ReputationState::new(FINALIZED_WAVE);
    state.set_reputation(node(1), 800);
    state.set_reputation(node(2), 700);
    state
}

fn evidence() -> InteractionEvidence {
    InteractionEvidence {
        finalized_wave: FINALIZED_WAVE,
        round: RATING_ROUND,
        kind: InteractionKind::BlockProduction,
        rater: node(1),
        recipient: node(2),
        evidence_ref: vec![9; 32],
    }
}

#[test]
fn admits_evidence_from_a_finalized_wave_for_known_active_validators() {
    let evidence = evidence();
    let admitted = admit_interaction_evidence(evidence.clone(), &state()).unwrap();

    assert_eq!(admitted.evidence(), &evidence);
    assert_eq!(admitted.into_evidence(), evidence);
}

#[test]
fn rejects_a_rating_round_not_derived_from_the_finalized_wave() {
    let mut evidence = evidence();
    evidence.round += 1;

    assert_eq!(
        admit_interaction_evidence(evidence, &state()),
        Err(PorError::InvalidInteractionRound)
    );
}

#[test]
fn rejects_evidence_when_state_is_not_the_preceding_round() {
    let stale_state = ReputationState::new(FINALIZED_WAVE - 1);

    assert_eq!(
        admit_interaction_evidence(evidence(), &stale_state),
        Err(PorError::InvalidInteractionStateRound)
    );
}

#[test]
fn rejects_self_interaction() {
    let mut evidence = evidence();
    evidence.recipient = evidence.rater.clone();

    assert_eq!(
        admit_interaction_evidence(evidence, &state()),
        Err(PorError::SelfInteraction)
    );
}

#[test]
fn rejects_empty_evidence_reference() {
    let mut evidence = evidence();
    evidence.evidence_ref.clear();

    assert_eq!(
        admit_interaction_evidence(evidence, &state()),
        Err(PorError::MissingInteractionReference)
    );
}

#[test]
fn rejects_unknown_rater() {
    let mut evidence = evidence();
    evidence.rater = node(3);

    assert_eq!(
        admit_interaction_evidence(evidence, &state()),
        Err(PorError::UnknownInteractionRater)
    );
}

#[test]
fn rejects_unknown_recipient() {
    let mut evidence = evidence();
    evidence.recipient = node(3);

    assert_eq!(
        admit_interaction_evidence(evidence, &state()),
        Err(PorError::UnknownInteractionRecipient)
    );
}

#[test]
fn rejects_ejected_rater() {
    let mut state = state();
    state.eject_validator(&node(1)).unwrap();

    assert_eq!(
        admit_interaction_evidence(evidence(), &state),
        Err(PorError::EjectedInteractionRater)
    );
}

#[test]
fn rejects_ejected_recipient() {
    let mut state = state();
    state.eject_validator(&node(2)).unwrap();

    assert_eq!(
        admit_interaction_evidence(evidence(), &state),
        Err(PorError::EjectedInteractionRecipient)
    );
}
