use cordial_f1r3node_adapter::por_ratings::{
    PorRatingError, build_verified_rating_batch, sign_admitted_interaction, validate_signed_rating,
    verify_rating_signature,
};
use cordial_miners_core::NodeId;
use cordial_por::{
    InteractionEvidence, InteractionKind, PorConfig, PorError, ReputationState,
    admit_interaction_evidence,
};
use k256::ecdsa::SigningKey;

const FINALIZED_WAVE: u64 = 4;
const RATING_ROUND: u64 = FINALIZED_WAVE + 1;

fn private_key(seed: u8) -> [u8; 32] {
    [seed; 32]
}

fn node(seed: u8) -> NodeId {
    let signing_key = SigningKey::from_slice(&private_key(seed)).unwrap();
    NodeId(
        signing_key
            .verifying_key()
            .to_encoded_point(true)
            .as_bytes()
            .to_vec(),
    )
}

fn admitted(rater_seed: u8, recipient_seed: u8) -> cordial_por::AdmittedInteraction {
    let rater = node(rater_seed);
    let recipient = node(recipient_seed);
    let mut state = ReputationState::new(FINALIZED_WAVE);
    state.set_reputation(rater.clone(), 100);
    state.set_reputation(recipient.clone(), 100);

    admit_interaction_evidence(
        InteractionEvidence {
            finalized_wave: FINALIZED_WAVE,
            round: RATING_ROUND,
            kind: InteractionKind::BlockProduction,
            rater,
            recipient,
            evidence_ref: vec![0x42; 32],
        },
        &state,
    )
    .unwrap()
}

#[test]
fn signs_and_verifies_an_admitted_interaction_with_the_rater_key() {
    let config = PorConfig::default();
    let rating = sign_admitted_interaction(admitted(1, 2), &config, &private_key(1)).unwrap();

    assert_eq!(rating.round, RATING_ROUND);
    assert_eq!(rating.rater, node(1));
    assert_eq!(rating.recipient, node(2));
    assert_eq!(rating.score, config.maximum_rating);
    assert_eq!(rating.interaction_ref, Some(vec![0x42; 32]));
    assert!(!rating.signature.is_empty());
    assert_eq!(validate_signed_rating(&rating, &config), Ok(()));
}

#[test]
fn signing_rejects_a_private_key_that_does_not_belong_to_the_rater() {
    assert_eq!(
        sign_admitted_interaction(admitted(1, 2), &PorConfig::default(), &private_key(3)),
        Err(PorRatingError::InvalidSignature)
    );
}

#[test]
fn signing_propagates_invalid_private_key_failures() {
    assert!(matches!(
        sign_admitted_interaction(admitted(1, 2), &PorConfig::default(), &[0; 31]),
        Err(PorRatingError::Signing(_))
    ));
}

#[test]
fn every_semantic_rating_field_is_covered_by_the_signature() {
    let rating =
        sign_admitted_interaction(admitted(1, 2), &PorConfig::default(), &private_key(1)).unwrap();

    let mut mutations = Vec::new();

    let mut changed = rating.clone();
    changed.round += 1;
    mutations.push(changed);

    let mut changed = rating.clone();
    changed.rater = node(3);
    mutations.push(changed);

    let mut changed = rating.clone();
    changed.recipient = node(3);
    mutations.push(changed);

    let mut changed = rating.clone();
    changed.score -= 1;
    mutations.push(changed);

    let mut changed = rating.clone();
    changed.interaction_ref.as_mut().unwrap()[0] ^= 0xff;
    mutations.push(changed);

    for changed in mutations {
        assert_eq!(
            verify_rating_signature(&changed),
            Err(PorRatingError::InvalidSignature)
        );
    }
}

#[test]
fn rejects_missing_and_malformed_signature_inputs() {
    let config = PorConfig::default();
    let mut rating = sign_admitted_interaction(admitted(1, 2), &config, &private_key(1)).unwrap();

    rating.signature.clear();
    assert_eq!(
        validate_signed_rating(&rating, &config),
        Err(PorRatingError::Protocol(PorError::MissingRatingSignature))
    );

    rating.signature = vec![1, 2, 3];
    assert_eq!(
        verify_rating_signature(&rating),
        Err(PorRatingError::InvalidSignature)
    );
}

#[test]
fn rejects_a_rating_without_an_interaction_reference() {
    let mut rating =
        sign_admitted_interaction(admitted(1, 2), &PorConfig::default(), &private_key(1)).unwrap();
    rating.interaction_ref = None;

    assert_eq!(
        verify_rating_signature(&rating),
        Err(PorRatingError::Protocol(
            PorError::MissingInteractionReference
        ))
    );
}

#[test]
fn verified_batch_rejects_a_forged_rating_before_batching() {
    let config = PorConfig::default();
    let valid = sign_admitted_interaction(admitted(1, 2), &config, &private_key(1)).unwrap();
    let mut forged = sign_admitted_interaction(admitted(3, 2), &config, &private_key(3)).unwrap();
    forged.score -= 1;

    assert_eq!(
        build_verified_rating_batch(RATING_ROUND, vec![valid, forged], &config),
        Err(PorRatingError::InvalidSignature)
    );
}

#[test]
fn verified_batch_preserves_canonical_recipient_then_rater_order() {
    let config = PorConfig::default();
    let first = sign_admitted_interaction(admitted(1, 3), &config, &private_key(1)).unwrap();
    let second = sign_admitted_interaction(admitted(2, 3), &config, &private_key(2)).unwrap();
    let third = sign_admitted_interaction(admitted(3, 1), &config, &private_key(3)).unwrap();

    let mut expected = vec![first.clone(), second.clone(), third.clone()];
    expected.sort_by(|a, b| {
        a.recipient
            .cmp(&b.recipient)
            .then_with(|| a.rater.cmp(&b.rater))
    });

    let batch =
        build_verified_rating_batch(RATING_ROUND, vec![first, second, third], &config).unwrap();

    assert_eq!(batch.ratings, expected);
}
