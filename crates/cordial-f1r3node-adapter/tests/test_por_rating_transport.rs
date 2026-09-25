use std::{cell::RefCell, collections::HashSet};

use cordial_f1r3node_adapter::{
    ordered_output::OrderedFinalizedOutput,
    por_finality::{FinalizedRatingRound, PorFinalityTracker},
    por_rating_collector::{BlockProductionRatingCollector, PorRatingCollectorError},
    por_rating_transport::{
        PorRatingTransportError, RatingEnvelopeBroadcaster, broadcast_rating_batch,
        encode_rating_batch, receive_rating_envelope,
    },
    por_rating_wire::{BlockProductionRatingEnvelopeV1, PorRatingWireError},
    por_ratings::{build_finalized_block_production_rating_batch, rating_signing_hash},
};
use cordial_miners_core::{
    Block, BlockContent, BlockIdentity, Blocklace, NodeId,
    crypto::{CryptoVerifier, Secp256k1Scheme, SignatureScheme},
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
    fail_at: Option<usize>,
}

impl RecordingBroadcaster {
    fn failing_at(index: usize) -> Self {
        Self {
            envelopes: RefCell::new(Vec::new()),
            fail_at: Some(index),
        }
    }
}

impl RatingEnvelopeBroadcaster for RecordingBroadcaster {
    fn broadcast_rating_envelope(&self, envelope: &[u8]) -> Result<(), String> {
        let delivered = self.envelopes.borrow().len();
        if self.fail_at == Some(delivered) {
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
            second.identity.clone(),
            third.identity.clone(),
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

#[test]
fn broadcasts_preencoded_envelopes_in_batch_order() {
    let fixture = fixture();
    let batch = local_batch(&fixture);
    let broadcaster = RecordingBroadcaster::default();

    let delivered =
        broadcast_rating_batch(&broadcaster, fixture.opened.finalized_wave, &batch).unwrap();

    assert_eq!(delivered, batch.ratings.len());
    let encoded = broadcaster.envelopes.borrow();
    assert_eq!(encoded.len(), batch.ratings.len());
    for (bytes, rating) in encoded.iter().zip(&batch.ratings) {
        let decoded = BlockProductionRatingEnvelopeV1::decode(bytes).unwrap();
        assert_eq!(decoded.finalized_wave(), fixture.opened.finalized_wave);
        assert_eq!(decoded.rating(), rating);
    }
}

#[test]
fn encoding_failure_prevents_all_broadcast_side_effects() {
    let fixture = fixture();
    let mut batch = local_batch(&fixture);
    batch.ratings[1].interaction_ref = Some(vec![0; 31]);
    let broadcaster = RecordingBroadcaster::default();

    assert_eq!(
        broadcast_rating_batch(&broadcaster, fixture.opened.finalized_wave, &batch),
        Err(PorRatingTransportError::Wire(
            PorRatingWireError::InvalidInteractionReferenceLength(31)
        ))
    );
    assert!(broadcaster.envelopes.borrow().is_empty());
}

#[test]
fn transport_failure_reports_the_delivered_prefix() {
    let fixture = fixture();
    let batch = local_batch(&fixture);
    let broadcaster = RecordingBroadcaster::failing_at(1);

    assert_eq!(
        broadcast_rating_batch(&broadcaster, fixture.opened.finalized_wave, &batch),
        Err(PorRatingTransportError::Broadcast {
            delivered: 1,
            message: "transport unavailable".to_string(),
        })
    );
    assert_eq!(broadcaster.envelopes.borrow().len(), 1);
}

#[test]
fn rejects_a_batch_round_unrelated_to_the_finalized_wave_before_broadcast() {
    let fixture = fixture();
    let mut batch = local_batch(&fixture);
    batch.round += 1;
    let broadcaster = RecordingBroadcaster::default();

    assert_eq!(
        broadcast_rating_batch(&broadcaster, fixture.opened.finalized_wave, &batch),
        Err(PorRatingTransportError::InvalidBatchRound)
    );
    assert!(broadcaster.envelopes.borrow().is_empty());
}

#[test]
fn empty_batch_is_a_successful_zero_delivery() {
    let fixture = fixture();
    let batch = RatingBatch {
        round: fixture.opened.rating_round,
        ratings: Vec::new(),
    };
    let broadcaster = RecordingBroadcaster::default();

    assert_eq!(
        broadcast_rating_batch(&broadcaster, fixture.opened.finalized_wave, &batch),
        Ok(0)
    );
    assert!(broadcaster.envelopes.borrow().is_empty());
}

#[test]
fn inbound_envelope_decodes_and_enters_the_evidence_backed_collector() {
    let fixture = fixture();
    let batch = local_batch(&fixture);
    let encoded = encode_rating_batch(fixture.opened.finalized_wave, &batch).unwrap();
    let mut collector = collector(&fixture);

    receive_rating_envelope(&mut collector, &encoded[0]).unwrap();
    let collected = collector.finish().unwrap();

    assert_eq!(collected.ratings, vec![batch.ratings[0].clone()]);
}

#[test]
fn malformed_inbound_bytes_do_not_mutate_the_collector() {
    let fixture = fixture();
    let mut collector = collector(&fixture);

    assert_eq!(
        receive_rating_envelope(&mut collector, &[1, 2, 3]),
        Err(PorRatingTransportError::Wire(
            PorRatingWireError::UnexpectedEnd
        ))
    );
    assert!(collector.is_empty());
}

#[test]
fn structurally_valid_but_false_evidence_is_rejected_after_decoding() {
    let fixture = fixture();
    let mut rating = local_batch(&fixture).ratings.remove(0);
    rating.interaction_ref.as_mut().unwrap()[0] ^= 0xff;
    rating.signature.clear();
    let hash = rating_signing_hash(&rating).unwrap();
    rating.signature = Secp256k1Scheme.sign(&hash, &private_key(9)).unwrap();
    let encoded = BlockProductionRatingEnvelopeV1::new(fixture.opened.finalized_wave, rating)
        .unwrap()
        .encode();
    let mut collector = collector(&fixture);

    assert_eq!(
        receive_rating_envelope(&mut collector, &encoded),
        Err(PorRatingTransportError::Collector(
            PorRatingCollectorError::InteractionReferenceMismatch
        ))
    );
    assert!(collector.is_empty());
}
