use std::collections::HashSet;

use cordial_f1r3node_adapter::{
    ordered_output::OrderedFinalizedOutput,
    por::{
        DEFAULT_RATING_QUORUM_DENOMINATOR, DEFAULT_RATING_QUORUM_NUMERATOR, FinalizedRatingRound,
        PorFinalityTracker, PorRatingQuorumError, PorRatingRoundClosurePolicy,
        PorRatingRoundCoordinator, PorRatingRoundError, RatingEnvelopeBroadcaster,
        build_finalized_block_production_rating_batch, encode_rating_batch,
    },
};
use cordial_miners_core::{
    Block, BlockContent, BlockIdentity, Blocklace, NodeId, crypto::CryptoVerifier,
};
use cordial_por::{PorConfig, RatingBatch, ReputationState};
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

struct NoopBroadcaster;

impl RatingEnvelopeBroadcaster for NoopBroadcaster {
    fn broadcast_rating_envelope(&self, _envelope: &[u8]) -> Result<(), String> {
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

fn fixture_with_weights(weights: &[(u8, u64)]) -> Fixture {
    fixture_with_producers(weights, [1, 2, 1])
}

fn fixture_with_producers(weights: &[(u8, u64)], producers: [u8; 3]) -> Fixture {
    let mut blocklace = Blocklace::new();
    let leader = block(1, producers[0], None);
    let second = block(2, producers[1], Some(&leader.identity));
    let third = block(3, producers[2], Some(&second.identity));

    for block in [&leader, &second, &third] {
        blocklace.insert(block.clone(), &AcceptAll).unwrap();
    }

    let output = OrderedFinalizedOutput::new(
        vec![
            second.identity.clone(),
            third.identity.clone(),
            leader.identity.clone(),
        ],
        Some(leader.identity.clone()),
        WAVELENGTH,
        3,
        weights.len(),
    )
    .with_timestamp(0);
    let opened = PorFinalityTracker::new()
        .observe_finalized_output(&blocklace, &output)
        .unwrap()
        .unwrap();
    let mut state = ReputationState::new(0);
    for &(seed, weight) in weights {
        state.set_reputation(node(seed), weight);
    }

    Fixture {
        blocklace,
        output,
        opened,
        state,
        config: PorConfig::default(),
    }
}

fn coordinator(fixture: &Fixture) -> PorRatingRoundCoordinator<'_> {
    PorRatingRoundCoordinator::new(
        &fixture.blocklace,
        &fixture.output,
        fixture.opened,
        &fixture.state,
        &fixture.config,
    )
    .unwrap()
}

fn rating_batch(fixture: &Fixture, rater_seed: u8) -> RatingBatch {
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

fn receive_batch(coordinator: &mut PorRatingRoundCoordinator<'_>, fixture: &Fixture, seed: u8) {
    let batch = rating_batch(fixture, seed);
    let envelopes = encode_rating_batch(fixture.opened.finalized_wave, &batch).unwrap();
    for envelope in envelopes {
        coordinator.receive_envelope(&envelope).unwrap();
    }
}

fn produce_and_broadcast_local(coordinator: &mut PorRatingRoundCoordinator<'_>, rater_seed: u8) {
    coordinator
        .produce_local_batch(&node(rater_seed), &private_key(rater_seed))
        .unwrap();
    coordinator.broadcast_pending(&NoopBroadcaster).unwrap();
}

#[test]
fn default_policy_requires_strictly_more_than_two_thirds_active_weight() {
    let fixture = fixture_with_weights(&[(1, 100), (2, 100), (8, 100), (9, 100)]);
    let mut coordinator = coordinator(&fixture);
    let policy = PorRatingRoundClosurePolicy::default();
    produce_and_broadcast_local(&mut coordinator, 9);

    let progress = coordinator.quorum_progress(&policy).unwrap();
    assert_eq!(policy.numerator(), DEFAULT_RATING_QUORUM_NUMERATOR);
    assert_eq!(policy.denominator(), DEFAULT_RATING_QUORUM_DENOMINATOR);
    assert_eq!(progress.eligible_raters, 4);
    assert_eq!(progress.completed_raters, vec![node(9)]);
    assert_eq!(progress.total_active_weight, 400);
    assert_eq!(progress.completed_weight, 100);
    assert_eq!(progress.required_weight, 267);
    assert!(!progress.is_reached());
    assert_eq!(
        coordinator.close_if_quorum(&policy),
        Err(PorRatingRoundError::QuorumNotReached {
            completed_weight: 100,
            required_weight: 267,
        })
    );

    receive_batch(&mut coordinator, &fixture, 8);
    receive_batch(&mut coordinator, &fixture, 1);
    let progress = coordinator.quorum_progress(&policy).unwrap();
    assert_eq!(progress.completed_weight, 300);
    assert!(progress.is_reached());
    assert_eq!(
        coordinator.close_if_quorum(&policy).unwrap().ratings.len(),
        5
    );
}

#[test]
fn partial_remote_batch_does_not_receive_participation_weight() {
    let fixture = fixture_with_weights(&[(1, 100), (2, 100), (8, 100), (9, 100)]);
    let mut coordinator = coordinator(&fixture);
    let policy = PorRatingRoundClosurePolicy::default();
    produce_and_broadcast_local(&mut coordinator, 9);
    let remote_batch = rating_batch(&fixture, 8);
    let envelopes = encode_rating_batch(fixture.opened.finalized_wave, &remote_batch).unwrap();

    coordinator.receive_envelope(&envelopes[0]).unwrap();
    let partial = coordinator.quorum_progress(&policy).unwrap();
    assert_eq!(partial.completed_weight, 100);
    assert!(!partial.completed_raters.contains(&node(8)));

    receive_batch(&mut coordinator, &fixture, 1);
    assert_eq!(
        coordinator
            .quorum_progress(&policy)
            .unwrap()
            .completed_weight,
        200
    );

    coordinator.receive_envelope(&envelopes[1]).unwrap();
    let complete = coordinator.quorum_progress(&policy).unwrap();
    assert_eq!(complete.completed_weight, 300);
    assert!(complete.completed_raters.contains(&node(8)));
    assert!(complete.is_reached());
}

#[test]
fn quorum_is_reputation_weighted_instead_of_head_counted() {
    let fixture = fixture_with_weights(&[(1, 50), (2, 50), (8, 100), (9, 800)]);
    let mut coordinator = coordinator(&fixture);
    let policy = PorRatingRoundClosurePolicy::default();
    produce_and_broadcast_local(&mut coordinator, 9);

    let progress = coordinator.quorum_progress(&policy).unwrap();
    assert_eq!(progress.completed_raters, vec![node(9)]);
    assert_eq!(progress.total_active_weight, 1_000);
    assert_eq!(progress.completed_weight, 800);
    assert_eq!(progress.required_weight, 667);
    assert!(progress.is_reached());
    coordinator.close_if_quorum(&policy).unwrap();
}

#[test]
fn ejected_validators_are_excluded_from_quorum_weight() {
    let mut fixture = fixture_with_weights(&[(1, 100), (2, 100), (8, 700), (9, 100)]);
    fixture.state.eject_validator(&node(8)).unwrap();
    let mut coordinator = coordinator(&fixture);
    let policy = PorRatingRoundClosurePolicy::default();
    produce_and_broadcast_local(&mut coordinator, 9);

    let progress = coordinator.quorum_progress(&policy).unwrap();
    assert_eq!(progress.eligible_raters, 3);
    assert!(!progress.completed_raters.contains(&node(8)));
    assert_eq!(progress.total_active_weight, 300);
    assert_eq!(progress.completed_weight, 100);
    assert_eq!(progress.required_weight, 201);
}

#[test]
fn empty_canonical_batch_is_complete_without_a_wire_envelope() {
    let fixture = fixture_with_producers(&[(1, 100)], [1, 1, 1]);
    let mut coordinator = coordinator(&fixture);
    let policy = PorRatingRoundClosurePolicy::default();
    produce_and_broadcast_local(&mut coordinator, 1);

    assert!(coordinator.local_batch().unwrap().ratings.is_empty());
    let progress = coordinator.quorum_progress(&policy).unwrap();
    assert_eq!(progress.completed_raters, vec![node(1)]);
    assert_eq!(progress.completed_weight, 100);
    assert_eq!(progress.required_weight, 67);
    assert!(progress.is_reached());
    assert!(
        coordinator
            .close_if_quorum(&policy)
            .unwrap()
            .ratings
            .is_empty()
    );
}

#[test]
fn invalid_thresholds_and_zero_active_weight_are_rejected() {
    for (numerator, denominator) in [(0, 3), (2, 0), (3, 3), (4, 3)] {
        assert_eq!(
            PorRatingRoundClosurePolicy::new(numerator, denominator),
            Err(PorRatingQuorumError::InvalidThreshold {
                numerator,
                denominator,
            })
        );
    }

    let half = PorRatingRoundClosurePolicy::new(1, 2).unwrap();
    assert_eq!(half.numerator(), 1);
    assert_eq!(half.denominator(), 2);

    let fixture = fixture_with_weights(&[(1, 0), (2, 0), (8, 0), (9, 0)]);
    let coordinator = coordinator(&fixture);
    assert_eq!(
        coordinator.quorum_progress(&PorRatingRoundClosurePolicy::default()),
        Err(PorRatingRoundError::Quorum(
            PorRatingQuorumError::ZeroActiveWeight
        ))
    );
}
