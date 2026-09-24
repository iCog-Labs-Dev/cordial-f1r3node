use cordial_f1r3node_adapter::{
    por_rating_wire::{
        BLOCK_PRODUCTION_RATING_ENVELOPE_DOMAIN, BLOCK_PRODUCTION_RATING_ENVELOPE_VERSION,
        BlockProductionRatingEnvelopeV1, MAX_BLOCK_PRODUCTION_RATING_ENVELOPE_LEN,
        PorRatingWireError,
    },
    por_ratings::sign_admitted_interaction,
};
use cordial_miners_core::NodeId;
use cordial_por::{
    InteractionEvidence, InteractionKind, PorConfig, RatingRecord, ReputationState,
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

fn rating() -> RatingRecord {
    let rater = node(1);
    let recipient = node(2);
    let mut state = ReputationState::new(FINALIZED_WAVE);
    state.set_reputation(rater.clone(), 100);
    state.set_reputation(recipient.clone(), 100);
    let admitted = admit_interaction_evidence(
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
    .unwrap();

    sign_admitted_interaction(admitted, &PorConfig::default(), &private_key(1)).unwrap()
}

#[test]
fn encodes_the_documented_v1_layout() {
    let rating = rating();
    let envelope = BlockProductionRatingEnvelopeV1::new(FINALIZED_WAVE, rating.clone()).unwrap();
    let mut expected = BLOCK_PRODUCTION_RATING_ENVELOPE_DOMAIN.to_vec();
    expected.extend_from_slice(&BLOCK_PRODUCTION_RATING_ENVELOPE_VERSION.to_be_bytes());
    expected.extend_from_slice(&FINALIZED_WAVE.to_be_bytes());
    expected.extend_from_slice(&rating.round.to_be_bytes());
    expected.extend_from_slice(&(rating.rater.0.len() as u16).to_be_bytes());
    expected.extend_from_slice(&rating.rater.0);
    expected.extend_from_slice(&(rating.recipient.0.len() as u16).to_be_bytes());
    expected.extend_from_slice(&rating.recipient.0);
    expected.extend_from_slice(&rating.score.to_be_bytes());
    expected.extend_from_slice(rating.interaction_ref.as_ref().unwrap());
    expected.extend_from_slice(&(rating.signature.len() as u16).to_be_bytes());
    expected.extend_from_slice(&rating.signature);

    assert_eq!(envelope.encode(), expected);
}

#[test]
fn round_trip_is_deterministic_and_lossless() {
    let rating = rating();
    let envelope = BlockProductionRatingEnvelopeV1::new(FINALIZED_WAVE, rating.clone()).unwrap();
    let first = envelope.encode();
    let second = envelope.encode();
    let decoded = BlockProductionRatingEnvelopeV1::decode(&first).unwrap();

    assert_eq!(first, second);
    assert_eq!(decoded.finalized_wave(), FINALIZED_WAVE);
    assert_eq!(decoded.rating(), &rating);
    assert_eq!(decoded.encode(), first);
}

#[test]
fn rejects_wrong_domain_version_truncation_and_trailing_bytes() {
    let bytes = BlockProductionRatingEnvelopeV1::new(FINALIZED_WAVE, rating())
        .unwrap()
        .encode();

    let mut wrong_domain = bytes.clone();
    wrong_domain[0] ^= 0xff;
    assert_eq!(
        BlockProductionRatingEnvelopeV1::decode(&wrong_domain),
        Err(PorRatingWireError::InvalidDomain)
    );

    let mut wrong_version = bytes.clone();
    let version_offset = BLOCK_PRODUCTION_RATING_ENVELOPE_DOMAIN.len();
    wrong_version[version_offset..version_offset + 2].copy_from_slice(&2_u16.to_be_bytes());
    assert_eq!(
        BlockProductionRatingEnvelopeV1::decode(&wrong_version),
        Err(PorRatingWireError::UnsupportedVersion(2))
    );

    let mut truncated = bytes.clone();
    truncated.pop();
    assert_eq!(
        BlockProductionRatingEnvelopeV1::decode(&truncated),
        Err(PorRatingWireError::UnexpectedEnd)
    );

    let mut trailing = bytes;
    trailing.push(0);
    assert_eq!(
        BlockProductionRatingEnvelopeV1::decode(&trailing),
        Err(PorRatingWireError::TrailingBytes)
    );
}

#[test]
fn rejects_an_envelope_over_the_v1_size_bound() {
    let oversized = vec![0; MAX_BLOCK_PRODUCTION_RATING_ENVELOPE_LEN + 1];

    assert_eq!(
        BlockProductionRatingEnvelopeV1::decode(&oversized),
        Err(PorRatingWireError::EnvelopeTooLong {
            actual: oversized.len(),
            maximum: MAX_BLOCK_PRODUCTION_RATING_ENVELOPE_LEN,
        })
    );
}

#[test]
fn rejects_a_round_not_derived_from_the_finalized_wave() {
    assert_eq!(
        BlockProductionRatingEnvelopeV1::new(FINALIZED_WAVE + 1, rating()),
        Err(PorRatingWireError::FinalizedWaveRoundMismatch)
    );

    let mut maximum_round = rating();
    maximum_round.round = u64::MAX;
    assert_eq!(
        BlockProductionRatingEnvelopeV1::new(u64::MAX, maximum_round),
        Err(PorRatingWireError::FinalizedWaveRoundMismatch)
    );
}

#[test]
fn rejects_non_secp256k1_validator_key_lengths() {
    let mut invalid_rater = rating();
    invalid_rater.rater = NodeId(vec![1; 32]);
    assert_eq!(
        BlockProductionRatingEnvelopeV1::new(FINALIZED_WAVE, invalid_rater),
        Err(PorRatingWireError::InvalidRaterKeyLength(32))
    );

    let mut invalid_recipient = rating();
    invalid_recipient.recipient = NodeId(vec![2; 64]);
    assert_eq!(
        BlockProductionRatingEnvelopeV1::new(FINALIZED_WAVE, invalid_recipient),
        Err(PorRatingWireError::InvalidRecipientKeyLength(64))
    );
}

#[test]
fn rejects_missing_or_non_block_hash_interaction_references() {
    let mut missing = rating();
    missing.interaction_ref = None;
    assert_eq!(
        BlockProductionRatingEnvelopeV1::new(FINALIZED_WAVE, missing),
        Err(PorRatingWireError::MissingInteractionReference)
    );

    let mut wrong_length = rating();
    wrong_length.interaction_ref = Some(vec![0x42; 31]);
    assert_eq!(
        BlockProductionRatingEnvelopeV1::new(FINALIZED_WAVE, wrong_length),
        Err(PorRatingWireError::InvalidInteractionReferenceLength(31))
    );
}

#[test]
fn rejects_missing_or_oversized_signatures() {
    let mut missing = rating();
    missing.signature.clear();
    assert_eq!(
        BlockProductionRatingEnvelopeV1::new(FINALIZED_WAVE, missing),
        Err(PorRatingWireError::MissingSignature)
    );

    let mut oversized = rating();
    oversized.signature = vec![1; 73];
    assert_eq!(
        BlockProductionRatingEnvelopeV1::new(FINALIZED_WAVE, oversized),
        Err(PorRatingWireError::SignatureTooLong {
            actual: 73,
            maximum: 72,
        })
    );
}
