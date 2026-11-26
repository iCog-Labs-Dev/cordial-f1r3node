//! Non-ideal graph workloads. Fixture labels describe actual graph structure.
use cordial_miners_core::bench_support as support;
use cordial_miners_core::consensus::{
    OrderingCache, latest_weighted_final_leader, weighted_tau, weighted_tau_with_cache,
};
use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use std::collections::HashSet;
use std::time::Duration;

fn bench_scenarios(c: &mut Criterion) {
    let mut group = c.benchmark_group("consensus_scenarios");
    group.sample_size(20);
    group.measurement_time(Duration::from_secs(2));
    for rounds in support::sizes(&[3, 9], &[3, 9, 30]) {
        for scenario in [
            "sparse",
            "missing_leader",
            "equivocation",
            "insufficient_support",
        ] {
            let mut blocks = Vec::new();
            let mut previous = Vec::<cordial_miners_core::Block>::new();
            for round in 0..rounds {
                let mut next = Vec::new();
                for validator in 0..4 {
                    if scenario == "missing_leader" && validator == 0 {
                        continue;
                    }
                    if scenario == "insufficient_support" && validator != 0 {
                        continue;
                    }
                    let predecessors = if scenario == "sparse" {
                        previous
                            .iter()
                            .filter(|b| {
                                b.identity.creator == support::node(validator)
                                    || b.identity.creator == support::node((validator + 1) % 4)
                            })
                            .map(|b| b.identity.clone())
                            .collect()
                    } else {
                        previous.iter().map(|b| b.identity.clone()).collect()
                    };
                    next.push(support::block(
                        support::node(validator),
                        (round * 4 + validator) as u64,
                        predecessors,
                        256,
                    ));
                }
                if scenario == "equivocation" && round == 0 {
                    next.push(support::block(
                        support::node(3),
                        u64::MAX,
                        HashSet::new(),
                        256,
                    ));
                }
                blocks.extend(next.iter().cloned());
                previous = next;
            }
            let dag = support::insert_all(&blocks);
            match scenario {
                "missing_leader" => assert!(dag.blocks_by(&support::node(0)).is_empty()),
                "insufficient_support" => assert_eq!(dag.dom().len(), rounds),
                "equivocation" => assert!(dag.find_equivacators().contains(&support::node(3))),
                "sparse" => assert!(blocks.iter().all(|b| b.content.predecessors.len() <= 2)),
                _ => unreachable!(),
            }
            let bonds = support::bonds(4);
            // A fixed leader isolates absence of that leader in missing_leader.
            let leader = |_| Some(support::node(0));
            let expected = weighted_tau(&dag, 3, &bonds, leader).expect("ordering");
            if scenario == "missing_leader" || scenario == "insufficient_support" {
                assert!(latest_weighted_final_leader(&dag, 3, &bonds, leader).is_none());
                assert!(expected.is_empty());
            }
            let mut cache = OrderingCache::default();
            assert_eq!(
                weighted_tau_with_cache(&dag, 3, &bonds, 0, leader, &mut cache).expect("cached"),
                expected
            );
            group.bench_function(BenchmarkId::new(scenario, rounds), |b| {
                b.iter(|| weighted_tau(&dag, 3, &bonds, leader).expect("ordering"));
            });
        }
    }
    group.finish();
}
criterion_group!(benches, bench_scenarios);
criterion_main!(benches);
