//! Weighted finality lookup; graph construction and crypto are excluded.
use cordial_miners_core::bench_support as support;
use cordial_miners_core::consensus::latest_weighted_final_leader;
use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use std::time::Duration;

fn bench_leader_lookup(c: &mut Criterion) {
    for skewed in [false, true] {
        let mut group = c.benchmark_group(if skewed {
            "leader_lookup/skewed_bonds"
        } else {
            "leader_lookup/uniform_bonds"
        });
        group.sample_size(20);
        group.measurement_time(Duration::from_secs(2));
        for waves in support::sizes(&[1, 3], &[1, 3, 10]) {
            for validators in support::sizes(&[4, 10], &[4, 10, 50]) {
                let layers = support::rounds(validators, waves * 3 + 1);
                let dag = support::insert_all(&support::flatten(&layers));
                let mut bonds = support::bonds(validators);
                if skewed {
                    bonds.insert(support::node(0), 1_000);
                }
                let leader = |wave: u64| Some(support::node(wave as usize % validators));
                let expected = layers[(waves - 1) * 3][(waves - 1) % validators]
                    .identity
                    .clone();
                assert_eq!(
                    latest_weighted_final_leader(&dag, 3, &bonds, leader),
                    Some(expected)
                );
                group.bench_function(
                    BenchmarkId::new(format!("w{waves}_v{validators}"), "lookup"),
                    |b| {
                        b.iter(|| latest_weighted_final_leader(&dag, 3, &bonds, leader));
                    },
                );
            }
        }
        group.finish();
    }
}
criterion_group!(benches, bench_leader_lookup);
criterion_main!(benches);
