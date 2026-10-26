use std::collections::{BTreeMap, HashSet};

use cordial_f1r3node_adapter::{
    ordered_output::OrderedFinalizedOutput,
    por_finality::{FinalizedRatingRound, PorFinalityTracker},
    por_ratings::{
        PorRatingError, build_finalized_block_production_rating_batch,
        build_verified_checkpoint_rating_batch, build_verified_rating_batch,
        sign_admitted_interaction, sign_checkpoint_rating, validate_checkpoint_signed_rating,
        validate_signed_rating, verify_checkpoint_rating_signature, verify_rating_signature,
    },
};
use cordial_miners_core::{
    Block, BlockContent, BlockIdentity, Blocklace, NodeId, crypto::CryptoVerifier,
};
use cordial_por::{
    InteractionEvidence, InteractionKind, PorConfig, PorError, RatingRecord,
    RatingSourceCheckpoint, ReputationState, admit_interaction_evidence,
};
use k256::ecdsa::SigningKey;

const FINALIZED_WAVE: u64 = 4;
const RATING_ROUND: u64 = FINALIZED_WAVE + 1;
const WAVELENGTH: u64 = 3;

struct AcceptAll;

impl CryptoVerifier for AcceptAll {
    type Error = String;

    fn verify_block(
        &self,
        _content: &BlockContent,
        _signature: &[u8],
        _creator: &NodeId,
    ) -> Result<(), Self::Error> {
        Ok(())
    }
}

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

fn checkpoint_source() -> RatingSourceCheckpoint {
    RatingSourceCheckpoint {
        round: 1,
        finalized_prefix_len: 16,
        finalized_prefix_hash: [0x31; 32],
    }
}

fn unsigned_checkpoint_rating() -> RatingRecord {
    let mut rating = RatingRecord::new(1, node(1), node(2), 80, Vec::new());
    rating.interaction_ref = Some(vec![0x42; 32]);
    rating
}

#[test]
fn checkpoint_signature_binds_the_exact_source_and_rejects_v1_replay() {
    let source = checkpoint_source();
    let rating = sign_checkpoint_rating(
        unsigned_checkpoint_rating(),
        &source,
        &PorConfig::default(),
        &private_key(1),
    )
    .unwrap();
    assert_eq!(
        validate_checkpoint_signed_rating(&rating, &source, &PorConfig::default()),
        Ok(())
    );
    assert_eq!(
        verify_rating_signature(&rating),
        Err(PorRatingError::InvalidSignature)
    );
    let changed_source = RatingSourceCheckpoint {
        finalized_prefix_hash: [0x32; 32],
        ..source
    };
    assert_eq!(
        verify_checkpoint_rating_signature(&rating, &changed_source),
        Err(PorRatingError::InvalidSignature)
    );
    assert_eq!(
        sign_checkpoint_rating(rating, &source, &PorConfig::default(), &private_key(1)),
        Err(PorRatingError::RatingAlreadySigned)
    );
}

#[test]
fn checkpoint_signing_rejects_another_key_or_round() {
    let source = checkpoint_source();
    assert_eq!(
        sign_checkpoint_rating(
            unsigned_checkpoint_rating(),
            &source,
            &PorConfig::default(),
            &private_key(3),
        ),
        Err(PorRatingError::InvalidSignature)
    );
    let mut wrong_round = unsigned_checkpoint_rating();
    wrong_round.round = 2;
    assert_eq!(
        sign_checkpoint_rating(wrong_round, &source, &PorConfig::default(), &private_key(1)),
        Err(PorRatingError::Protocol(PorError::InvalidRatingRound))
    );
}

#[test]
fn checkpoint_batch_verifies_every_signature_before_ordering() {
    let source = checkpoint_source();
    let first = sign_checkpoint_rating(
        unsigned_checkpoint_rating(),
        &source,
        &PorConfig::default(),
        &private_key(1),
    )
    .unwrap();
    let mut second = RatingRecord::new(1, node(2), node(1), 70, Vec::new());
    second.interaction_ref = Some(vec![0x43; 32]);
    let second =
        sign_checkpoint_rating(second, &source, &PorConfig::default(), &private_key(2)).unwrap();
    let batch = build_verified_checkpoint_rating_batch(
        &source,
        vec![second.clone(), first.clone()],
        &PorConfig::default(),
    )
    .unwrap();
    assert_eq!(batch.ratings, vec![first, second]);

    let other_source = RatingSourceCheckpoint {
        finalized_prefix_hash: [0x32; 32],
        ..source
    };
    assert_eq!(
        build_verified_checkpoint_rating_batch(&other_source, batch.ratings, &PorConfig::default(),),
        Err(PorRatingError::InvalidSignature)
    );
}

fn block(tag: u8, creator_seed: u8, predecessor: Option<&BlockIdentity>) -> Block {
    let mut content_hash = [0; 32];
    content_hash[0] = tag;

    Block {
        identity: BlockIdentity {
            content_hash,
            creator: node(creator_seed),
            signature: vec![tag],
        },
        content: BlockContent {
            payload: vec![tag],
            predecessors: predecessor.into_iter().cloned().collect::<HashSet<_>>(),
        },
    }
}

fn finalized_wave_zero_fixture() -> (
    Blocklace,
    OrderedFinalizedOutput,
    FinalizedRatingRound,
    ReputationState,
    BTreeMap<NodeId, Vec<u8>>,
) {
    let mut blocklace = Blocklace::new();
    let leader = block(1, 1, None);
    let second = block(2, 2, Some(&leader.identity));
    let third = block(3, 1, Some(&second.identity));

    for block in [&leader, &second, &third] {
        blocklace.insert(block.clone(), &AcceptAll).unwrap();
    }

    let output = OrderedFinalizedOutput::new(
        vec![
            third.identity.clone(),
            second.identity.clone(),
            leader.identity.clone(),
        ],
        Some(leader.identity.clone()),
        WAVELENGTH,
        3,
        3,
    )
    .with_timestamp(0);
    let opened = PorFinalityTracker::new()
        .observe_finalized_output(&blocklace, &output)
        .unwrap()
        .into_iter()
        .last()
        .unwrap();
    let mut state = ReputationState::new(0);
    state.set_reputation(node(1), 100);
    state.set_reputation(node(2), 100);
    state.set_reputation(node(9), 100);
    let expected_references = BTreeMap::from([
        (node(1), leader.identity.content_hash.to_vec()),
        (node(2), second.identity.content_hash.to_vec()),
    ]);

    (blocklace, output, opened, state, expected_references)
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

#[test]
fn finalized_output_builds_a_verified_block_production_rating_batch() {
    let (blocklace, output, opened, state, expected_references) = finalized_wave_zero_fixture();
    let config = PorConfig::default();

    let batch = build_finalized_block_production_rating_batch(
        &blocklace,
        &output,
        opened,
        &node(9),
        &state,
        &config,
        &private_key(9),
    )
    .unwrap();

    assert_eq!(batch.round, opened.rating_round);
    assert_eq!(batch.ratings.len(), expected_references.len());
    assert!(batch.ratings.windows(2).all(|ratings| {
        (&ratings[0].recipient, &ratings[0].rater) <= (&ratings[1].recipient, &ratings[1].rater)
    }));

    for rating in &batch.ratings {
        assert_eq!(rating.rater, node(9));
        assert_eq!(rating.score, config.maximum_rating);
        assert_eq!(
            rating.interaction_ref.as_ref(),
            expected_references.get(&rating.recipient)
        );
        assert_eq!(validate_signed_rating(rating, &config), Ok(()));
    }
}

#[test]
fn finalized_rating_batch_is_deterministic() {
    let (blocklace, output, opened, state, _) = finalized_wave_zero_fixture();
    let config = PorConfig::default();

    let first = build_finalized_block_production_rating_batch(
        &blocklace,
        &output,
        opened,
        &node(9),
        &state,
        &config,
        &private_key(9),
    )
    .unwrap();
    let second = build_finalized_block_production_rating_batch(
        &blocklace,
        &output,
        opened,
        &node(9),
        &state,
        &config,
        &private_key(9),
    )
    .unwrap();

    assert_eq!(first, second);
}

#[test]
fn unknown_recipients_are_silently_skipped_in_rating_batch() {
    // Before Bug 3 was fixed, a single unknown recipient caused
    // build_finalized_block_production_rating_batch to return
    // Err(Interaction(Admission(UnknownInteractionRecipient))), aborting the
    // whole batch for all remaining valid recipients atomically.
    //
    // After the fix, unknown (and ejected) recipients are filtered before
    // admission; the function returns Ok with only the known members rated.
    //
    // Setup: rater = node(9); fixture wave has producers node(1), node(2), node(3).
    // State contains node(1) and node(9) only — node(2) and node(3) are absent.
    // Expected: Ok(batch) with exactly one rating for node(1).
    let (blocklace, output, opened, _, _) = finalized_wave_zero_fixture();
    let state = {
        let mut missing_recipient = ReputationState::new(0);
        missing_recipient.set_reputation(node(1), 100);
        missing_recipient.set_reputation(node(9), 100);
        missing_recipient
    };

    let batch = build_finalized_block_production_rating_batch(
        &blocklace,
        &output,
        opened,
        &node(9),
        &state,
        &PorConfig::default(),
        &private_key(9),
    )
    .expect("unknown recipients must be silently skipped, not cause an error");

    // Only node(1) is a known, non-ejected member; node(2) and node(3) are
    // absent from the state and must be excluded from the batch.
    assert_eq!(batch.ratings.len(), 1, "only node(1) should be rated");
    assert_eq!(batch.ratings[0].recipient, node(1));
}

#[test]
fn finalized_rating_batch_rejects_a_non_rater_signing_key() {
    let (blocklace, output, opened, state, _) = finalized_wave_zero_fixture();

    assert_eq!(
        build_finalized_block_production_rating_batch(
            &blocklace,
            &output,
            opened,
            &node(9),
            &state,
            &PorConfig::default(),
            &private_key(8),
        ),
        Err(PorRatingError::InvalidSignature)
    );
}
