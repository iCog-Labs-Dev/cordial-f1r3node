//! Benchmark: proposer dissemination and predecessor selection
//!
//! Measures performance of block proposal construction:
//!
//! * **`select_predecessors`** — collecting honest tips across 4, 20, and 100
//!   bonded validators under `Compatibility` vs `Strict` modes.
//! * **`build_block_candidate`** — full block assembly including payload
//!   packaging, content hashing, and predecessor selection.

use std::collections::{HashMap, HashSet};

use cordial_miners_core::Block;
use cordial_miners_core::blocklace::Blocklace;
use cordial_miners_core::consensus::dissemination::{
    PredecessorSelectionMode, build_block_candidate_with_mode, select_predecessors_with_mode,
};
use cordial_miners_core::crypto::CryptoVerifier;
use cordial_miners_core::types::{BlockContent, BlockIdentity, NodeId};

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};

// ── helpers ───────────────────────────────────────────────────────────────────

struct MockVerifier;

impl CryptoVerifier for MockVerifier {
    type Error = String;

    fn verify_block(
        &self,
        _content: &BlockContent,
        _sig: &[u8],
        _creator: &NodeId,
    ) -> Result<(), Self::Error> {
        Ok(())
    }
}

fn node(id: u16) -> NodeId {
    NodeId(id.to_le_bytes().to_vec())
}

fn make_id(creator: &NodeId, tag: u64) -> BlockIdentity {
    let mut hash = [0u8; 32];
    hash[0..8].copy_from_slice(&tag.to_le_bytes());
    if creator.0.len() >= 2 {
        hash[8] = creator.0[0];
        hash[9] = creator.0[1];
    }
    BlockIdentity {
        content_hash: hash,
        creator: creator.clone(),
        signature: tag.to_le_bytes().to_vec(),
    }
}

fn make_block(creator: &NodeId, tag: u64, predecessors: HashSet<BlockIdentity>) -> Block {
    Block {
        identity: make_id(creator, tag),
        content: BlockContent {
            payload: tag.to_le_bytes().to_vec(),
            predecessors,
        },
    }
}

fn build_dag_with_validator_count(
    num_validators: usize,
    rounds: usize,
) -> (Blocklace, HashMap<NodeId, u64>) {
    let validators: Vec<NodeId> = (0..num_validators as u16).map(node).collect();
    let bonds: HashMap<NodeId, u64> = validators.iter().map(|v| (v.clone(), 100)).collect();
    let mut blocklace = Blocklace::new();
    let mut prev_round: Vec<Block> = Vec::new();
    let mut tag: u64 = 1;

    for _ in 0..rounds {
        let preds: HashSet<BlockIdentity> = prev_round.iter().map(|b| b.identity.clone()).collect();
        let mut this_round = Vec::new();
        for v in &validators {
            let block = make_block(v, tag, preds.clone());
            tag += 1;
            blocklace
                .insert(block.clone(), &MockVerifier)
                .expect("block insert");
            this_round.push(block);
        }
        prev_round = this_round;
    }

    (blocklace, bonds)
}

// ── benchmarks ────────────────────────────────────────────────────────────────

fn bench_select_predecessors(c: &mut Criterion) {
    let validator_counts: &[usize] = &[4, 20, 100];
    const ROUNDS: usize = 5;

    let mut group = c.benchmark_group("proposer_selection/select_predecessors");
    group.sample_size(20);

    for &v in validator_counts {
        let (blocklace, bonds) = build_dag_with_validator_count(v, ROUNDS);

        group.bench_with_input(
            BenchmarkId::new(format!("compat_validators_{v}"), ""),
            &(),
            |b, _| {
                b.iter(|| {
                    select_predecessors_with_mode(
                        &blocklace,
                        &bonds,
                        PredecessorSelectionMode::Compatibility,
                    )
                });
            },
        );

        group.bench_with_input(
            BenchmarkId::new(format!("strict_validators_{v}"), ""),
            &(),
            |b, _| {
                b.iter(|| {
                    select_predecessors_with_mode(
                        &blocklace,
                        &bonds,
                        PredecessorSelectionMode::Strict,
                    )
                });
            },
        );
    }
    group.finish();
}

fn bench_build_block_candidate(c: &mut Criterion) {
    let validator_counts: &[usize] = &[4, 20, 100];
    const ROUNDS: usize = 5;

    let mut group = c.benchmark_group("proposer_selection/build_block_candidate");
    group.sample_size(20);

    for &v in validator_counts {
        let (blocklace, bonds) = build_dag_with_validator_count(v, ROUNDS);
        let payload = vec![0xAB; 256];

        group.bench_with_input(
            BenchmarkId::new(format!("validators_{v}"), ""),
            &(),
            |b, _| {
                b.iter(|| {
                    build_block_candidate_with_mode(
                        &blocklace,
                        &bonds,
                        payload.clone(),
                        PredecessorSelectionMode::Compatibility,
                    )
                });
            },
        );
    }
    group.finish();
}

criterion_group!(
    benches,
    bench_select_predecessors,
    bench_build_block_candidate,
);
criterion_main!(benches);
