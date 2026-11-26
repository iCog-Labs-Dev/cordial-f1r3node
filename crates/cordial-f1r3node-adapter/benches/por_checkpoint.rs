//! Signed finalized authorization, penalty replay and v3 checkpoint audit.
//! Key generation, signing, graph construction and file-based rating exchange
//! are untimed. Authorization verifies evidence and publisher signatures.
//! Replay/audit start with authenticated ratings and penalties; they do not
//! include transport, durable state application, or consensus finalization.
use cordial_f1r3node_adapter::{
    grpc_ingest::BlocklaceAdapter,
    live_ingress::LiveIngress,
    por::{
        checkpoint_ratings::{
            close_checkpoint_rating_exchange_if_complete, stage_local_checkpoint_rating_batch,
        },
        finalized_prefix_checkpoint,
        rating_key::load_bonded_rating_key,
        ratings::build_finalized_checkpoint_rating_batch,
        transport::{file_exchange::FileRatingExchange, outbox::RatingOutbox},
    },
    por_rating_wire::BlockProductionRatingEnvelopeV2,
    shard_conf::CasperShardConf,
};
use cordial_miners_core::{
    Block, BlockContent, BlockIdentity, Blocklace, NodeId, bench_support as support,
    consensus::{CordialEquivocationEvidence, verify_equivocation_evidence},
    crypto::{self, CryptoVerifier},
    execution::{CordialBlockPayload, PorEquivocationAuthorization},
};
use cordial_por::{
    CheckpointReputationBlockContext, PorConfig, ReputationState, ReputationVector,
    build_checkpoint_reputation_block_with_penalties,
    checkpoint_equivocation_authorization_payload, replay_reputation_transition_with_penalties,
    verify_checkpoint_reputation_transition_with_penalties,
};
use criterion::{BatchSize, BenchmarkId, Criterion, criterion_group, criterion_main};
use k256::ecdsa::SigningKey;
use std::{collections::HashSet, time::Duration};

struct AcceptVerifiedBlocks;
impl BlocklaceAdapter<BlockIdentity> for AcceptVerifiedBlocks {
    fn on_block(&mut self, _: Block) -> anyhow::Result<()> {
        Ok(())
    }
}

fn node(seed: u8) -> NodeId {
    NodeId(
        SigningKey::from_slice(&[seed; 32])
            .expect("fixture key")
            .verifying_key()
            .to_encoded_point(true)
            .as_bytes()
            .to_vec(),
    )
}
fn signed(seed: u8, payload: Vec<u8>, predecessors: HashSet<BlockIdentity>) -> Block {
    let content = BlockContent {
        payload,
        predecessors,
    };
    let creator = node(seed);
    let content_hash = crypto::block_hash(&creator, &content);
    Block {
        identity: BlockIdentity {
            content_hash,
            creator,
            signature: crypto::sign(&content_hash, &[seed; 32]),
        },
        content,
    }
}

fn bench_checkpoint(c: &mut Criterion) {
    let mut group = c.benchmark_group("por_checkpoint");
    group.sample_size(10);
    group.measurement_time(Duration::from_secs(2));
    for count in support::sizes(&[1, 2], &[1, 2, 3]) {
        let config = PorConfig::new(1000, 200);
        let mut state = ReputationState::new(0);
        let membership = (1..=4)
            .map(|seed| {
                state.set_reputation(node(seed), 1000);
                (node(seed), 1000)
            })
            .collect();
        let claims: Vec<_> = (2..=count as u8 + 1)
            .map(|seed| {
                CordialEquivocationEvidence::new(
                    node(seed),
                    0,
                    [0, 1]
                        .map(|branch| signed(seed, vec![branch], HashSet::new()))
                        .to_vec(),
                )
            })
            .collect();
        let carriers: Vec<_> = claims
            .iter()
            .map(|claim| {
                let proof = verify_equivocation_evidence(claim, &Blocklace::new())
                    .expect("authentic conflicting blocks");
                PorEquivocationAuthorization {
                    shard_id: b"root".to_vec(),
                    source_finalized_wave: 0,
                    round: 1,
                    message: checkpoint_equivocation_authorization_payload(b"root", 1, &proof)
                        .expect("checkpoint authorization"),
                }
            })
            .collect();
        let mut ingress = LiveIngress::with_consensus_view(
            AcceptVerifiedBlocks,
            membership,
            CasperShardConf::default(),
            "root",
        );
        let mut predecessors = HashSet::new();
        for round in 0..9 {
            let mut next = HashSet::new();
            for seed in 1..=4 {
                let mut payload = CordialBlockPayload::genesis(vec![]);
                payload.state.block_number = round;
                if round == 1 && seed == 1 {
                    payload.por_authorizations = carriers.clone();
                }
                let block = signed(seed, payload.to_bytes(), predecessors.clone());
                crypto::Secp256k1Scheme
                    .verify_block(
                        &block.content,
                        &block.identity.signature,
                        &block.identity.creator,
                    )
                    .expect("signed history");
                next.insert(block.identity.clone());
                ingress
                    .ingest_trusted_block(block)
                    .expect("verified ingress");
            }
            predecessors = next;
        }
        let output = ingress
            .latest_finalized_ordered_output(3)
            .expect("production finality");
        let checkpoint = finalized_prefix_checkpoint(&output, 1)
            .expect("checkpoint")
            .expect("at least sixteen finalized blocks");
        let lace = ingress.blocklace();
        let membership = ingress.bonds();

        // Complete the real signed rating exchange once, outside timing.
        let directory = tempfile::tempdir().expect("exchange directory");
        let key_path = directory.path().join("key");
        std::fs::write(&key_path, hex::encode([1; 32])).expect("development key file");
        let key = load_bonded_rating_key(&key_path, membership).expect("bonded key");
        let outbox = RatingOutbox::open(&directory.path().join("outbox")).expect("outbox");
        let exchange = FileRatingExchange::open(&directory.path().join("exchange"), b"root")
            .expect("exchange");
        stage_local_checkpoint_rating_batch(
            lace,
            &output,
            &checkpoint,
            &state,
            &config,
            &key,
            &outbox,
        )
        .expect("stage local ratings");
        for seed in 2..=4 {
            let batch = build_finalized_checkpoint_rating_batch(
                lace,
                &output,
                &checkpoint,
                &node(seed),
                &state,
                &config,
                &[seed; 32],
            )
            .expect("signed peer ratings");
            let publisher = exchange
                .checkpoint_broadcaster(checkpoint.rating_source(), node(seed))
                .expect("publisher");
            for rating in batch.ratings {
                publisher
                    .publish(
                        &BlockProductionRatingEnvelopeV2::new(checkpoint.rating_source(), rating)
                            .expect("envelope")
                            .encode(),
                    )
                    .expect("publish fixture rating");
            }
        }
        let completed = close_checkpoint_rating_exchange_if_complete(
            lace,
            &output,
            &checkpoint,
            &state,
            &config,
            &key,
            &membership.keys().cloned().collect(),
            &outbox,
            &exchange,
        )
        .expect("close exchange")
        .expect("all raters complete");
        assert!(!completed.batch().ratings.is_empty());
        let authorized = completed
            .clone()
            .with_finalized_equivocations(lace, &output, membership, b"root")
            .expect("authenticate finalized penalties");
        let penalties = authorized.penalties().expect("penalties present");
        assert_eq!(penalties.equivocations.len(), count);
        let expected_offenders: HashSet<_> =
            claims.iter().map(|claim| claim.validator.clone()).collect();
        assert_eq!(
            penalties
                .equivocations
                .iter()
                .map(|event| event.offender.clone())
                .collect::<HashSet<_>>(),
            expected_offenders
        );
        assert!(
            completed
                .clone()
                .with_authorized_equivocations(&[], lace, &output, membership, b"root",)
                .is_err(),
            "a finalized authorization cannot be omitted"
        );

        let mut invalid_claims = claims.clone();
        invalid_claims[0].blocks[0].identity.signature = vec![0xFF];
        assert!(
            completed
                .clone()
                .with_authorized_equivocations(&invalid_claims, lace, &output, membership, b"root",)
                .is_err(),
            "invalid evidence signatures must not authorize a penalty",
        );

        let previous = ReputationVector {
            round: 0,
            values: state.reputation_list().entries.clone(),
        };
        let ratings = &authorized.batch().ratings;
        let replay = || {
            replay_reputation_transition_with_penalties(
                &previous,
                ratings,
                1,
                &config,
                Some(penalties),
            )
            .expect("authenticated penalty replay")
        };
        let expected = replay();
        for entry in &expected.entries {
            assert_eq!(
                entry.is_excluded,
                expected_offenders.contains(&entry.node_id)
            );
            if entry.is_excluded {
                assert_eq!(entry.reputation, 0);
                // One of four offenders receives the base tier; two or more
                // exceed the default correlation threshold and lose all capital.
                assert_eq!(entry.retained_reputation, if count == 1 { 750 } else { 0 });
            }
        }
        assert_eq!(replay(), expected);
        let context = CheckpointReputationBlockContext {
            shard_id: b"root",
            source: authorized.source(),
            previous_block: None,
        };
        let block = build_checkpoint_reputation_block_with_penalties(
            context,
            authorized.batch(),
            expected,
            &config,
            Some(penalties),
        )
        .expect("v3 checkpoint block");
        let audit = || {
            verify_checkpoint_reputation_transition_with_penalties(
                &previous,
                ratings,
                &block,
                context,
                &config,
                Some(penalties),
            )
            .expect("checkpoint audit")
        };
        audit();
        let mut wrong_context = context;
        wrong_context.source.finalized_prefix_hash[0] ^= 1;
        assert!(
            verify_checkpoint_reputation_transition_with_penalties(
                &previous,
                ratings,
                &block,
                wrong_context,
                &config,
                Some(penalties),
            )
            .is_err(),
            "audit must bind the finalized source"
        );

        group.bench_function(
            BenchmarkId::new("authenticate_finalized_penalties", count),
            |b| {
                b.iter_batched(
                    || completed.clone(),
                    |round| {
                        round
                            .with_finalized_equivocations(lace, &output, membership, b"root")
                            .expect("authenticated penalties")
                    },
                    BatchSize::SmallInput,
                );
            },
        );
        group.bench_function(BenchmarkId::new("penalty_transition_replay", count), |b| {
            b.iter(replay)
        });
        group.bench_function(
            BenchmarkId::new("checkpoint_reputation_audit", count),
            |b| b.iter(audit),
        );
    }
    group.finish();
}
criterion_group!(benches, bench_checkpoint);
criterion_main!(benches);
