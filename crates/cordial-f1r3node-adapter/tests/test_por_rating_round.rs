use std::{
    cell::{Cell, RefCell},
    collections::HashSet,
};

use cordial_f1r3node_adapter::{
    ordered_output::OrderedFinalizedOutput,
    por_finality::{FinalizedRatingRound, PorFinalityTracker},
    por_rating_round::{PorRatingRoundCoordinator, PorRatingRoundError, PorRatingRoundStatus},
    por_rating_transport::{
        PorRatingTransportError, RatingEnvelopeBroadcaster, encode_rating_batch,
    },
    por_rating_wire::PorRatingWireError,
    por_ratings::{PorRatingError, build_finalized_block_production_rating_batch},
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

    assert_eq!(coordinator.close(), Ok(&local_batch));
    assert_eq!(coordinator.status(), PorRatingRoundStatus::Closed);
    assert_eq!(coordinator.completed_batch(), Some(&local_batch));
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

    let completed = coordinator.close().unwrap();
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
fn close_requires_local_production_and_complete_delivery() {
    let fixture = fixture();
    let mut coordinator = coordinator(&fixture);

    assert_eq!(
        coordinator.close(),
        Err(PorRatingRoundError::LocalBatchNotProduced)
    );
    let local_len = coordinator
        .produce_local_batch(&node(9), &private_key(9))
        .unwrap()
        .ratings
        .len();
    assert_eq!(
        coordinator.close(),
        Err(PorRatingRoundError::PendingOutboundRatings(local_len))
    );
    assert_eq!(coordinator.status(), PorRatingRoundStatus::Open);

    coordinator
        .broadcast_pending(&RecordingBroadcaster::default())
        .unwrap();
    coordinator.close().unwrap();
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
    coordinator.close().unwrap();

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
    assert_eq!(coordinator.close(), Err(PorRatingRoundError::RoundClosed));
}
