//! Benchmark: DAG traversal and reachability primitives
//!
//! Measures performance of fundamental blocklace DAG graph operations:
//!
//! * **`observe` on deep linear chains** — transitive predecessor closure traversal
//!   at depths 100, 1 000, and 5 000.
//! * **`observe` on wide multi-validator DAGs** — transitive closure over multi-round
//!   DAGs with 4, 10, and 50 validators per round.
//! * **`precedes` query** — reachability checks between ancestor pairs vs non-ancestors.
//! * **`is_closed` validation** — closure invariant verification over entire DAGs.

use std::collections::HashSet;

use cordial_miners_core::Block;
use cordial_miners_core::blocklace::Blocklace;
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

fn insert(blocklace: &mut Blocklace, block: &Block) {
    blocklace
        .insert(block.clone(), &MockVerifier)
        .expect("bench block should insert");
}

fn build_linear_chain(n: usize) -> (Blocklace, BlockIdentity, BlockIdentity) {
    let mut blocklace = Blocklace::new();
    let v = node(0);
    let mut prev: Option<Block> = None;
    let mut first_id = None;
    let mut last_id = None;

    for i in 0..n as u64 {
        let preds: HashSet<BlockIdentity> = prev
            .as_ref()
            .map(|b| std::iter::once(b.identity.clone()).collect())
            .unwrap_or_default();
        let block = make_block(&v, i + 1, preds);
        if i == 0 {
            first_id = Some(block.identity.clone());
        }
        last_id = Some(block.identity.clone());
        insert(&mut blocklace, &block);
        prev = Some(block);
    }

    (blocklace, first_id.unwrap(), last_id.unwrap())
}

fn build_wide_dag(num_validators: usize, rounds: usize) -> (Blocklace, BlockIdentity) {
    let validators: Vec<NodeId> = (0..num_validators as u16).map(node).collect();
    let mut blocklace = Blocklace::new();
    let mut prev_round: Vec<Block> = Vec::new();
    let mut tag: u64 = 1;
    let mut tip = None;

    for _ in 0..=rounds {
        let preds: HashSet<BlockIdentity> = prev_round.iter().map(|b| b.identity.clone()).collect();
        let mut this_round = Vec::new();
        for v in &validators {
            let block = make_block(v, tag, preds.clone());
            tag += 1;
            insert(&mut blocklace, &block);
            this_round.push(block);
        }
        tip = this_round.last().map(|b| b.identity.clone());
        prev_round = this_round;
    }

    (blocklace, tip.unwrap())
}

// ── benchmarks ────────────────────────────────────────────────────────────────

fn bench_observe_linear_chain(c: &mut Criterion) {
    let sizes: &[usize] = &[100, 1_000, 5_000];

    let mut group = c.benchmark_group("dag_traversal/observe_linear");
    group.sample_size(20);

    for &n in sizes {
        let (blocklace, _, tip) = build_linear_chain(n);

        group.bench_with_input(
            BenchmarkId::new(format!("depth_{n}"), ""),
            &tip,
            |b, tip| {
                b.iter(|| blocklace.observe(tip));
            },
        );
    }
    group.finish();
}

fn bench_observe_wide_dag(c: &mut Criterion) {
    let validator_counts: &[usize] = &[4, 10, 50];
    const ROUNDS: usize = 10;

    let mut group = c.benchmark_group("dag_traversal/observe_wide_dag");
    group.sample_size(20);

    for &v in validator_counts {
        let (blocklace, tip) = build_wide_dag(v, ROUNDS);

        group.bench_with_input(
            BenchmarkId::new(format!("validators_{v}_rounds_{ROUNDS}"), ""),
            &tip,
            |b, tip| {
                b.iter(|| blocklace.observe(tip));
            },
        );
    }
    group.finish();
}

fn bench_precedes_reachability(c: &mut Criterion) {
    let sizes: &[usize] = &[100, 1_000, 5_000];

    let mut group = c.benchmark_group("dag_traversal/precedes");
    group.sample_size(20);

    for &n in sizes {
        let (blocklace, root, tip) = build_linear_chain(n);

        group.bench_with_input(
            BenchmarkId::new(format!("ancestor_query_n{n}"), ""),
            &(),
            |b, _| {
                b.iter(|| blocklace.precedes(&root, &tip));
            },
        );

        group.bench_with_input(
            BenchmarkId::new(format!("non_ancestor_query_n{n}"), ""),
            &(),
            |b, _| {
                b.iter(|| blocklace.precedes(&tip, &root));
            },
        );
    }
    group.finish();
}

fn bench_is_closed_invariant(c: &mut Criterion) {
    let sizes: &[usize] = &[100, 1_000, 5_000];

    let mut group = c.benchmark_group("dag_traversal/is_closed");
    group.sample_size(20);

    for &n in sizes {
        let (blocklace, _, _) = build_linear_chain(n);

        group.bench_with_input(
            BenchmarkId::new(format!("n{n}"), ""),
            &blocklace,
            |b, bl| {
                b.iter(|| bl.is_closed());
            },
        );
    }
    group.finish();
}

criterion_group!(
    benches,
    bench_observe_linear_chain,
    bench_observe_wide_dag,
    bench_precedes_reachability,
    bench_is_closed_invariant,
);
criterion_main!(benches);
