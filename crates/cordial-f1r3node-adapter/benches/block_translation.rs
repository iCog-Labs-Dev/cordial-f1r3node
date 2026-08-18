//! Benchmark: Protobuf ↔ Block translation
//!
//! Measures conversion throughput between internal consensus [`Block`] and
//! f1r3node wire format [`BlockMessage`]:
//!
//! * **`block_to_message`** — translation to protobuf wire model across blocks
//!   containing 0, 10, 100, and 500 deploys.
//! * **`message_to_block`** — decoding and verification from protobuf wire model.

use std::collections::HashSet;

use cordial_f1r3node_adapter::block_translation::{block_to_message, message_to_block};
use cordial_miners_core::Block;
use cordial_miners_core::crypto::hash_content;
use cordial_miners_core::execution::{
    BlockState, Bond as CmBond, CordialBlockPayload, Deploy as CmDeploy,
    ProcessedDeploy as CmProcessed, ProcessedSystemDeploy as CmSystem,
    SignedDeploy as CmSignedDeploy,
};
use cordial_miners_core::types::{BlockContent, BlockIdentity, NodeId};

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
                },
                deployer: vec![0xAA; 32],
                signature: vec![0xBB; 64],
            },
            cost: 100,
            is_failed: false,
        })
        .collect();

    let payload = CordialBlockPayload {
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

    Block {
        identity: BlockIdentity {
            content_hash: hash_content(&content),
            creator: node(1),
            signature: vec![0xFF; 64],
        },
        content,
    }
}

// ── benchmarks ────────────────────────────────────────────────────────────────

fn bench_block_to_message(c: &mut Criterion) {
    let deploy_counts: &[usize] = &[0, 10, 100, 500];

    let mut group = c.benchmark_group("block_translation/block_to_message");
    group.sample_size(20);

    for &n in deploy_counts {
        let block = build_test_block(n);

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
    let deploy_counts: &[usize] = &[0, 10, 100, 500];

    let mut group = c.benchmark_group("block_translation/message_to_block");
    group.sample_size(20);

    for &n in deploy_counts {
        let block = build_test_block(n);
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

criterion_group!(benches, bench_block_to_message, bench_message_to_block,);
criterion_main!(benches);
