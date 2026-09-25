use std::{
    cell::{Cell, RefCell},
    collections::HashSet,
};

use cordial_f1r3node_adapter::{
    ordered_output::OrderedFinalizedOutput,
    por_finality::{FinalizedRatingRound, PorFinalityTracker},
    por_rating_quorum::PorRatingRoundClosurePolicy,
    por_rating_round::{
        CompletedPorRatingRound, PorRatingRoundCloseReason, PorRatingRoundCoordinator,
        PorRatingRoundError, PorRatingRoundStatus,
        cutoff::{PorRatingCutoffError, PorRatingRoundCutoffPolicy},
    },
    por_rating_transport::{
        PorRatingTransportError, RatingEnvelopeBroadcaster, encode_rating_batch,
    },
    por_rating_wire::PorRatingWireError,
    por_ratings::{PorRatingError, build_finalized_block_production_rating_batch},
    por_reputation_transition::apply_completed_reputation_round,
};
use cordial_miners_core::{
    Block, BlockContent, BlockIdentity, Blocklace, NodeId, crypto::CryptoVerifier,
};
use cordial_por::{
    PorConfig, PorError, RatingBatch, ReputationState, ReputationVector,
    replay_reputation_transition, reputation_weights,
};
use k256::ecdsa::SigningKey;

const WAVELENGTH: u64 = 3;
const SHARD_ID: &[u8] = b"root";

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

#[derive(Default)]
struct RecordingBroadcaster {
    envelopes: RefCell<Vec<Vec<u8>>>,
}

impl RatingEnvelopeBroadcaster for RecordingBroadcaster {
    fn broadcast_rating_envelope(&self, envelope: &[u8]) -> Result<(), String> {
        self.envelopes.borrow_mut().push(envelope.to_vec());
        Ok(())
    }
}

#[derive(Default)]
struct FailSecondEnvelopeOnce {
    calls: Cell<usize>,
    failed: Cell<bool>,
    envelopes: RefCell<Vec<Vec<u8>>>,
}

impl RatingEnvelopeBroadcaster for FailSecondEnvelopeOnce {
    fn broadcast_rating_envelope(&self, envelope: &[u8]) -> Result<(), String> {
        let call = self.calls.get();
        self.calls.set(call + 1);
        if call == 1 && !self.failed.replace(true) {
            return Err("transport unavailable".to_string());
        }

        self.envelopes.borrow_mut().push(envelope.to_vec());
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
    let third = block(3, 1, Some(&second.identity));

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
    for seed in [1, 2, 8, 9] {
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

fn receive_batch(
    coordinator: &mut PorRatingRoundCoordinator<'_>,
    fixture: &Fixture,
    rater_seed: u8,
) {
    let batch = rating_batch(fixture, rater_seed);
    let envelopes = encode_rating_batch(fixture.opened.finalized_wave, &batch).unwrap();
    for envelope in envelopes {
        coordinator.receive_envelope(&envelope).unwrap();
    }
}

fn cutoff_wave(fixture: &Fixture) -> u64 {
    fixture.opened.finalized_wave + 1
}

fn complete_local_round(fixture: &Fixture) -> CompletedPorRatingRound {
    let mut coordinator = coordinator(fixture);
    coordinator
        .produce_local_batch(&node(9), &private_key(9))
        .unwrap();
    coordinator
        .broadcast_pending(&RecordingBroadcaster::default())
        .unwrap();
    coordinator
        .close_at_finalized_wave(&PorRatingRoundCutoffPolicy::default(), cutoff_wave(fixture))
        .unwrap();
    coordinator.into_completed().unwrap()
}

#[test]
fn coordinates_local_production_broadcast_and_close() {
    let fixture = fixture();
    let mut coordinator = coordinator(&fixture);
    let broadcaster = RecordingBroadcaster::default();

    assert_eq!(coordinator.status(), PorRatingRoundStatus::Open);
    assert_eq!(coordinator.opened(), fixture.opened);
    assert_eq!(coordinator.collected_len(), 0);
    assert_eq!(coordinator.pending_outbound(), 0);

    let local_batch = coordinator
        .produce_local_batch(&node(9), &private_key(9))
        .unwrap()
        .clone();
    assert_eq!(coordinator.local_batch(), Some(&local_batch));
    assert_eq!(coordinator.collected_len(), local_batch.ratings.len());
    assert_eq!(coordinator.pending_outbound(), local_batch.ratings.len());

    assert_eq!(
        coordinator.broadcast_pending(&broadcaster),
        Ok(local_batch.ratings.len())
    );
    assert_eq!(coordinator.pending_outbound(), 0);
    assert_eq!(
        broadcaster.envelopes.borrow().len(),
        local_batch.ratings.len()
    );

    assert_eq!(
        coordinator.close_at_finalized_wave(
            &PorRatingRoundCutoffPolicy::default(),
            cutoff_wave(&fixture),
        ),
        Ok(&local_batch)
    );
    assert_eq!(coordinator.status(), PorRatingRoundStatus::Closed);
    assert_eq!(coordinator.completed_batch(), Some(&local_batch));
    assert_eq!(
        coordinator.close_reason(),
        Some(PorRatingRoundCloseReason::FinalizedWaveCutoff {
            observed_finalized_wave: cutoff_wave(&fixture),
            required_finalized_wave: cutoff_wave(&fixture),
        })
    );
}

#[test]
fn open_coordinator_cannot_be_consumed_as_a_completed_round() {
    let fixture = fixture();

    assert_eq!(
        coordinator(&fixture).into_completed(),
        Err(PorRatingRoundError::RoundNotClosed)
    );
}

#[test]
fn completed_round_drives_the_full_atomic_reputation_transition() {
    let mut fixture = fixture();
    let previous = ReputationVector {
        round: fixture.state.round(),
        values: fixture.state.reputation_list().entries.clone(),
    };
    let completed = complete_local_round(&fixture);
    let expected = replay_reputation_transition(
        &previous,
        &completed.batch().ratings,
        completed.batch().round,
        &fixture.config,
    )
    .unwrap();
    let config = fixture.config.clone();

    let applied =
        apply_completed_reputation_round(&completed, &mut fixture.state, &config, SHARD_ID)
            .unwrap();

    assert_eq!(applied.close_reason, completed.close_reason());
    assert_eq!(applied.block.reputation_list, expected);
    assert_eq!(fixture.state.reputation_list(), &expected);
    assert_eq!(fixture.state.latest_block(), Some(&applied.block));
    assert_eq!(applied.weights, reputation_weights(&fixture.state));
}

#[test]
fn transition_failure_leaves_previous_reputation_state_unchanged() {
    let mut fixture = fixture();
    let completed = complete_local_round(&fixture);
    let before = fixture.state.clone();
    let config = fixture.config.clone();

    assert_eq!(
        apply_completed_reputation_round(&completed, &mut fixture.state, &config, b""),
        Err(PorError::MissingReputationBlockShardId)
    );
    assert_eq!(fixture.state, before);
}

#[test]
fn collects_remote_envelopes_before_explicit_close() {
    let fixture = fixture();
    let mut coordinator = coordinator(&fixture);
    let broadcaster = RecordingBroadcaster::default();
    let local_batch = coordinator
        .produce_local_batch(&node(9), &private_key(9))
        .unwrap()
        .clone();
    coordinator.broadcast_pending(&broadcaster).unwrap();
    let remote_batch = rating_batch(&fixture, 8);
    let remote_envelopes =
        encode_rating_batch(fixture.opened.finalized_wave, &remote_batch).unwrap();

    for envelope in remote_envelopes {
        coordinator.receive_envelope(&envelope).unwrap();
    }

    let completed = coordinator
        .close_at_finalized_wave(
            &PorRatingRoundCutoffPolicy::default(),
            cutoff_wave(&fixture),
        )
        .unwrap();
    assert_eq!(
        completed.ratings.len(),
        local_batch.ratings.len() + remote_batch.ratings.len()
    );
    assert!(completed.ratings.windows(2).all(|ratings| {
        (&ratings[0].recipient, &ratings[0].rater) <= (&ratings[1].recipient, &ratings[1].rater)
    }));
}

#[test]
fn partial_broadcast_resumes_after_the_delivered_prefix() {
    let fixture = fixture();
    let mut coordinator = coordinator(&fixture);
    let local_batch = coordinator
        .produce_local_batch(&node(9), &private_key(9))
        .unwrap()
        .clone();
    let expected = encode_rating_batch(fixture.opened.finalized_wave, &local_batch).unwrap();
    let broadcaster = FailSecondEnvelopeOnce::default();

    assert_eq!(
        coordinator.broadcast_pending(&broadcaster),
        Err(PorRatingRoundError::Broadcast {
            delivered: 1,
            remaining: 1,
            message: "transport unavailable".to_string(),
        })
    );
    assert_eq!(coordinator.pending_outbound(), 1);
    assert_eq!(coordinator.broadcast_pending(&broadcaster), Ok(1));
    assert_eq!(coordinator.pending_outbound(), 0);
    assert_eq!(*broadcaster.envelopes.borrow(), expected);
}

#[test]
fn cutoff_close_requires_local_production_and_complete_delivery() {
    let fixture = fixture();
    let mut coordinator = coordinator(&fixture);
    let policy = PorRatingRoundCutoffPolicy::default();

    assert_eq!(
        coordinator.close_at_finalized_wave(&policy, cutoff_wave(&fixture)),
        Err(PorRatingRoundError::LocalBatchNotProduced)
    );
    let local_len = coordinator
        .produce_local_batch(&node(9), &private_key(9))
        .unwrap()
        .ratings
        .len();
    assert_eq!(
        coordinator.close_at_finalized_wave(&policy, cutoff_wave(&fixture)),
        Err(PorRatingRoundError::PendingOutboundRatings(local_len))
    );
    assert_eq!(coordinator.status(), PorRatingRoundStatus::Open);

    coordinator
        .broadcast_pending(&RecordingBroadcaster::default())
        .unwrap();
    coordinator
        .close_at_finalized_wave(&policy, cutoff_wave(&fixture))
        .unwrap();
    assert_eq!(coordinator.status(), PorRatingRoundStatus::Closed);
}

#[test]
fn local_production_failure_is_atomic() {
    let fixture = fixture();
    let mut coordinator = coordinator(&fixture);

    assert_eq!(
        coordinator.produce_local_batch(&node(9), &private_key(8)),
        Err(PorRatingRoundError::Rating(
            PorRatingError::InvalidSignature
        ))
    );
    assert!(coordinator.local_batch().is_none());
    assert_eq!(coordinator.collected_len(), 0);
    assert_eq!(coordinator.pending_outbound(), 0);
}

#[test]
fn malformed_inbound_data_does_not_mutate_collection() {
    let fixture = fixture();
    let mut coordinator = coordinator(&fixture);

    assert_eq!(
        coordinator.receive_envelope(&[1, 2, 3]),
        Err(PorRatingRoundError::Transport(
            PorRatingTransportError::Wire(PorRatingWireError::UnexpectedEnd)
        ))
    );
    assert_eq!(coordinator.collected_len(), 0);
}

#[test]
fn rejects_duplicate_local_production_and_all_mutation_after_close() {
    let fixture = fixture();
    let mut coordinator = coordinator(&fixture);
    let broadcaster = RecordingBroadcaster::default();
    let local_batch = coordinator
        .produce_local_batch(&node(9), &private_key(9))
        .unwrap()
        .clone();
    let encoded = encode_rating_batch(fixture.opened.finalized_wave, &local_batch).unwrap();

    assert_eq!(
        coordinator.produce_local_batch(&node(9), &private_key(9)),
        Err(PorRatingRoundError::LocalBatchAlreadyProduced)
    );
    coordinator.broadcast_pending(&broadcaster).unwrap();
    coordinator
        .close_at_finalized_wave(
            &PorRatingRoundCutoffPolicy::default(),
            cutoff_wave(&fixture),
        )
        .unwrap();

    assert_eq!(
        coordinator.produce_local_batch(&node(9), &private_key(9)),
        Err(PorRatingRoundError::RoundClosed)
    );
    assert_eq!(
        coordinator.broadcast_pending(&broadcaster),
        Err(PorRatingRoundError::RoundClosed)
    );
    assert_eq!(
        coordinator.receive_envelope(&encoded[0]),
        Err(PorRatingRoundError::RoundClosed)
    );
    assert_eq!(
        coordinator.close_at_finalized_wave(
            &PorRatingRoundCutoffPolicy::default(),
            cutoff_wave(&fixture),
        ),
        Err(PorRatingRoundError::RoundClosed)
    );
}

#[test]
fn finalized_wave_cutoff_rejects_early_close_and_discards_partial_batches() {
    let fixture = fixture();
    let mut coordinator = coordinator(&fixture);
    let broadcaster = RecordingBroadcaster::default();
    let policy = PorRatingRoundCutoffPolicy::default();
    let local_batch = coordinator
        .produce_local_batch(&node(9), &private_key(9))
        .unwrap()
        .clone();
    coordinator.broadcast_pending(&broadcaster).unwrap();
    let partial_remote = rating_batch(&fixture, 8);
    let envelopes = encode_rating_batch(fixture.opened.finalized_wave, &partial_remote).unwrap();
    coordinator.receive_envelope(&envelopes[0]).unwrap();

    assert_eq!(coordinator.collected_len(), local_batch.ratings.len() + 1);
    assert_eq!(
        coordinator.close_at_finalized_wave(&policy, fixture.opened.finalized_wave),
        Err(PorRatingRoundError::Cutoff(
            PorRatingCutoffError::CutoffNotReached {
                observed_finalized_wave: fixture.opened.finalized_wave,
                required_finalized_wave: cutoff_wave(&fixture),
            }
        ))
    );
    assert_eq!(coordinator.status(), PorRatingRoundStatus::Open);
    assert_eq!(coordinator.close_reason(), None);

    let completed = coordinator
        .close_at_finalized_wave(&policy, cutoff_wave(&fixture))
        .unwrap();
    assert_eq!(completed, &local_batch);
    assert!(
        completed
            .ratings
            .iter()
            .all(|rating| rating.rater == node(9))
    );
}

#[test]
fn quorum_close_discards_a_non_quorum_partial_prefix() {
    let fixture = fixture();
    let mut coordinator = coordinator(&fixture);
    let broadcaster = RecordingBroadcaster::default();
    coordinator
        .produce_local_batch(&node(9), &private_key(9))
        .unwrap();
    coordinator.broadcast_pending(&broadcaster).unwrap();
    receive_batch(&mut coordinator, &fixture, 1);
    receive_batch(&mut coordinator, &fixture, 2);
    let partial_remote = rating_batch(&fixture, 8);
    let envelopes = encode_rating_batch(fixture.opened.finalized_wave, &partial_remote).unwrap();
    coordinator.receive_envelope(&envelopes[0]).unwrap();

    assert_eq!(coordinator.collected_len(), 5);
    let completed = coordinator
        .close_if_quorum(&PorRatingRoundClosurePolicy::default())
        .unwrap();
    assert_eq!(completed.ratings.len(), 4);
    assert!(
        completed
            .ratings
            .iter()
            .all(|rating| rating.rater != node(8))
    );
    assert_eq!(
        coordinator.close_reason(),
        Some(PorRatingRoundCloseReason::Quorum {
            completed_weight: 300,
            required_weight: 267,
        })
    );
}

#[test]
fn finalized_wave_cutoff_policy_validates_lag_and_overflow() {
    assert_eq!(
        PorRatingRoundCutoffPolicy::new(0),
        Err(PorRatingCutoffError::ZeroWaveLag)
    );

    let fixture = fixture();
    let policy = PorRatingRoundCutoffPolicy::new(2).unwrap();
    assert_eq!(policy.wave_lag(), 2);
    assert_eq!(
        policy.required_finalized_wave(fixture.opened),
        Ok(fixture.opened.finalized_wave + 2)
    );

    let overflowed = FinalizedRatingRound {
        finalized_wave: u64::MAX,
        rating_round: 0,
    };
    assert_eq!(
        policy.required_finalized_wave(overflowed),
        Err(PorRatingCutoffError::RequiredWaveOverflow {
            finalized_wave: u64::MAX,
            wave_lag: 2,
        })
    );
}
