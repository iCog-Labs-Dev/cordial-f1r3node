use std::collections::HashSet;

use cordial_f1r3node_adapter::{
    ordered_output::OrderedFinalizedOutput,
    por_finality::{FinalizedRatingRound, PorFinalityTracker},
    por_rating_channel::{
        PorRatingChannelError, RatingEnvelopeReceiveOutcome, bounded_rating_envelope_channel,
    },
    por_rating_collector::BlockProductionRatingCollector,
    por_rating_transport::{PorRatingTransportError, broadcast_rating_batch},
    por_rating_wire::{MAX_BLOCK_PRODUCTION_RATING_ENVELOPE_LEN, PorRatingWireError},
    por_ratings::build_finalized_block_production_rating_batch,
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
            leader.identity.clone(),
            third.identity.clone(),
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
    for seed in [1, 2, 9] {
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

fn local_batch(fixture: &Fixture) -> RatingBatch {
    build_finalized_block_production_rating_batch(
        &fixture.blocklace,
        &fixture.output,
        fixture.opened,
        &node(9),
        &fixture.state,
        &fixture.config,
        &private_key(9),
    )
    .unwrap()
}

fn collector<'a>(fixture: &'a Fixture) -> BlockProductionRatingCollector<'a> {
    BlockProductionRatingCollector::new(
        &fixture.blocklace,
        &fixture.output,
        fixture.opened,
        &fixture.state,
        &fixture.config,
    )
    .unwrap()
}

#[tokio::test]
async fn channel_delivers_a_complete_batch_into_the_collector() {
    let fixture = fixture();
    let batch = local_batch(&fixture);
    let (broadcaster, mut receiver) = bounded_rating_envelope_channel(batch.ratings.len()).unwrap();
    let mut collector = collector(&fixture);

    assert_eq!(
        broadcast_rating_batch(&broadcaster, fixture.opened.finalized_wave, &batch),
        Ok(batch.ratings.len())
    );
    for _ in &batch.ratings {
        assert_eq!(
            receiver.receive_next(&mut collector).await,
            Ok(RatingEnvelopeReceiveOutcome::Accepted)
        );
    }

    assert_eq!(collector.finish().unwrap(), batch);
}

#[tokio::test]
async fn bounded_channel_reports_partial_delivery_under_backpressure() {
    let fixture = fixture();
    let batch = local_batch(&fixture);
    let (broadcaster, mut receiver) = bounded_rating_envelope_channel(1).unwrap();
    let mut collector = collector(&fixture);

    assert_eq!(
        broadcast_rating_batch(&broadcaster, fixture.opened.finalized_wave, &batch),
        Err(PorRatingTransportError::Broadcast {
            delivered: 1,
            message: PorRatingChannelError::Full.to_string(),
        })
    );
    assert_eq!(
        receiver.receive_next(&mut collector).await,
        Ok(RatingEnvelopeReceiveOutcome::Accepted)
    );
    assert_eq!(collector.len(), 1);
}

#[tokio::test]
async fn receiver_reports_closed_after_every_sender_is_dropped() {
    let fixture = fixture();
    let (broadcaster, mut receiver) = bounded_rating_envelope_channel(1).unwrap();
    let mut collector = collector(&fixture);
    drop(broadcaster);

    assert_eq!(
        receiver.receive_next(&mut collector).await,
        Ok(RatingEnvelopeReceiveOutcome::Closed)
    );
    assert!(collector.is_empty());
}

#[tokio::test]
async fn malformed_channel_item_is_rejected_without_collector_mutation() {
    let fixture = fixture();
    let (broadcaster, mut receiver) = bounded_rating_envelope_channel(1).unwrap();
    let mut collector = collector(&fixture);
    broadcaster.try_broadcast(&[1, 2, 3]).unwrap();

    assert_eq!(
        receiver.receive_next(&mut collector).await,
        Err(PorRatingTransportError::Wire(
            PorRatingWireError::UnexpectedEnd
        ))
    );
    assert!(collector.is_empty());
}

#[test]
fn sender_rejects_oversized_envelopes_and_closed_receivers() {
    let (broadcaster, receiver) = bounded_rating_envelope_channel(1).unwrap();
    let oversized = vec![0; MAX_BLOCK_PRODUCTION_RATING_ENVELOPE_LEN + 1];

    assert_eq!(
        broadcaster.try_broadcast(&oversized),
        Err(PorRatingChannelError::EnvelopeTooLong {
            actual: oversized.len(),
            maximum: MAX_BLOCK_PRODUCTION_RATING_ENVELOPE_LEN,
        })
    );

    drop(receiver);
    assert_eq!(
        broadcaster.try_broadcast(&[1, 2, 3]),
        Err(PorRatingChannelError::Closed)
    );
}

#[test]
fn zero_capacity_is_rejected_without_panicking() {
    assert!(matches!(
        bounded_rating_envelope_channel(0),
        Err(PorRatingChannelError::ZeroCapacity)
    ));
}
