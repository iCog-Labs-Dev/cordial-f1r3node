//! Production Blocklace ingestion. Adapter startup is in startup_recovery.
//! Fixture cloning and graph destruction are outside the measured routine.
use cordial_miners_core::bench_support as support;
use cordial_miners_core::blocklace::Blocklace;
use criterion::{BatchSize, BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use std::time::Duration;

fn bench_ingest(c: &mut Criterion) {
    let mut group = c.benchmark_group("height_bootstrap");
    group.sample_size(10);
    group.measurement_time(Duration::from_secs(2));
    for n in support::sizes(&[100, 1_000], &[100, 1_000, 5_000]) {
        let history = support::chain(n + 1, 256);
        for trusted in [false, true] {
            // The window really has a missing predecessor; content/signatures
            // are preserved by the production trusted-boundary API.
            let blocks = if trusted {
                &history[1..]
            } else {
                &history[..n]
            };
            let populate = |dag: &mut Blocklace, values: Vec<cordial_miners_core::Block>| {
                for block in values {
                    if trusted {
                        dag.insert_with_trusted_boundary(block, &support::FixtureVerifier)
                            .map(|_| ())
                    } else {
                        dag.insert(block, &support::FixtureVerifier)
                    }
                    .expect("ingestion");
                }
            };
            let mut checked = Blocklace::new();
            populate(&mut checked, blocks.to_vec());
            assert_eq!(checked.dom().len(), n);
            if trusted {
                assert!(!checked.is_closed());
                assert_eq!(
                    checked
                        .trusted_boundary_predecessors(&blocks[0].identity)
                        .expect("boundary recorded")
                        .len(),
                    1
                );
            } else {
                assert!(checked.is_closed());
            }
            for block in blocks {
                support::assert_content_eq(
                    &checked.get(&block.identity).expect("restored").content,
                    &block.content,
                );
            }
            group.throughput(Throughput::Elements(n as u64));
            group.bench_function(
                BenchmarkId::new(if trusted { "trusted_window" } else { "strict" }, n),
                |b| {
                    b.iter_batched_ref(
                        || (Blocklace::new(), blocks.to_vec()),
                        |(dag, blocks)| {
                            for block in blocks.drain(..) {
                                if trusted {
                                    dag.insert_with_trusted_boundary(
                                        block,
                                        &support::FixtureVerifier,
                                    )
                                    .map(|_| ())
                                } else {
                                    dag.insert(block, &support::FixtureVerifier)
                                }
                                .expect("ingestion");
                            }
                        },
                        BatchSize::PerIteration,
                    );
                },
            );
        }
    }
    group.finish();
}
criterion_group!(benches, bench_ingest);
criterion_main!(benches);
