//! Benchmark: height bootstrap
//!
//! Measures the cost of replaying a large block history into a blocklace
//! mirror — the same path taken during node startup.
//!
//! Two ingestion modes are benchmarked:
//!
//! * **strict ingest** — normal live ingestion, predecessors must be present.
//! * **window ingest** — drops unknown predecessors, used when starting from a
//!   mid-chain snapshot (mirrors `ingest_with_trusted_boundary`).
//!
//! Block counts: 100, 1 000, and 5 000.
//!
//! We also benchmark the DAG-height topological sort used during LMDB
//! recovery (`topo_sort_blocks` equivalent) at sizes 1 000 and 10 000.

use std::collections::HashSet;

use cordial_miners_core::Block;
use cordial_miners_core::blocklace::Blocklace;
use cordial_miners_core::crypto::CryptoVerifier;
use cordial_miners_core::types::{BlockContent, BlockIdentity, NodeId};

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};

// ── shared helpers ────────────────────────────────────────────────────────────

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

/// Build a linear chain of `n` blocks authored by a round-robin of 4
/// validators. Each block references only its direct predecessor, giving a
/// minimal but valid DAG that exercises the full ingestion path.
fn build_chain(n: usize) -> Vec<Block> {
    let validators: Vec<NodeId> = (0..4u16).map(node).collect();
    let mut blocks = Vec::with_capacity(n);
    let mut prev: Option<Block> = None;

    for i in 0..n as u64 {
        let v = &validators[(i as usize) % validators.len()];
        let predecessors: HashSet<BlockIdentity> = prev
            .as_ref()
            .map(|b| std::iter::once(b.identity.clone()).collect())
            .unwrap_or_default();
        let block = make_block(v, i + 1, predecessors);
        prev = Some(block.clone());
        blocks.push(block);
    }
    blocks
}

// ── SimpleMirror ─────────────────────────────────────────────────────────────
//
// Re-implements the two hot ingestion paths of LiveBlocklaceMirror using only
// cordial-miners-core types, keeping this benchmark self-contained without
// pulling in the heavy f1r3node external deps of the adapter crate.

struct SimpleMirror {
    blocklace: Blocklace,
}

impl SimpleMirror {
    fn new() -> Self {
        Self {
            blocklace: Blocklace::new(),
        }
    }

    /// Strict ingest: block must have all predecessors already present.
    fn ingest(&mut self, block: Block) -> Result<(), String> {
        self.blocklace
            .insert(block, &MockVerifier)
            .map_err(|e| format!("{e:?}"))
    }

    /// Window ingest: drop unknown predecessors before inserting.
    fn ingest_with_trusted_boundary(&mut self, mut block: Block) -> Result<(), String> {
        let known: HashSet<BlockIdentity> = self.blocklace.dom().into_iter().cloned().collect();
        block
            .content
            .predecessors
            .retain(|pred_id| known.contains(pred_id));
        self.blocklace
            .insert(block, &MockVerifier)
            .map_err(|e| format!("{e:?}"))
    }
}

// ── topo_sort_blocks — iterative local copy ───────────────────────────────────
//
// The real implementation in cordial-f1r3space-adapter uses recursive DFS
// height computation which stack-overflows on chains of ~8000+ blocks.
// This benchmark copy uses an equivalent iterative Kahn's-BFS approach.

fn topo_sort_blocks(mut blocks: Vec<Block>) -> Vec<Block> {
    use std::collections::{HashMap, VecDeque};

    if blocks.is_empty() {
        return blocks;
    }

    // Build index: content_hash → position in `blocks`
    let hash_to_pos: HashMap<[u8; 32], usize> = blocks
        .iter()
        .enumerate()
        .map(|(i, b)| (b.identity.content_hash, i))
        .collect();

    let n = blocks.len();

    // Build in-degree and reverse adjacency list for blocks within the set.
    let mut in_degree: Vec<u64> = vec![0; n];
    let mut dependents: Vec<Vec<usize>> = vec![Vec::new(); n];

    for (i, block) in blocks.iter().enumerate() {
        for pred_id in &block.content.predecessors {
            if let Some(&pred_pos) = hash_to_pos.get(&pred_id.content_hash) {
                in_degree[i] += 1;
                dependents[pred_pos].push(i);
            }
        }
    }

    // BFS from roots (in_degree == 0) to compute height iteratively.
    let mut height: Vec<u64> = vec![0; n];
    let mut rem_in_degree = in_degree;
    let mut queue: VecDeque<usize> = (0..n).filter(|&i| rem_in_degree[i] == 0).collect();

    while let Some(pos) = queue.pop_front() {
        let h = height[pos];
        for &dep in &dependents[pos] {
            if height[dep] < h + 1 {
                height[dep] = h + 1;
            }
            rem_in_degree[dep] -= 1;
            if rem_in_degree[dep] == 0 {
                queue.push_back(dep);
            }
        }
    }

    // Stable sort ascending by height — predecessors always before successors.
    let heights_copy = height;
    blocks.sort_by_key(|b| {
        hash_to_pos
            .get(&b.identity.content_hash)
            .map(|&i| heights_copy[i])
            .unwrap_or(0)
    });
    blocks
}

// ── benchmarks ────────────────────────────────────────────────────────────────

fn bench_ingest(c: &mut Criterion) {
    let sizes: &[usize] = &[100, 1_000, 5_000];

    let mut group = c.benchmark_group("height_bootstrap/ingest");
    group.sample_size(10);

    for &n in sizes {
        let chain = build_chain(n);

        group.bench_with_input(BenchmarkId::new(format!("n{n}"), ""), &chain, |b, chain| {
            b.iter_batched(
                || (SimpleMirror::new(), chain.clone()),
                |(mut mirror, blocks)| {
                    for block in blocks {
                        mirror.ingest(block).expect("chain ingest should succeed");
                    }
                },
                criterion::BatchSize::SmallInput,
            );
        });
    }
    group.finish();
}

fn bench_ingest_with_trusted_boundary(c: &mut Criterion) {
    let sizes: &[usize] = &[100, 1_000, 5_000];

    let mut group = c.benchmark_group("height_bootstrap/ingest_with_trusted_boundary");
    group.sample_size(10);

    for &n in sizes {
        let chain = build_chain(n);

        group.bench_with_input(BenchmarkId::new(format!("n{n}"), ""), &chain, |b, chain| {
            b.iter_batched(
                || (SimpleMirror::new(), chain.clone()),
                |(mut mirror, blocks)| {
                    for block in blocks {
                        mirror
                            .ingest_with_trusted_boundary(block)
                            .expect("window ingest should succeed");
                    }
                },
                criterion::BatchSize::SmallInput,
            );
        });
    }
    group.finish();
}

fn bench_topo_sort_blocks(c: &mut Criterion) {
    let sizes: &[usize] = &[1_000, 10_000];

    let mut group = c.benchmark_group("height_bootstrap/topo_sort_blocks");
    group.sample_size(10);

    for &n in sizes {
        let chain = build_chain(n);

        group.bench_with_input(BenchmarkId::new(format!("n{n}"), ""), &chain, |b, chain| {
            b.iter_batched(
                || chain.clone(),
                topo_sort_blocks,
                criterion::BatchSize::SmallInput,
            );
        });
    }
    group.finish();
}

criterion_group!(
    benches,
    bench_ingest,
    bench_ingest_with_trusted_boundary,
    bench_topo_sort_blocks,
);
criterion_main!(benches);
