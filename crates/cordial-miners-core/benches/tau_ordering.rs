//! Benchmark: tau ordering
//!
//! Covers two main axes:
//!
//! 1. **`weighted_tau` vs `weighted_tau_with_cache`** — over blocklaces with
//!    1, 3, and 10 finalized waves.
//! 2. **`xsort`** — deterministic topological sort of block sets of sizes
//!    50, 500, and 5 000.

use std::collections::{HashMap, HashSet};

use cordial_miners_core::Block;
use cordial_miners_core::blocklace::Blocklace;
use cordial_miners_core::consensus::{OrderingCache, weighted_tau, weighted_tau_with_cache, xsort};
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

fn build_blocklace(num_validators: usize, waves: u64, wavelength: u64) -> Blocklace {
    let validators: Vec<NodeId> = (0..num_validators as u16).map(node).collect();
    let mut blocklace = Blocklace::new();
    let total_rounds = waves * wavelength;
    let mut prev_round: Vec<Block> = Vec::new();
    let mut tag: u64 = 1;

    for _round in 0..=total_rounds {
        let predecessors: HashSet<BlockIdentity> =
            prev_round.iter().map(|b| b.identity.clone()).collect();
        let mut this_round = Vec::new();
        for v in &validators {
            let block = make_block(v, tag, predecessors.clone());
            tag += 1;
            insert(&mut blocklace, &block);
            this_round.push(block);
        }
        prev_round = this_round;
    }
    blocklace
}

fn uniform_bonds(num_validators: usize) -> HashMap<NodeId, u64> {
    (0..num_validators as u16)
        .map(|id| (node(id), 100))
        .collect()
}

fn leader_fn(num_validators: usize) -> impl Fn(u64) -> Option<NodeId> + Copy {
    let n = num_validators as u16;
    move |wave: u64| Some(node((wave % u64::from(n)) as u16))
}

// Build a flat chain of `n` blocks (block i references block i-1) returned
// as a HashSet so xsort can operate on them.
fn chain_block_set(n: usize) -> HashSet<Block> {
    let v = node(0);
    let mut blocks = Vec::with_capacity(n);
    let mut prev: Option<Block> = None;
    for i in 0..n as u64 {
        let predecessors: HashSet<BlockIdentity> = prev
            .as_ref()
            .map(|b| std::iter::once(b.identity.clone()).collect())
            .unwrap_or_default();
        let block = make_block(&v, i + 1, predecessors);
        prev = Some(block.clone());
        blocks.push(block);
    }
    blocks.into_iter().collect()
}

// ── benchmarks ────────────────────────────────────────────────────────────────

fn bench_weighted_tau(c: &mut Criterion) {
    const WAVELENGTH: u64 = 3;
    const NUM_VALIDATORS: usize = 4;
    let wave_counts: &[u64] = &[1, 3, 10];

    let mut group = c.benchmark_group("tau_ordering/weighted_tau");
    group.sample_size(20);

    for &waves in wave_counts {
        let blocklace = build_blocklace(NUM_VALIDATORS, waves, WAVELENGTH);
        let bonds = uniform_bonds(NUM_VALIDATORS);
        let lf = leader_fn(NUM_VALIDATORS);

        group.bench_with_input(BenchmarkId::new(format!("w{waves}"), ""), &(), |b, _| {
            b.iter(|| weighted_tau(&blocklace, WAVELENGTH, &bonds, lf));
        });
    }
    group.finish();
}

fn bench_weighted_tau_with_cache(c: &mut Criterion) {
    const WAVELENGTH: u64 = 3;
    const NUM_VALIDATORS: usize = 4;
    let wave_counts: &[u64] = &[1, 3, 10];

    let mut group = c.benchmark_group("tau_ordering/weighted_tau_with_cache");
    group.sample_size(20);

    for &waves in wave_counts {
        let blocklace = build_blocklace(NUM_VALIDATORS, waves, WAVELENGTH);
        let bonds = uniform_bonds(NUM_VALIDATORS);
        let lf = leader_fn(NUM_VALIDATORS);
        let mut cache = OrderingCache::default();

        // Warm the cache once before the timed loop.
        let _ = weighted_tau_with_cache(&blocklace, WAVELENGTH, &bonds, 0, lf, &mut cache);

        group.bench_with_input(
            BenchmarkId::new(format!("w{waves}_warm"), ""),
            &(),
            |b, _| {
                b.iter(|| {
                    weighted_tau_with_cache(&blocklace, WAVELENGTH, &bonds, 0, lf, &mut cache)
                });
            },
        );
    }
    group.finish();
}

fn bench_xsort(c: &mut Criterion) {
    let sizes: &[usize] = &[50, 500, 5_000];

    let mut group = c.benchmark_group("tau_ordering/xsort");
    group.sample_size(20);

    for &n in sizes {
        let block_set = chain_block_set(n);

        group.bench_with_input(
            BenchmarkId::new(format!("n{n}"), ""),
            &block_set,
            |b, bs| {
                b.iter(|| xsort(bs).expect("chain has no cycles"));
            },
        );
    }
    group.finish();
}

criterion_group!(
    benches,
    bench_weighted_tau,
    bench_weighted_tau_with_cache,
    bench_xsort,
);
criterion_main!(benches);
