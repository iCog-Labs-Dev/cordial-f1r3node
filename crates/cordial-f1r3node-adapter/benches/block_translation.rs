//! Benchmark: Protobuf ↔ Block translation
//!
//! Measures conversion throughput between internal consensus [`Block`] and
//! f1r3node wire format [`BlockMessage`]:
//!
//! * **`block_to_message`** — translation to protobuf wire model across blocks
//!   containing 0, 10, 100, and 500 deploys.
//! * **`message_to_block`** — conversion from the protobuf model and internal content hashing.
//!   No signature verification, protobuf byte encoding, network or execution is measured.

use cordial_miners_core::bench_support as support;
use std::collections::HashSet;

use cordial_f1r3node_adapter::block_translation::{block_to_message, message_to_block};
use cordial_miners_core::Block;
use cordial_miners_core::crypto::block_hash;
use cordial_miners_core::execution::{
    BlockState, Bond as CmBond, CordialBlockPayload, Deploy as CmDeploy, DeploySignatureAlgorithm,
    ProcessedDeploy as CmProcessed, ProcessedSystemDeploy as CmSystem,
    SignedDeploy as CmSignedDeploy,
};
use cordial_miners_core::types::{BlockContent, BlockIdentity, NodeId};

use std::time::Duration;

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};

// ── helpers ───────────────────────────────────────────────────────────────────

fn node(b: u8) -> NodeId {
    NodeId(vec![b])
}

fn build_test_block(num_deploys: usize) -> Block {
    let deploys: Vec<CmProcessed> = (0..num_deploys)
        .map(|i| CmProcessed {
            deploy: CmSignedDeploy {
                deploy: CmDeploy {
                    term: format!("@0!(\"deploy_payload_{i}\")").into_bytes(),
                    timestamp: 1_700_000_000_000 + (i as u64),
                    phlo_price: 1,
                    phlo_limit: 10_000,
                    valid_after_block_number: 0,
                    shard_id: "root".to_string(),
                    expiration_timestamp: None,
                },
                deployer: vec![0xAA; 32],
                signature: vec![0xBB; 64],
                signature_algorithm: DeploySignatureAlgorithm::Ed25519,
            },
            cost: 100,
            is_failed: false,
        })
        .collect();

    let payload = CordialBlockPayload {
        por_authorizations: vec![],
        state: BlockState {
            pre_state_hash: vec![0x11; 32],
            post_state_hash: vec![0x22; 32],
            bonds: vec![
                CmBond {
                    validator: node(1),
                    stake: 100,
                },
                CmBond {
                    validator: node(2),
                    stake: 200,
                },
            ],
            block_number: 42,
        },
        deploys,
        rejected_deploys: vec![],
        system_deploys: vec![CmSystem::CloseBlock { succeeded: true }],
    };

    let content = BlockContent {
        payload: payload.to_bytes(),
        predecessors: HashSet::new(),
    };

    let creator = node(1);
    let content_hash = block_hash(&creator, &content);

    Block {
        identity: BlockIdentity {
            content_hash,
            creator,
            signature: vec![0xFF; 64],
        },
        content,
    }
}

// ── benchmarks ────────────────────────────────────────────────────────────────

fn bench_block_to_message(c: &mut Criterion) {
    let deploy_counts: &[usize] = if support::smoke() {
        &[0, 10]
    } else {
        &[0, 10, 100, 500]
    };

    let mut group = c.benchmark_group("block_translation/block_to_message");
    group.sample_size(20);
    group.measurement_time(Duration::from_secs(2));

    for &n in deploy_counts {
        let block = build_test_block(n);
        let wire = block_to_message(&block, "root").expect("preflight translation");
        assert_eq!(wire.body.deploys.len(), n);
        let restored = message_to_block(&wire).expect("preflight reverse translation");
        assert_eq!(restored.content.payload, block.content.payload);
        assert_eq!(restored.content.predecessors, block.content.predecessors);
        assert_eq!(restored.identity.content_hash, block.identity.content_hash);

        group.bench_with_input(
            BenchmarkId::new(format!("deploys_{n}"), ""),
            &block,
            |b, blk| {
                b.iter(|| block_to_message(blk, "root").expect("translation must succeed"));
            },
        );
    }
    group.finish();
}

fn bench_message_to_block(c: &mut Criterion) {
    let deploy_counts: &[usize] = if support::smoke() {
        &[0, 10]
    } else {
        &[0, 10, 100, 500]
    };

    let mut group = c.benchmark_group("block_translation/message_to_block");
    group.sample_size(20);
    group.measurement_time(Duration::from_secs(2));

    for &n in deploy_counts {
        let block = build_test_block(n);
        let wire = block_to_message(&block, "root").expect("preflight translation");
        assert_eq!(wire.body.deploys.len(), n);
        let restored = message_to_block(&wire).expect("preflight reverse translation");
        assert_eq!(restored.content.payload, block.content.payload);
        assert_eq!(restored.content.predecessors, block.content.predecessors);
        assert_eq!(restored.identity.content_hash, block.identity.content_hash);
        let msg = block_to_message(&block, "root").expect("setup translation");

        group.bench_with_input(
            BenchmarkId::new(format!("deploys_{n}"), ""),
            &msg,
            |b, m| {
                b.iter(|| message_to_block(m).expect("reverse translation must succeed"));
            },
        );
    }
    group.finish();
}

// The versioned carrier contains real signed proof bytes. Translation only
// preserves them; proof authentication is measured separately in por_checkpoint.
fn bench_authorization_translation(c: &mut Criterion) {
    use cordial_miners_core::{
        Blocklace,
        consensus::{CordialEquivocationEvidence, verify_equivocation_evidence},
        execution::PorEquivocationAuthorization,
    };
    use cordial_por::checkpoint_equivocation_authorization_payload;
    use k256::ecdsa::SigningKey;

    let mut group = c.benchmark_group("block_translation/por_authorizations");
    group.sample_size(20);
    group.measurement_time(Duration::from_secs(2));
    for count in support::sizes(&[1, 4], &[1, 4, 16]) {
        let carriers: Vec<_> = (1..=count)
            .map(|index| {
                let key = SigningKey::from_slice(&[index as u8; 32]).expect("fixture key");
                let blocks = [0, 1].map(|tag| support::signed_block(&key, tag, HashSet::new()));
                let claim = CordialEquivocationEvidence::new(
                    blocks[0].identity.creator.clone(),
                    0,
                    blocks.to_vec(),
                );
                let proof = verify_equivocation_evidence(&claim, &Blocklace::new())
                    .expect("signed equivocation");
                PorEquivocationAuthorization {
                    shard_id: b"root".to_vec(),
                    source_finalized_wave: 0,
                    round: 1,
                    message: checkpoint_equivocation_authorization_payload(b"root", 1, &proof)
                        .expect("encoded authorization"),
                }
            })
            .collect();
        let mut block = build_test_block(10);
        let mut payload = CordialBlockPayload::from_bytes(&block.content.payload).expect("payload");
        payload.por_authorizations = carriers.clone();
        block.content.payload = payload.to_bytes();
        block.identity.content_hash = block_hash(&block.identity.creator, &block.content);
        let wire = block_to_message(&block, "root").expect("versioned transport");
        let restored = message_to_block(&wire).expect("versioned round trip");
        support::assert_content_eq(&restored.content, &block.content);
        assert_eq!(restored.identity, block.identity);
        assert_eq!(
            CordialBlockPayload::from_bytes(&restored.content.payload)
                .expect("decoded payload")
                .por_authorizations,
            carriers
        );
        assert_eq!(
            CordialBlockPayload::decode_por_extra(&wire.body.extra_bytes)
                .expect("transport carrier"),
            carriers
        );
        assert!(!wire.body.extra_bytes.is_empty());
        group.throughput(criterion::Throughput::Bytes(
            wire.body.extra_bytes.len() as u64
        ));
        group.bench_function(BenchmarkId::new("block_to_message", count), |b| {
            b.iter(|| block_to_message(&block, "root").expect("encode carrier"))
        });
        group.bench_function(BenchmarkId::new("message_to_block", count), |b| {
            b.iter(|| message_to_block(&wire).expect("decode carrier"))
        });
    }
    group.finish();
}
criterion_group!(
    benches,
    bench_block_to_message,
    bench_message_to_block,
    bench_authorization_translation
);
criterion_main!(benches);
