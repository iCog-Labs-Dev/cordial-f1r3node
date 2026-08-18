//! Benchmark: LMDB block persistence and recovery
//!
//! Measures disk storage I/O and recovery performance:
//!
//! * **`put_block`** — persisting raw blocks to LMDB `cordial-blocks`.
//! * **`get_block`** — random access block lookups from LMDB.
//! * **`recover_into_engine`** — full startup recovery scanning LMDB,
//!   sorting by DAG height, and replaying into `Blocklace`.

use std::collections::HashSet;

use cordial_f1r3node_adapter::repository::{BlocklaceRepository, RSpaceBlocklaceRepository};
use cordial_miners_core::Block;
use cordial_miners_core::blocklace::Blocklace;
use cordial_miners_core::crypto::CryptoVerifier;
use cordial_miners_core::types::{BlockContent, BlockIdentity, NodeId};
use tempfile::tempdir;

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};

const MAP_SIZE: usize = 50 * 1024 * 1024; // 50MB for benchmark tempdb

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

fn node(id: u8) -> NodeId {
    NodeId(vec![id])
}

fn make_id(creator: &NodeId, tag: u64) -> BlockIdentity {
    let mut hash = [0u8; 32];
    hash[0..8].copy_from_slice(&tag.to_le_bytes());
    hash[8] = creator.0[0];

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

fn build_chain_blocks(n: usize) -> Vec<Block> {
    let mut blocks = Vec::with_capacity(n);
    let mut prev: Option<Block> = None;

    for i in 0..n as u64 {
        let v = node((i % 4) as u8);
        let preds: HashSet<BlockIdentity> = prev
            .as_ref()
            .map(|b| std::iter::once(b.identity.clone()).collect())
            .unwrap_or_default();
        let block = make_block(&v, i + 1, preds);
        prev = Some(block.clone());
        blocks.push(block);
    }
    blocks
}

// ── benchmarks ────────────────────────────────────────────────────────────────

fn bench_lmdb_put_blocks(c: &mut Criterion) {
    let sizes: &[usize] = &[100, 500, 1_000];

    let mut group = c.benchmark_group("lmdb_persistence/put_blocks");
    group.sample_size(10);

    for &n in sizes {
        let blocks = build_chain_blocks(n);

        group.bench_with_input(
            BenchmarkId::new(format!("n{n}"), ""),
            &blocks,
            |b, blocks| {
                b.iter_batched(
                    || {
                        let dir = tempdir().expect("tempdir");
                        let repo = RSpaceBlocklaceRepository::open(dir.path(), MAP_SIZE)
                            .expect("open repo");
                        (dir, repo)
                    },
                    |(_dir, repo)| {
                        for block in blocks {
                            repo.put_block(block).expect("put_block");
                        }
                    },
                    criterion::BatchSize::SmallInput,
                );
            },
        );
    }
    group.finish();
}

fn bench_lmdb_get_blocks(c: &mut Criterion) {
    let sizes: &[usize] = &[100, 500, 1_000];

    let mut group = c.benchmark_group("lmdb_persistence/get_blocks");
    group.sample_size(10);

    for &n in sizes {
        let blocks = build_chain_blocks(n);
        let dir = tempdir().expect("tempdir");
        let repo = RSpaceBlocklaceRepository::open(dir.path(), MAP_SIZE).expect("open repo");
        for block in &blocks {
            repo.put_block(block).expect("put_block");
        }

        group.bench_with_input(
            BenchmarkId::new(format!("n{n}"), ""),
            &(&repo, &blocks),
            |b, (repo, blocks)| {
                b.iter(|| {
                    for block in *blocks {
                        let _ = repo.get_block(&block.identity).expect("get_block");
                    }
                });
            },
        );
    }
    group.finish();
}

fn bench_lmdb_recover_into_engine(c: &mut Criterion) {
    let sizes: &[usize] = &[100, 500, 1_000];

    let mut group = c.benchmark_group("lmdb_persistence/recover_into_engine");
    group.sample_size(10);

    for &n in sizes {
        let blocks = build_chain_blocks(n);
        let dir = tempdir().expect("tempdir");
        let repo = RSpaceBlocklaceRepository::open(dir.path(), MAP_SIZE).expect("open repo");
        for block in &blocks {
            repo.put_block(block).expect("put_block");
        }

        group.bench_with_input(BenchmarkId::new(format!("n{n}"), ""), &repo, |b, repo| {
            b.iter_batched(
                Blocklace::new,
                |mut engine| {
                    repo.recover_into_engine(&mut engine, &MockVerifier)
                        .expect("recovery must succeed");
                },
                criterion::BatchSize::SmallInput,
            );
        });
    }
    group.finish();
}

criterion_group!(
    benches,
    bench_lmdb_put_blocks,
    bench_lmdb_get_blocks,
    bench_lmdb_recover_into_engine,
);
criterion_main!(benches);
