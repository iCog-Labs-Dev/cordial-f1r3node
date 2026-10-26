//! Production PoR math and deterministic replay. Rating authentication is an
//! upstream trust boundary: fixture signature bytes are not verified here.
use cordial_miners_core::bench_support as support;
use cordial_por::{
    PorConfig, RatingRecord, ReputationEntry, ReputationVector, build_rating_batch,
    build_rating_matrix, compute_liquid_rank_contribution, normalize_rating_matrix,
    replay_reputation_transition,
};
use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use std::time::Duration;

fn bench_reputation(c: &mut Criterion) {
    let config = PorConfig::default();
    let mut group = c.benchmark_group("por");
    group.sample_size(20);
    group.measurement_time(Duration::from_secs(2));
    for n in support::sizes(&[10, 100], &[10, 100, 1_000]) {
        let previous = ReputationVector {
            round: 0,
            values: (0..n)
                .map(|i| ReputationEntry::new(support::node(i), config.initial_reputation))
                .collect(),
        };
        // Four sparse, distinct incoming ratings per participant; no self-rating.
        let ratings: Vec<_> = (0..n)
            .flat_map(|recipient| {
                let config = &config;
                (1..=4).map(move |offset| {
                    RatingRecord::new(
                        1,
                        support::node((recipient + offset) % n),
                        support::node(recipient),
                        config.maximum_rating,
                        vec![1; 70],
                    )
                })
            })
            .collect();
        let batch = build_rating_batch(1, ratings.clone(), &config).expect("batch");
        let matrix = build_rating_matrix(&batch).expect("matrix");
        let normalized = normalize_rating_matrix(&matrix, &config).expect("normalize");
        let contribution =
            compute_liquid_rank_contribution(&normalized, &previous, &config).expect("rank");
        assert_eq!(normalized.ratings.len(), n * 4);
        assert_eq!(contribution.values.len(), n);
        let expected =
            replay_reputation_transition(&previous, &ratings, 1, &config).expect("replay");
        assert_eq!(expected.entries.len(), n);
        assert_eq!(
            replay_reputation_transition(&previous, &ratings, 1, &config).expect("repeat"),
            expected
        );
        group.throughput(Throughput::Elements(ratings.len() as u64));
        group.bench_function(BenchmarkId::new("normalization", n), |b| {
            b.iter(|| normalize_rating_matrix(&matrix, &config).expect("normalize"))
        });
        group.bench_function(BenchmarkId::new("liquid_rank", n), |b| {
            b.iter(|| {
                compute_liquid_rank_contribution(&normalized, &previous, &config).expect("rank")
            })
        });
        group.bench_function(BenchmarkId::new("transition_replay", n), |b| {
            b.iter(|| {
                replay_reputation_transition(&previous, &ratings, 1, &config).expect("replay")
            })
        });
    }
    group.finish();
}
criterion_group!(benches, bench_reputation);
criterion_main!(benches);
