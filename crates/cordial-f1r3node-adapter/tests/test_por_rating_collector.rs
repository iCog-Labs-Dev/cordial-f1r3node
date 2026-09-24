use std::collections::HashSet;

use cordial_f1r3node_adapter::{
    ordered_output::OrderedFinalizedOutput,
    por_finality::{FinalizedRatingRound, PorFinalityTracker},
    por_interactions::PorInteractionError,
    por_rating_collector::{BlockProductionRatingCollector, PorRatingCollectorError},
    por_rating_wire::BlockProductionRatingEnvelopeV1,
    por_ratings::{
        PorRatingError, build_finalized_block_production_rating_batch, rating_signing_hash,
        validate_signed_rating,
    },
};
use cordial_miners_core::{
    Block, BlockContent, BlockIdentity, Blocklace, NodeId,
    crypto::{CryptoVerifier, Secp256k1Scheme, SignatureScheme},
};
use cordial_por::{PorConfig, PorError, RatingBatch, RatingRecord, ReputationState};
use k256::ecdsa::SigningKey;

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

struct Fixture {
    blocklace: Blocklace,
    output: OrderedFinalizedOutput,
    opened: FinalizedRatingRound,
    state: ReputationState,
    config: PorConfig,
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

fn fixture() -> Fixture {
    let mut blocklace = Blocklace::new();
    let leader = block(1, 1, None);
    let second = block(2, 2, Some(&leader.identity));
    let third = block(3, 3, Some(&second.identity));

    for block in [&leader, &second, &third] {
        blocklace.insert(block.clone(), &AcceptAll).unwrap();
    }

    let output = OrderedFinalizedOutput::new(
        vec![
            third.identity.clone(),
            leader.identity.clone(),
            second.identity.clone(),
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
        .unwrap();
    let mut state = ReputationState::new(0);
    for seed in [1, 2, 3, 8, 9] {
        state.set_reputation(node(seed), 100);
    }

    Fixture {
        blocklace,
        output,
        opened,
        state,
        config: PorConfig::default(),
    }
}

fn local_batch(fixture: &Fixture, rater_seed: u8) -> RatingBatch {
    build_finalized_block_production_rating_batch(
        &fixture.blocklace,
        &fixture.output,
        fixture.opened,
        &node(rater_seed),
        &fixture.state,
        &fixture.config,
        &private_key(rater_seed),
    )
    .unwrap()
}

fn resign(rating: &mut RatingRecord, rater_seed: u8) {
    rating.signature.clear();
    let hash = rating_signing_hash(rating).unwrap();
    rating.signature = Secp256k1Scheme
        .sign(&hash, &private_key(rater_seed))
        .unwrap();
}

#[test]
fn collects_multiple_validators_into_one_canonical_verified_batch() {
    let fixture = fixture();
    let batch_eight = local_batch(&fixture, 8);
    let batch_nine = local_batch(&fixture, 9);
    let mut collector = BlockProductionRatingCollector::new(
        &fixture.blocklace,
        &fixture.output,
        fixture.opened,
        &fixture.state,
        &fixture.config,
    )
    .unwrap();

    collector.insert_batch(batch_nine).unwrap();
    collector.insert_batch(batch_eight).unwrap();

    assert_eq!(collector.round(), fixture.opened.rating_round);
    assert_eq!(collector.len(), 6);
    let batch = collector.finish().unwrap();

    assert_eq!(batch.round, fixture.opened.rating_round);
    assert_eq!(batch.ratings.len(), 6);
    assert!(batch.ratings.windows(2).all(|ratings| {
        (&ratings[0].recipient, &ratings[0].rater) <= (&ratings[1].recipient, &ratings[1].rater)
    }));
    for rating in &batch.ratings {
        assert_eq!(validate_signed_rating(rating, &fixture.config), Ok(()));
    }
}

#[test]
fn accepts_a_decoded_block_production_rating_envelope() {
    let fixture = fixture();
    let rating = local_batch(&fixture, 8).ratings.remove(0);
    let bytes = BlockProductionRatingEnvelopeV1::new(fixture.opened.finalized_wave, rating.clone())
        .unwrap()
        .encode();
    let decoded = BlockProductionRatingEnvelopeV1::decode(&bytes).unwrap();
    let mut collector = BlockProductionRatingCollector::new(
        &fixture.blocklace,
        &fixture.output,
        fixture.opened,
        &fixture.state,
        &fixture.config,
    )
    .unwrap();

    collector.insert_envelope(decoded).unwrap();
    let batch = collector.finish().unwrap();

    assert_eq!(batch.ratings, vec![rating]);
}

#[test]
fn rejects_an_envelope_from_another_finalized_wave() {
    let fixture = fixture();
    let mut rating = local_batch(&fixture, 8).ratings.remove(0);
    rating.round += 1;
    resign(&mut rating, 8);
    let envelope =
        BlockProductionRatingEnvelopeV1::new(fixture.opened.finalized_wave + 1, rating).unwrap();
    let mut collector = BlockProductionRatingCollector::new(
        &fixture.blocklace,
        &fixture.output,
        fixture.opened,
        &fixture.state,
        &fixture.config,
    )
    .unwrap();

    assert_eq!(
        collector.insert_envelope(envelope),
        Err(PorRatingCollectorError::InvalidFinalizedWave)
    );
    assert!(collector.is_empty());
}

#[test]
fn rejects_a_signed_rating_with_the_wrong_finalized_reference() {
    let fixture = fixture();
    let mut rating = local_batch(&fixture, 8).ratings.remove(0);
    rating.interaction_ref.as_mut().unwrap()[0] ^= 0xff;
    resign(&mut rating, 8);
    let mut collector = BlockProductionRatingCollector::new(
        &fixture.blocklace,
        &fixture.output,
        fixture.opened,
        &fixture.state,
        &fixture.config,
    )
    .unwrap();

    assert_eq!(
        collector.insert(rating),
        Err(PorRatingCollectorError::InteractionReferenceMismatch)
    );
    assert!(collector.is_empty());
}

#[test]
fn rejects_a_signed_rating_with_a_non_deterministic_score() {
    let fixture = fixture();
    let mut rating = local_batch(&fixture, 8).ratings.remove(0);
    rating.score -= 1;
    resign(&mut rating, 8);
    let mut collector = BlockProductionRatingCollector::new(
        &fixture.blocklace,
        &fixture.output,
        fixture.opened,
        &fixture.state,
        &fixture.config,
    )
    .unwrap();

    assert_eq!(
        collector.insert(rating),
        Err(PorRatingCollectorError::ScoreMismatch)
    );
    assert!(collector.is_empty());
}

#[test]
fn rejects_duplicate_and_conflicting_submissions_for_the_same_pair() {
    let fixture = fixture();
    let rating = local_batch(&fixture, 8).ratings.remove(0);
    let mut collector = BlockProductionRatingCollector::new(
        &fixture.blocklace,
        &fixture.output,
        fixture.opened,
        &fixture.state,
        &fixture.config,
    )
    .unwrap();

    collector.insert(rating.clone()).unwrap();
    assert_eq!(
        collector.insert(rating.clone()),
        Err(PorRatingCollectorError::DuplicateRating)
    );

    let mut conflicting = rating;
    conflicting.score -= 1;
    resign(&mut conflicting, 8);
    assert_eq!(
        collector.insert(conflicting),
        Err(PorRatingCollectorError::ConflictingRating)
    );
    assert_eq!(collector.len(), 1);
}

#[test]
fn rejects_rating_and_batch_round_mismatches_without_mutation() {
    let fixture = fixture();
    let mut wrong_round_rating = local_batch(&fixture, 8).ratings.remove(0);
    wrong_round_rating.round += 1;
    resign(&mut wrong_round_rating, 8);
    let mut wrong_round_batch = local_batch(&fixture, 9);
    wrong_round_batch.round += 1;
    let mut collector = BlockProductionRatingCollector::new(
        &fixture.blocklace,
        &fixture.output,
        fixture.opened,
        &fixture.state,
        &fixture.config,
    )
    .unwrap();

    assert_eq!(
        collector.insert(wrong_round_rating),
        Err(PorRatingCollectorError::InvalidRatingRound)
    );
    assert_eq!(
        collector.insert_batch(wrong_round_batch),
        Err(PorRatingCollectorError::InvalidBatchRound)
    );
    assert!(collector.is_empty());
}

#[test]
fn rejects_a_rating_from_an_ejected_rater() {
    let fixture = fixture();
    let rating = local_batch(&fixture, 8).ratings.remove(0);
    let mut ejected_state = fixture.state.clone();
    ejected_state.eject_validator(&node(8)).unwrap();
    let mut collector = BlockProductionRatingCollector::new(
        &fixture.blocklace,
        &fixture.output,
        fixture.opened,
        &ejected_state,
        &fixture.config,
    )
    .unwrap();

    assert_eq!(
        collector.insert(rating),
        Err(PorRatingCollectorError::Interaction(
            PorInteractionError::Admission(PorError::EjectedInteractionRater)
        ))
    );
}

#[test]
fn rejects_a_signed_rating_without_finalized_recipient_evidence() {
    let fixture = fixture();
    let mut rating = local_batch(&fixture, 8).ratings.remove(0);
    rating.recipient = node(7);
    resign(&mut rating, 8);
    let mut collector = BlockProductionRatingCollector::new(
        &fixture.blocklace,
        &fixture.output,
        fixture.opened,
        &fixture.state,
        &fixture.config,
    )
    .unwrap();

    assert_eq!(
        collector.insert(rating),
        Err(PorRatingCollectorError::MissingFinalizedInteraction)
    );
}

#[test]
fn batch_insertion_is_atomic_when_one_signature_is_invalid() {
    let fixture = fixture();
    let mut invalid_batch = local_batch(&fixture, 8);
    invalid_batch.ratings[1].signature = vec![1, 2, 3];
    let mut collector = BlockProductionRatingCollector::new(
        &fixture.blocklace,
        &fixture.output,
        fixture.opened,
        &fixture.state,
        &fixture.config,
    )
    .unwrap();
    collector.insert_batch(local_batch(&fixture, 9)).unwrap();
    let original_len = collector.len();

    assert_eq!(
        collector.insert_batch(invalid_batch),
        Err(PorRatingCollectorError::Rating(
            PorRatingError::InvalidSignature
        ))
    );
    assert_eq!(collector.len(), original_len);
}

#[test]
fn constructor_rejects_a_state_that_does_not_precede_the_rating_round() {
    let fixture = fixture();
    let stale_state = ReputationState::new(fixture.opened.rating_round);

    assert!(matches!(
        BlockProductionRatingCollector::new(
            &fixture.blocklace,
            &fixture.output,
            fixture.opened,
            &stale_state,
            &fixture.config,
        ),
        Err(PorRatingCollectorError::InvalidStateRound)
    ));
}

#[test]
fn constructor_rejects_a_round_not_opened_by_the_finalized_output() {
    let fixture = fixture();
    let mismatched = FinalizedRatingRound {
        finalized_wave: fixture.opened.finalized_wave + 1,
        rating_round: fixture.opened.rating_round + 1,
    };

    assert!(matches!(
        BlockProductionRatingCollector::new(
            &fixture.blocklace,
            &fixture.output,
            mismatched,
            &fixture.state,
            &fixture.config,
        ),
        Err(PorRatingCollectorError::Interaction(
            PorInteractionError::FinalizedRoundMismatch
        ))
    ));
}

#[test]
fn closes_an_empty_validated_round_without_inventing_ratings() {
    let fixture = fixture();
    let collector = BlockProductionRatingCollector::new(
        &fixture.blocklace,
        &fixture.output,
        fixture.opened,
        &fixture.state,
        &fixture.config,
    )
    .unwrap();

    let batch = collector.finish().unwrap();

    assert_eq!(batch.round, fixture.opened.rating_round);
    assert!(batch.ratings.is_empty());
}
