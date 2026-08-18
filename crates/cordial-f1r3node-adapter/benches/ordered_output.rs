//! Benchmark: ordered output
//!
//! Covers the two adapter-side hot paths in the live mirror loop:
//!
//! 1. **`SharedOrderedOutput::update`** — publishing a new output under
//!    concurrent read contention (simulated with a read-biased workload).
//! 2. **`SharedOrderedOutput::latest`** — reading the latest output.
//! 3. **`OrderedFinalizedOutput` construction and hash extraction** at 100,
//!    1 000, and 10 000 blocks.
//! 4. **JSON serialisation** of `OrderedFinalizedOutput` at 100, 1 000, and
//!    10 000 blocks.
//!
//! Notes on concurrency model
//! ──────────────────────────
//! `SharedOrderedOutput` uses plain `&mut self` / `&self` and is not designed
//! for truly lock-free concurrent access.  The benchmarks simulate the
//! typical access pattern: a single writer interleaved with many reads,
//! using the existing Criterion framework's single-threaded measurement.
//! For a true concurrent benchmark a `RwLock<SharedOrderedOutput>` wrapper
//! would be required; that is noted here for future work.

use cordial_f1r3node_adapter::ordered_output::OrderedFinalizedOutput;
use cordial_f1r3node_adapter::shared_ordered_output::{ReadOrderedOutput, SharedOrderedOutput};
use cordial_miners_core::types::{BlockIdentity, NodeId};

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};

// ── helpers ───────────────────────────────────────────────────────────────────

fn node(id: u8) -> NodeId {
    NodeId(vec![id])
}

fn make_blocks(n: usize) -> Vec<BlockIdentity> {
    (0..n)
        .map(|i| {
            let mut hash = [0u8; 32];
            hash[0..8].copy_from_slice(&(i as u64).to_le_bytes());
            BlockIdentity {
                content_hash: hash,
                creator: node((i % 256) as u8),
                signature: (i as u64).to_le_bytes().to_vec(),
            }
        })
        .collect()
}

/// Build an `OrderedFinalizedOutput` with `n` synthetic blocks.
fn make_output(n: usize) -> OrderedFinalizedOutput {
    let blocks = make_blocks(n);
    let anchor = blocks.last().cloned();
    OrderedFinalizedOutput::new(blocks, anchor, 3, 4, n).with_timestamp(0)
}

// ── benchmarks ────────────────────────────────────────────────────────────────

fn bench_shared_ordered_output_update(c: &mut Criterion) {
    let sizes: &[usize] = &[100, 1_000, 10_000];

    let mut group = c.benchmark_group("ordered_output/shared_update");
    group.sample_size(50);

    for &n in sizes {
        let output = make_output(n);

        group.bench_with_input(
            BenchmarkId::new(format!("n{n}"), ""),
            &output,
            |b, output| {
                b.iter_batched(
                    || {
                        let mut shared = SharedOrderedOutput::new();
                        // Seed with an empty output so subsequent updates always
                        // satisfy the prefix-preservation check.
                        shared
                            .update(OrderedFinalizedOutput::default())
                            .expect("seed should succeed");
                        (shared, output.clone())
                    },
                    |(mut shared, new_output)| {
                        shared.update(new_output).expect("update should succeed");
                    },
                    criterion::BatchSize::SmallInput,
                );
            },
        );
    }
    group.finish();
}

fn bench_shared_ordered_output_latest(c: &mut Criterion) {
    let sizes: &[usize] = &[100, 1_000, 10_000];

    let mut group = c.benchmark_group("ordered_output/shared_latest");
    group.sample_size(50);

    for &n in sizes {
        let output = make_output(n);
        let shared = SharedOrderedOutput::from_output(output);

        group.bench_with_input(
            BenchmarkId::new(format!("n{n}"), ""),
            &shared,
            |b, shared| {
                b.iter(|| shared.latest());
            },
        );
    }
    group.finish();
}

fn bench_json_serialisation(c: &mut Criterion) {
    let sizes: &[usize] = &[100, 1_000, 10_000];

    let mut group = c.benchmark_group("ordered_output/json_serialise");
    group.sample_size(20);

    for &n in sizes {
        let output = make_output(n);

        group.bench_with_input(
            BenchmarkId::new(format!("n{n}"), ""),
            &output,
            |b, output| {
                b.iter(|| serde_json::to_string(output).expect("serialisation should succeed"));
            },
        );
    }
    group.finish();
}

fn bench_block_hashes_extraction(c: &mut Criterion) {
    let sizes: &[usize] = &[100, 1_000, 10_000];

    let mut group = c.benchmark_group("ordered_output/block_hashes");
    group.sample_size(20);

    for &n in sizes {
        let output = make_output(n);

        group.bench_with_input(
            BenchmarkId::new(format!("n{n}"), ""),
            &output,
            |b, output| {
                b.iter(|| output.block_hashes());
            },
        );
    }
    group.finish();
}

fn bench_ordered_output_construction(c: &mut Criterion) {
    let sizes: &[usize] = &[100, 1_000, 10_000];

    let mut group = c.benchmark_group("ordered_output/construction");
    group.sample_size(20);

    for &n in sizes {
        let blocks = make_blocks(n);
        let anchor = blocks.last().cloned();

        group.bench_with_input(
            BenchmarkId::new(format!("n{n}"), ""),
            &blocks,
            |b, blocks| {
                b.iter_batched(
                    || (blocks.clone(), anchor.clone()),
                    |(blocks, anchor)| OrderedFinalizedOutput::new(blocks, anchor, 3, 4, n),
                    criterion::BatchSize::SmallInput,
                );
            },
        );
    }
    group.finish();
}

criterion_group!(
    benches,
    bench_shared_ordered_output_update,
    bench_shared_ordered_output_latest,
    bench_json_serialisation,
    bench_ordered_output_construction,
    bench_block_hashes_extraction,
);
criterion_main!(benches);
