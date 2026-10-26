use std::{
    collections::{HashMap, HashSet},
    fs,
};

use cordial_f1r3node_adapter::{
    live_ingress::LiveIngress,
    ordered_output::OrderedFinalizedOutput,
    por::{
        checkpoint_ratings::{
            close_checkpoint_rating_exchange_if_complete, stage_local_checkpoint_rating_batch,
        },
        lifecycle::PorRatingRoundCloseReason,
        persistence::DurablePorState,
        rating_key::load_bonded_rating_key,
        runtime::{PorRuntime, PorRuntimeError},
        transport::{file_exchange::FileRatingExchange, outbox::RatingOutbox},
    },
    por_finality::{FinalizedPrefixCheckpoint, PorFinalityError, finalized_prefix_checkpoint},
    por_interactions::PorInteractionError,
    por_rating_collector::{CheckpointBlockProductionRatingCollector, PorRatingCollectorError},
    por_rating_transport::{PorRatingTransportError, receive_checkpoint_rating_envelope},
    por_rating_wire::{
        BlockProductionRatingEnvelopeV1, BlockProductionRatingEnvelopeV2, PorRatingWireError,
    },
    por_ratings::{
        PorRatingError, build_finalized_checkpoint_rating_batch, sign_checkpoint_rating,
    },
    shard_conf::CasperShardConf,
    snapshot::CORDIAL_WAVELENGTH,
};
use cordial_miners_core::{
    Block, BlockContent, BlockIdentity, Blocklace, NodeId, crypto::CryptoVerifier,
};
use cordial_por::{
    CHECKPOINT_REPUTATION_BLOCK_VERSION, PorConfig, RatingBatch, RatingRecord, ReputationState,
    reputation_block_hash,
};
use k256::ecdsa::SigningKey;

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

fn key(seed: u8) -> [u8; 32] {
    [seed; 32]
}

fn node(seed: u8) -> NodeId {
    let signing_key = SigningKey::from_slice(&key(seed)).unwrap();
    NodeId(
        signing_key
            .verifying_key()
            .to_encoded_point(true)
            .as_bytes()
            .to_vec(),
    )
}

struct Fixture {
    blocklace: Blocklace,
    output: OrderedFinalizedOutput,
    checkpoint: FinalizedPrefixCheckpoint,
    state: ReputationState,
    config: PorConfig,
}

impl Fixture {
    fn new() -> Self {
        let mut blocklace = Blocklace::new();
        let mut identities: Vec<BlockIdentity> = Vec::new();
        for tag in 1..=32u8 {
            let creator = if tag <= 16 {
                1
            } else if tag % 2 == 0 {
                3
            } else {
                2
            };
            let mut hash = [0; 32];
            hash[0] = tag;
            let block = Block {
                identity: BlockIdentity {
                    content_hash: hash,
                    creator: node(creator),
                    signature: vec![tag],
                },
                content: BlockContent {
                    payload: vec![tag],
                    predecessors: identities
                        .last()
                        .cloned()
                        .into_iter()
                        .collect::<HashSet<_>>(),
                },
            };
            blocklace.insert(block.clone(), &AcceptAll).unwrap();
            identities.push(block.identity);
        }
        let output =
            OrderedFinalizedOutput::new(identities.clone(), identities.last().cloned(), 3, 32, 32)
                .with_timestamp(0);
        let checkpoint = finalized_prefix_checkpoint(&output, 2).unwrap().unwrap();
        let mut state = ReputationState::new(1);
        for seed in [1, 2, 3, 9] {
            state.set_reputation(node(seed), 100);
        }
        Self {
            blocklace,
            output,
            checkpoint,
            state,
            config: PorConfig::default(),
        }
    }

    fn first_checkpoint() -> Self {
        let mut fixture = Self::new();
        fixture.checkpoint = finalized_prefix_checkpoint(&fixture.output, 1)
            .unwrap()
            .unwrap();
        fixture.state = ReputationState::new(0);
        for seed in [1, 2, 3, 9] {
            fixture.state.set_reputation(node(seed), 100);
        }
        fixture
    }

    fn collector(&self) -> CheckpointBlockProductionRatingCollector<'_> {
        CheckpointBlockProductionRatingCollector::new(
            &self.blocklace,
            &self.output,
            self.checkpoint.clone(),
            [node(2), node(3), node(9)],
            &self.state,
            &self.config,
        )
        .unwrap()
    }

    fn batch(&self, seed: u8) -> RatingBatch {
        build_finalized_checkpoint_rating_batch(
            &self.blocklace,
            &self.output,
            &self.checkpoint,
            &node(seed),
            &self.state,
            &self.config,
            &key(seed),
        )
        .unwrap()
    }

    fn resign(&self, mut rating: RatingRecord, seed: u8) -> RatingRecord {
        rating.signature.clear();
        sign_checkpoint_rating(
            rating,
            &self.checkpoint.rating_source(),
            &self.config,
            &key(seed),
        )
        .unwrap()
    }
}

#[test]
fn receives_v2_bytes_and_builds_one_canonical_multi_rater_batch() {
    let fixture = Fixture::new();
    let mut collector = fixture.collector();
    let batch_nine = fixture.batch(9);
    let batch_two = fixture.batch(2);

    for rating in batch_nine.ratings.iter().chain(&batch_two.ratings) {
        let bytes = BlockProductionRatingEnvelopeV2::new(
            fixture.checkpoint.rating_source(),
            rating.clone(),
        )
        .unwrap()
        .encode();
        receive_checkpoint_rating_envelope(&mut collector, &bytes).unwrap();
    }

    assert!(collector.is_rater_complete(&node(9)).unwrap());
    assert!(collector.is_rater_complete(&node(2)).unwrap());
    assert_eq!(collector.len(), 3);
    let batch = collector.finish().unwrap();
    assert_eq!(batch.round, 2);
    assert!(batch.ratings.windows(2).all(|ratings| {
        (&ratings[0].recipient, &ratings[0].rater) <= (&ratings[1].recipient, &ratings[1].rater)
    }));
}

#[test]
fn rejects_other_source_and_v1_envelopes_without_mutation() {
    let fixture = Fixture::new();
    let rating = fixture.batch(9).ratings.remove(0);
    let mut collector = fixture.collector();
    let mut other_source = fixture.checkpoint.rating_source();
    other_source.finalized_prefix_hash[0] ^= 1;
    let other = BlockProductionRatingEnvelopeV2::new(other_source, rating.clone()).unwrap();
    assert_eq!(
        collector.insert_envelope(other),
        Err(PorRatingCollectorError::InvalidCheckpointSource)
    );

    let v1 = BlockProductionRatingEnvelopeV1::new(1, rating)
        .unwrap()
        .encode();
    assert_eq!(
        receive_checkpoint_rating_envelope(&mut collector, &v1),
        Err(PorRatingTransportError::Wire(
            PorRatingWireError::UnsupportedVersion(1)
        ))
    );
    assert!(collector.is_empty());
}

#[test]
fn rejects_wrong_reference_score_and_signature_even_when_signed() {
    let fixture = Fixture::new();
    let rating = fixture.batch(9).ratings.remove(0);
    let mut collector = fixture.collector();

    let mut wrong_ref = rating.clone();
    wrong_ref.interaction_ref.as_mut().unwrap()[0] ^= 1;
    assert_eq!(
        collector.insert(fixture.resign(wrong_ref, 9)),
        Err(PorRatingCollectorError::InteractionReferenceMismatch)
    );
    let mut wrong_score = rating.clone();
    wrong_score.score -= 1;
    assert_eq!(
        collector.insert(fixture.resign(wrong_score, 9)),
        Err(PorRatingCollectorError::ScoreMismatch)
    );
    let mut wrong_signature = rating;
    wrong_signature.signature[0] ^= 1;
    assert_eq!(
        collector.insert(wrong_signature),
        Err(PorRatingCollectorError::Rating(
            PorRatingError::InvalidSignature
        ))
    );
    assert!(collector.is_empty());
}

#[test]
fn rejects_a_known_but_unauthorized_rater() {
    let fixture = Fixture::new();
    let rating = fixture.batch(1).ratings.remove(0);
    let mut collector = fixture.collector();
    assert_eq!(
        collector.insert(rating),
        Err(PorRatingCollectorError::UnauthorizedRater(node(1)))
    );
    assert!(collector.is_empty());
}

#[test]
fn invalid_batch_is_atomic_and_partial_rater_is_excluded_from_complete_batch() {
    let fixture = Fixture::new();
    let mut ratings = fixture.batch(9).ratings;
    let first = ratings.remove(0);
    let mut bad_second = ratings.remove(0);
    bad_second.interaction_ref.as_mut().unwrap()[0] ^= 1;
    let mut collector = fixture.collector();
    assert_eq!(
        collector.insert_batch(RatingBatch {
            round: 2,
            ratings: vec![first.clone(), fixture.resign(bad_second, 9)],
        }),
        Err(PorRatingCollectorError::InteractionReferenceMismatch)
    );
    assert!(collector.is_empty());

    collector.insert(first.clone()).unwrap();
    assert!(!collector.is_rater_complete(&node(9)).unwrap());
    assert!(collector.build_complete_batch().unwrap().ratings.is_empty());
    assert_eq!(collector.build_batch().unwrap().ratings, vec![first]);
}

#[test]
fn duplicate_and_conflicting_ratings_keep_the_first_valid_submission() {
    let fixture = Fixture::new();
    let rating = fixture.batch(9).ratings.remove(0);
    let mut collector = fixture.collector();
    collector.insert(rating.clone()).unwrap();
    assert_eq!(
        collector.insert(rating.clone()),
        Err(PorRatingCollectorError::DuplicateRating)
    );
    let mut conflict = rating.clone();
    conflict.score -= 1;
    assert_eq!(
        collector.insert(fixture.resign(conflict, 9)),
        Err(PorRatingCollectorError::ConflictingRating)
    );
    assert_eq!(collector.finish().unwrap().ratings, vec![rating]);
}

#[test]
fn constructor_checks_source_and_state_even_for_empty_collector() {
    let fixture = Fixture::new();
    let mut wrong_checkpoint = fixture.checkpoint.clone();
    wrong_checkpoint.prefix_hash[0] ^= 1;
    assert!(matches!(
        CheckpointBlockProductionRatingCollector::new(
            &fixture.blocklace,
            &fixture.output,
            wrong_checkpoint,
            [node(9)],
            &fixture.state,
            &fixture.config,
        ),
        Err(PorRatingCollectorError::Interaction(
            PorInteractionError::Checkpoint(PorFinalityError::PrefixCheckpointMismatch)
        ))
    ));
    let stale = ReputationState::new(0);
    assert!(matches!(
        CheckpointBlockProductionRatingCollector::new(
            &fixture.blocklace,
            &fixture.output,
            fixture.checkpoint.clone(),
            [node(9)],
            &stale,
            &fixture.config,
        ),
        Err(PorRatingCollectorError::InvalidStateRound)
    ));
}

#[test]
fn constructor_rejects_a_window_block_missing_from_the_blocklace() {
    let fixture = Fixture::new();
    let mut output = fixture.output.clone();
    output.blocks[20].content_hash[0] ^= 1;
    let checkpoint = finalized_prefix_checkpoint(&output, 2).unwrap().unwrap();
    assert!(matches!(
        CheckpointBlockProductionRatingCollector::new(
            &fixture.blocklace,
            &output,
            checkpoint,
            [node(9)],
            &fixture.state,
            &fixture.config,
        ),
        Err(PorRatingCollectorError::Interaction(
            PorInteractionError::UnknownFinalizedBlock
        ))
    ));
}

#[test]
fn completion_rejects_an_ejected_rater_even_without_submissions() {
    let fixture = Fixture::new();
    let mut state = fixture.state.clone();
    state.eject_validator(&node(9)).unwrap();
    let collector = CheckpointBlockProductionRatingCollector::new(
        &fixture.blocklace,
        &fixture.output,
        fixture.checkpoint.clone(),
        [node(9)],
        &state,
        &fixture.config,
    )
    .unwrap();
    assert_eq!(
        collector.is_rater_complete(&node(9)),
        Err(PorRatingCollectorError::UnauthorizedRater(node(9)))
    );
}

#[test]
fn checkpoint_exchange_closes_only_after_complete_peer_batches_and_recovers() {
    let fixture = Fixture::new();
    let directory = tempfile::tempdir().unwrap();
    let exchange = FileRatingExchange::open(&directory.path().join("exchange"), b"root").unwrap();
    let authorized = HashSet::from([node(2), node(3), node(9)]);
    let key_path_two = directory.path().join("key-two");
    let key_path_nine = directory.path().join("key-nine");
    fs::write(&key_path_two, hex::encode(key(2))).unwrap();
    fs::write(&key_path_nine, hex::encode(key(9))).unwrap();
    let key_two = load_bonded_rating_key(&key_path_two, &HashMap::from([(node(2), 100)])).unwrap();
    let key_nine =
        load_bonded_rating_key(&key_path_nine, &HashMap::from([(node(9), 100)])).unwrap();
    let data_two = directory.path().join("node-two");
    let data_nine = directory.path().join("node-nine");
    let outbox_two = RatingOutbox::open(&data_two).unwrap();
    let outbox_nine = RatingOutbox::open(&data_nine).unwrap();
    let source = fixture.checkpoint.rating_source();
    stage_local_checkpoint_rating_batch(
        &fixture.blocklace,
        &fixture.output,
        &fixture.checkpoint,
        &fixture.state,
        &fixture.config,
        &key_two,
        &outbox_two,
    )
    .unwrap();
    stage_local_checkpoint_rating_batch(
        &fixture.blocklace,
        &fixture.output,
        &fixture.checkpoint,
        &fixture.state,
        &fixture.config,
        &key_nine,
        &outbox_nine,
    )
    .unwrap();

    let close_two = |outbox: &RatingOutbox, exchange: &FileRatingExchange| {
        close_checkpoint_rating_exchange_if_complete(
            &fixture.blocklace,
            &fixture.output,
            &fixture.checkpoint,
            &fixture.state,
            &fixture.config,
            &key_two,
            &authorized,
            outbox,
            exchange,
        )
        .unwrap()
    };
    assert!(close_two(&outbox_two, &exchange).is_none());

    let batch_nine = fixture.batch(9);
    assert!(batch_nine.ratings.len() > 1);
    let peer_nine = exchange.checkpoint_broadcaster(source, node(9)).unwrap();
    let first = BlockProductionRatingEnvelopeV2::new(source, batch_nine.ratings[0].clone())
        .unwrap()
        .encode();
    peer_nine.publish(&first).unwrap();
    assert!(close_two(&outbox_two, &exchange).is_none());

    for rating in &fixture.batch(3).ratings {
        exchange
            .checkpoint_broadcaster(source, node(3))
            .unwrap()
            .publish(
                &BlockProductionRatingEnvelopeV2::new(source, rating.clone())
                    .unwrap()
                    .encode(),
            )
            .unwrap();
    }
    let closed_nine = close_checkpoint_rating_exchange_if_complete(
        &fixture.blocklace,
        &fixture.output,
        &fixture.checkpoint,
        &fixture.state,
        &fixture.config,
        &key_nine,
        &authorized,
        &outbox_nine,
        &exchange,
    )
    .unwrap()
    .unwrap();
    let closed_two = close_two(&outbox_two, &exchange).unwrap();
    assert_eq!(closed_two, closed_nine);
    assert_eq!(closed_two.source(), source);
    assert_eq!(closed_two.batch().round, source.round);

    drop(outbox_two);
    drop(exchange);
    let recovered_outbox = RatingOutbox::open(&data_two).unwrap();
    let recovered_exchange =
        FileRatingExchange::open(&directory.path().join("exchange"), b"root").unwrap();
    assert_eq!(
        close_two(&recovered_outbox, &recovered_exchange),
        Some(closed_two)
    );
}

#[test]
fn checkpoint_closure_rejects_an_empty_authorization_set() {
    let fixture = Fixture::new();
    let collector = CheckpointBlockProductionRatingCollector::new(
        &fixture.blocklace,
        &fixture.output,
        fixture.checkpoint.clone(),
        [],
        &fixture.state,
        &fixture.config,
    )
    .unwrap();
    assert_eq!(
        collector.all_authorized_raters_complete(),
        Err(PorRatingCollectorError::NoAuthorizedRaters)
    );
}

#[test]
fn first_checkpoint_commit_recovers_source_and_rejects_replay() {
    let fixture = Fixture::first_checkpoint();
    let directory = tempfile::tempdir().unwrap();
    let key_path = directory.path().join("key-two");
    fs::write(&key_path, hex::encode(key(2))).unwrap();
    let local_key = load_bonded_rating_key(&key_path, &HashMap::from([(node(2), 100)])).unwrap();
    let outbox = RatingOutbox::open(&directory.path().join("node-two")).unwrap();
    let exchange = FileRatingExchange::open(&directory.path().join("exchange"), b"root").unwrap();
    let authorized = HashSet::from([node(1), node(2), node(3), node(9)]);
    let source = fixture.checkpoint.rating_source();
    stage_local_checkpoint_rating_batch(
        &fixture.blocklace,
        &fixture.output,
        &fixture.checkpoint,
        &fixture.state,
        &fixture.config,
        &local_key,
        &outbox,
    )
    .unwrap();
    for seed in [1, 3, 9] {
        let publisher = exchange.checkpoint_broadcaster(source, node(seed)).unwrap();
        for rating in fixture.batch(seed).ratings {
            publisher
                .publish(
                    &BlockProductionRatingEnvelopeV2::new(source, rating)
                        .unwrap()
                        .encode(),
                )
                .unwrap();
        }
    }
    let completed = close_checkpoint_rating_exchange_if_complete(
        &fixture.blocklace,
        &fixture.output,
        &fixture.checkpoint,
        &fixture.state,
        &fixture.config,
        &local_key,
        &authorized,
        &outbox,
        &exchange,
    )
    .unwrap()
    .unwrap();

    let runtime_root = directory.path().join("unfinalized-runtime");
    let ingress = LiveIngress::with_consensus_view(
        (),
        HashMap::from([
            (node(1), 100),
            (node(2), 100),
            (node(3), 100),
            (node(9), 100),
        ]),
        CasperShardConf::default(),
        "root",
    );
    let mut unfinalized_runtime = PorRuntime::open(
        &runtime_root,
        fixture.state.clone(),
        ingress,
        fixture.config.clone(),
        b"root",
        CORDIAL_WAVELENGTH,
    )
    .unwrap();
    assert!(matches!(
        unfinalized_runtime.commit_completed_checkpoint_round(&completed),
        Err(PorRuntimeError::CheckpointSource(_))
    ));
    assert_eq!(unfinalized_runtime.state().state().unwrap().round(), 0);
    assert_eq!(
        unfinalized_runtime
            .state()
            .committed_checkpoint_source()
            .unwrap(),
        None
    );

    let state_root = directory.path().join("state");
    let mut durable = DurablePorState::open(&state_root, fixture.state.clone()).unwrap();
    assert_eq!(durable.committed_checkpoint_source().unwrap(), None);
    assert!(
        durable
            .apply_completed_checkpoint_round(&completed, &fixture.config, b"")
            .is_err()
    );
    assert_eq!(durable.state().unwrap().round(), 0);
    assert!(durable.history().is_empty().unwrap());
    let applied = durable
        .apply_completed_checkpoint_round(&completed, &fixture.config, b"root")
        .unwrap();
    assert_eq!(
        applied.close_reason,
        PorRatingRoundCloseReason::AllAuthorizedRaters
    );
    assert_eq!(
        applied.block.header.version,
        CHECKPOINT_REPUTATION_BLOCK_VERSION
    );
    assert_eq!(applied.block.header.source_checkpoint, Some(source));
    assert_eq!(durable.state().unwrap().round(), 1);
    assert_eq!(durable.committed_checkpoint_source().unwrap(), Some(source));
    assert_eq!(
        durable.history().latest().unwrap(),
        Some(applied.block.clone())
    );
    let committed_hash = reputation_block_hash(&applied.block).unwrap();
    drop(durable);

    let mut recovered = DurablePorState::open(&state_root, ReputationState::new(99)).unwrap();
    assert_eq!(
        recovered.committed_checkpoint_source().unwrap(),
        Some(source)
    );
    assert_eq!(recovered.history().len().unwrap(), 1);
    assert_eq!(
        reputation_block_hash(recovered.state().unwrap().latest_block().unwrap()).unwrap(),
        committed_hash
    );
    assert!(
        recovered
            .apply_completed_checkpoint_round(&completed, &fixture.config, b"root")
            .is_err()
    );
    assert_eq!(
        recovered.committed_checkpoint_source().unwrap(),
        Some(source)
    );
}

#[test]
fn checkpoint_completion_excludes_ejected_keys_but_still_requires_zero_weight_active_keys() {
    let fixture = Fixture::new();
    let mut state = fixture.state.clone();
    state.eject_validator(&node(9)).unwrap();
    state.set_reputation(node(2), 0);
    let mut collector = CheckpointBlockProductionRatingCollector::new(
        &fixture.blocklace,
        &fixture.output,
        fixture.checkpoint.clone(),
        [node(2), node(3), node(9)],
        &state,
        &fixture.config,
    )
    .unwrap();
    assert!(!collector.all_authorized_raters_complete().unwrap());
    for seed in [3, 2] {
        let batch = build_finalized_checkpoint_rating_batch(
            &fixture.blocklace,
            &fixture.output,
            &fixture.checkpoint,
            &node(seed),
            &state,
            &fixture.config,
            &key(seed),
        )
        .unwrap();
        collector.insert_batch(batch).unwrap();
        assert_eq!(
            collector.all_authorized_raters_complete().unwrap(),
            seed == 2
        );
    }
    let batch = collector.build_complete_batch().unwrap();
    assert!(batch.ratings.iter().all(|rating| rating.rater != node(9)));
}
