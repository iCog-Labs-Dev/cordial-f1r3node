//! Checkpoint operations and sustained growth with periodic checkpointing.
//! Graph construction/destruction is excluded. Sustained mode includes insertion,
//! payload ownership, prefix materialization and pruning, as a node would.
use cordial_miners_core::bench_support as support;
use cordial_miners_core::blocklace::Blocklace;
use cordial_miners_core::consensus::pruning::{CheckpointGc, checkpoint_after_weighted_finality};
use cordial_miners_core::consensus::weighted_final_leader_for_wave;
use criterion::{BatchSize, BenchmarkId, Criterion, criterion_group, criterion_main};
use std::time::Duration;

fn bench_pruning(c: &mut Criterion) {
    let mut group = c.benchmark_group("pruning_gc");
    group.sample_size(20);
    group.measurement_time(Duration::from_secs(2));
    for waves in support::sizes(&[3, 10], &[3, 10, 30]) {
        let layers = support::rounds(4, waves * 3 + 1);
        let blocks = support::flatten(&layers);
        let checkpoint_wave = waves / 2;
        let checkpoint = layers[checkpoint_wave * 3][checkpoint_wave % 4]
            .identity
            .clone();
        let weights = support::bonds(4);
        for weighted in [false, true] {
            let apply = |dag: &mut Blocklace| {
                if weighted {
                    checkpoint_after_weighted_finality(dag, 3, &weights, support::leader)
                        .expect("weighted checkpoint")
                        .expect("checkpoint advances")
                } else {
                    dag.prune_below_checkpoint(&checkpoint)
                        .expect("explicit checkpoint")
                }
            };
            let mut checked = support::insert_all(&blocks);
            // The low-level pruning API trusts the caller to establish finality.
            assert_eq!(
                weighted_final_leader_for_wave(
                    &checked,
                    checkpoint_wave as u64,
                    3,
                    &weights,
                    support::leader,
                ),
                Some(checkpoint.clone()),
                "explicit checkpoint must be the proven final leader",
            );
            let report = apply(&mut checked);
            assert_eq!(checked.checkpoint(), Some(&report.checkpoint));
            assert_eq!(checked.dom().len(), report.retained_blocks);
            assert_eq!(report.removed.len() + report.retained_blocks, blocks.len());
            assert!(checked.is_closed());
            // Peers at the checkpoint depth still reference its ancestors.
            assert!(
                report.removed.is_empty(),
                "retained peer references protect history"
            );
            assert_eq!(report.retained_blocks, blocks.len());
            group.bench_function(
                BenchmarkId::new(
                    if weighted {
                        "checkpoint_after_weighted_finality_retained_references"
                    } else {
                        "prune_below_checkpoint_retained_references"
                    },
                    blocks.len(),
                ),
                |b| {
                    b.iter_batched_ref(
                        || support::insert_all(&blocks),
                        apply,
                        BatchSize::PerIteration,
                    );
                },
            );
        }
    }

    for cycles in support::sizes(&[3], &[3, 10, 30]) {
        for reclaimable in [false, true] {
            let shape = if reclaimable {
                "reclaimable"
            } else {
                "retained_references"
            };
            let layers = if reclaimable {
                support::checkpoint_rounds(cycles * 3)
            } else {
                support::rounds(4, cycles * 3)
            };
            let inserted: usize = layers.iter().map(Vec::len).sum();
            let weights = support::bonds(4);
            let run = |dag: &mut Blocklace| {
                let mut removed = 0;
                for wave in layers.chunks(3) {
                    for block in wave.iter().flatten() {
                        dag.insert(block.clone(), &support::FixtureVerifier)
                            .expect("growth");
                    }
                    let report =
                        checkpoint_after_weighted_finality(dag, 3, &weights, support::leader)
                            .expect("periodic checkpoint")
                            .expect("checkpoint advances");
                    removed += report.removed.len();
                }
                removed
            };
            let memory_before = std::fs::read_to_string("/proc/self/status").ok();
            let mut checked = Blocklace::new();
            let removed = run(&mut checked);
            let memory_after = std::fs::read_to_string("/proc/self/status").ok();
            for (label, status) in [
                ("before_growth", memory_before),
                ("after_growth", memory_after),
            ] {
                if let Some(status) = status {
                    for line in status
                        .lines()
                        .filter(|line| line.starts_with("VmRSS:") || line.starts_with("VmHWM:"))
                    {
                        eprintln!(
                            "process_memory shape={shape} cycles={cycles} phase={label} {line}; includes allocator, prefixes, fixtures and indexes"
                        );
                    }
                }
            }
            let last_leader =
                &layers[(cycles - 1) * 3][if reclaimable { 0 } else { (cycles - 1) % 4 }];
            assert_eq!(checked.checkpoint(), Some(&last_leader.identity));
            assert!(checked.is_closed());
            assert_eq!(checked.dom().len() + removed, inserted);
            if reclaimable {
                assert_eq!(inserted, cycles * 9);
                assert_eq!(checked.dom().len(), 9, "only the latest wave remains");
                assert_eq!(removed, (cycles - 1) * 9);
                assert!(checked.get(&layers[0][0].identity).is_none());
            } else {
                assert_eq!(inserted, cycles * 12);
                assert_eq!(removed, 0, "peer references keep earlier history alive");
                assert_eq!(checked.dom().len(), inserted);
            }
            for block in layers[(cycles - 1) * 3..].iter().flatten() {
                support::assert_content_eq(
                    &checked
                        .get(&block.identity)
                        .expect("latest wave retained")
                        .content,
                    &block.content,
                );
            }
            let payload_bytes: usize = checked
                .dom()
                .iter()
                .map(|id| checked.get(id).expect("retained").content.payload.len())
                .sum();
            eprintln!(
                "sustained fixture: shape={shape} cycles={cycles} inserted_blocks={inserted} removed_blocks={removed} retained_blocks={} retained_payload_bytes={payload_bytes}; excludes indexes and saved prefix memory",
                checked.dom().len()
            );
            group.bench_function(
                BenchmarkId::new(format!("growth_and_periodic_checkpoint_{shape}"), cycles),
                |b| {
                    b.iter_batched_ref(Blocklace::new, run, BatchSize::PerIteration);
                },
            );
        }
    }
    group.finish();
}
criterion_group!(benches, bench_pruning);
criterion_main!(benches);
