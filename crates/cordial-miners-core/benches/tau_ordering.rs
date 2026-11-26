//! Ordering with cold/warm caches and invalidation after structural/weight changes.
//! Mutation happens in setup: changing-state cases time the first ordering call
//! after that change, excluding mutation, fixture construction and cache cloning.
use cordial_miners_core::Block;
use cordial_miners_core::bench_support as support;
use cordial_miners_core::blocklace::Blocklace;
use cordial_miners_core::consensus::pruning::checkpoint_after_weighted_finality;
use cordial_miners_core::consensus::{
    OrderingCache, WeightedEpochBoundary, latest_weighted_final_leader, weighted_tau,
    weighted_tau_after_epoch_boundary, weighted_tau_with_cache, xsort,
};
use cordial_miners_core::types::{BlockIdentity, NodeId};
use criterion::{BatchSize, BenchmarkId, Criterion, criterion_group, criterion_main};
use std::collections::{HashMap, HashSet};
use std::time::Duration;

fn order(dag: &Blocklace, bonds: &HashMap<NodeId, u64>) -> Vec<BlockIdentity> {
    weighted_tau(dag, 3, bonds, support::leader).expect("ordering succeeds")
}
fn cached(
    dag: &Blocklace,
    bonds: &HashMap<NodeId, u64>,
    cache: &mut OrderingCache,
) -> Vec<BlockIdentity> {
    weighted_tau_with_cache(dag, 3, bonds, 0, support::leader, cache)
        .expect("cached ordering succeeds")
}
fn check_order(dag: &Blocklace, result: &[BlockIdentity]) {
    assert!(!result.is_empty());
    let unique: HashSet<_> = result.iter().collect();
    assert_eq!(unique.len(), result.len());
    let positions: HashMap<_, _> = result.iter().enumerate().map(|(i, id)| (id, i)).collect();
    for id in result {
        if let Some(block) = dag.get(id) {
            for predecessor in &block.content.predecessors {
                if let Some(position) = positions.get(predecessor) {
                    assert!(
                        position < &positions[id],
                        "predecessors precede descendants"
                    );
                }
            }
        }
    }
}

fn bench_ordering(c: &mut Criterion) {
    let mut group = c.benchmark_group("tau_ordering");
    group.sample_size(20);
    group.measurement_time(Duration::from_secs(2));
    for waves in support::sizes(&[1, 3], &[1, 3, 10]) {
        let layers = support::rounds(4, waves * 3 + 4);
        let blocks = support::flatten(&layers[..waves * 3]);
        let dag = support::insert_all(&blocks);
        let bonds = support::bonds(4);
        let expected = order(&dag, &bonds);
        check_order(&dag, &expected);
        let anchor = latest_weighted_final_leader(&dag, 3, &bonds, support::leader)
            .expect("finalized leader");
        assert_eq!(anchor, layers[(waves - 1) * 3][(waves - 1) % 4].identity);
        let mut warm = OrderingCache::default();
        assert_eq!(cached(&dag, &bonds, &mut warm), expected);
        group.bench_function(BenchmarkId::new("weighted_tau", waves), |b| {
            b.iter(|| order(&dag, &bonds))
        });
        group.bench_function(BenchmarkId::new("cache_warm", waves), |b| {
            b.iter(|| cached(&dag, &bonds, &mut warm))
        });
        group.bench_function(BenchmarkId::new("cache_cold", waves), |b| {
            b.iter_batched_ref(
                OrderingCache::default,
                |cache| cached(&dag, &bonds, cache),
                BatchSize::SmallInput,
            );
        });

        for case in [
            "new_block",
            "new_finalized_leader",
            "checkpoint_retained_references",
            "weight_change",
        ] {
            let prepare = || {
                let mut next = support::insert_all(&blocks);
                let mut cache = OrderingCache::default();
                assert_eq!(cached(&next, &bonds, &mut cache), expected);
                let mut weights = bonds.clone();
                match case {
                    "new_block" => {
                        next.insert(layers[waves * 3][0].clone(), &support::FixtureVerifier)
                            .expect("new block");
                    }
                    "new_finalized_leader" => {
                        for block in support::flatten(&layers[waves * 3..waves * 3 + 3]) {
                            next.insert(block, &support::FixtureVerifier)
                                .expect("next wave");
                        }
                    }
                    "checkpoint_retained_references" => {
                        let report = checkpoint_after_weighted_finality(
                            &mut next,
                            3,
                            &weights,
                            support::leader,
                        )
                        .expect("checkpoint")
                        .expect("checkpoint advances");
                        assert!(report.removed.is_empty(), "peer references retain history");
                        assert_eq!(report.retained_blocks, blocks.len());
                    }
                    "weight_change" => {
                        weights.insert(support::node(0), 200);
                    }
                    _ => unreachable!(),
                }
                (next, weights, cache)
            };
            let (next, weights, mut cache) = prepare();
            let after = order(&next, &weights);
            check_order(&next, &after);
            assert_eq!(cached(&next, &weights, &mut cache), after);
            if case == "new_finalized_leader" {
                assert!(after.starts_with(&expected));
                assert!(after.len() > expected.len());
            } else if case == "new_block" || case == "checkpoint_retained_references" {
                assert_eq!(after, expected);
            }
            group.bench_function(BenchmarkId::new(case, waves), |b| {
                b.iter_batched_ref(
                    prepare,
                    |(next, weights, cache)| cached(next, weights, cache),
                    BatchSize::PerIteration,
                );
            });
        }
        // Raw weight_change above measures cache invalidation only. Production
        // future-only epochs must retain the prefix sealed under the old weights.
        let mut new_weights = bonds.clone();
        new_weights.insert(support::node(0), 200);
        for finalized in [false, true] {
            let end = waves * 3 + if finalized { 3 } else { 1 };
            let next = support::insert_all(&support::flatten(&layers[..end]));
            let replay = || {
                weighted_tau_after_epoch_boundary(
                    &next,
                    3,
                    &bonds,
                    &new_weights,
                    WeightedEpochBoundary {
                        final_leader: &anchor,
                        ordered_prefix: &expected,
                        effective_wave: waves as u64,
                    },
                    support::leader,
                )
                .expect("future-only epoch replay")
            };
            let (after, final_leader) = replay();
            check_order(&next, &after);
            assert!(after.starts_with(&expected));
            if finalized {
                assert!(after.len() > expected.len());
                assert_eq!(final_leader, layers[waves * 3][waves % 4].identity);
            } else {
                assert_eq!(after, expected);
                assert_eq!(final_leader, anchor);
            }
            group.bench_function(
                BenchmarkId::new(
                    if finalized {
                        "future_weight_epoch_finalized"
                    } else {
                        "future_weight_epoch_pending"
                    },
                    waves,
                ),
                |b| b.iter(replay),
            );
        }
    }
    // A separate shape is needed for actual reclamation: peers at a
    // checkpoint's own depth would otherwise protect the entire old history.
    for waves in support::sizes(&[2, 3], &[2, 3, 10]) {
        let layers = support::checkpoint_rounds((waves + 1) * 3);
        let blocks = support::flatten(&layers[..waves * 3]);
        let bonds = support::bonds(4);
        let expected = order(&support::insert_all(&blocks), &bonds);
        let checkpoint = layers[(waves - 1) * 3][0].identity.clone();
        let prepare = || {
            let mut dag = support::insert_all(&blocks);
            let mut cache = OrderingCache::default();
            assert_eq!(cached(&dag, &bonds, &mut cache), expected);
            let before = dag.dom().len();
            let report = checkpoint_after_weighted_finality(&mut dag, 3, &bonds, support::leader)
                .expect("reclaim history")
                .expect("checkpoint advances");
            assert_eq!(report.checkpoint, checkpoint);
            assert!(
                !report.removed.is_empty(),
                "fixture must actually remove blocks"
            );
            assert_eq!(report.removed.len(), (waves - 1) * 9);
            assert_eq!(report.retained_blocks, 9);
            assert_eq!(dag.dom().len() + report.removed.len(), before);
            assert!(report.removed.iter().all(|id| dag.get(id).is_none()));
            assert!(dag.is_closed());
            (dag, cache)
        };
        let (checked, mut cache) = prepare();
        let after = order(&checked, &bonds);
        check_order(&checked, &after);
        assert_eq!(after, expected, "reclamation preserves finalized output");
        assert_eq!(cached(&checked, &bonds, &mut cache), after);
        group.bench_function(BenchmarkId::new("pruning_reclaims_history", waves), |b| {
            b.iter_batched_ref(
                prepare,
                |(dag, cache)| cached(dag, &bonds, cache),
                BatchSize::PerIteration,
            );
        });

        // Exercise continuation across the pruned checkpoint floor: the next
        // final leader must extend exactly the unpruned reference prefix.
        let continued_expected = order(&support::insert_all(&support::flatten(&layers)), &bonds);
        let prepare_continued = || {
            let (mut dag, mut cache) = prepare();
            assert_eq!(cached(&dag, &bonds, &mut cache), expected);
            for block in layers[waves * 3..].iter().flatten() {
                dag.insert(block.clone(), &support::FixtureVerifier)
                    .expect("append after checkpoint");
            }
            (dag, cache)
        };
        let (continued, mut continued_cache) = prepare_continued();
        let after = order(&continued, &bonds);
        assert_eq!(after, continued_expected);
        assert!(after.starts_with(&expected));
        assert!(after.len() > expected.len());
        check_order(&continued, &after);
        assert_eq!(cached(&continued, &bonds, &mut continued_cache), after);
        group.bench_function(
            BenchmarkId::new("new_finalized_leader_after_pruning", waves),
            |b| {
                b.iter_batched_ref(
                    prepare_continued,
                    |(dag, cache)| cached(dag, &bonds, cache),
                    BatchSize::PerIteration,
                );
            },
        );
    }
    group.finish();

    let mut group = c.benchmark_group("tau_ordering/xsort");
    group.sample_size(20);
    group.measurement_time(Duration::from_secs(2));
    for n in support::sizes(&[50, 500], &[50, 500, 2_000]) {
        let chain = support::chain(n, 8);
        let expected: Vec<_> = chain.iter().map(|block| block.identity.clone()).collect();
        let blocks: HashSet<Block> = chain.into_iter().collect();
        assert_eq!(xsort(&blocks).expect("sort"), expected);
        group.bench_function(BenchmarkId::from_parameter(n), |b| {
            b.iter(|| xsort(&blocks).expect("acyclic"))
        });
    }
    group.finish();
}
criterion_group!(benches, bench_ordering);
criterion_main!(benches);
