//! Real and structural validation over exact four-validator DAG sizes.
//! Key derivation, signing, fixture verification and graph construction are untimed.
use cordial_miners_core::Block;
use cordial_miners_core::bench_support as support;
use cordial_miners_core::blocklace::Blocklace;
use cordial_miners_core::consensus::validation::{
    InvalidBlock, ValidationConfig, validate_block, validated_insert,
};
use cordial_miners_core::types::{BlockIdentity, NodeId};
use criterion::{BatchSize, BenchmarkId, Criterion, criterion_group, criterion_main};
use std::cell::OnceCell;
use std::collections::HashMap;
use std::hint::black_box;
use std::time::Duration;

fn fresh_history(
    dag: &Blocklace,
    ordered: &[BlockIdentity],
    bonds: &HashMap<NodeId, u64>,
    candidate: &Block,
) -> (Blocklace, HashMap<NodeId, u64>, Option<Block>) {
    let mut fresh = Blocklace::new();
    // Signatures were checked by signed_dag. Rebuild in production xsort order
    // without re-signing/re-verifying every calibration loop.
    for id in ordered {
        fresh
            .insert(
                dag.get(id).expect("history block"),
                &support::FixtureVerifier,
            )
            .expect("rebuild");
    }
    (fresh, bonds.clone(), Some(candidate.clone()))
}

fn bench_validation(c: &mut Criterion) {
    let mut group = c.benchmark_group("block_validation");
    group.sample_size(20);
    group.measurement_time(Duration::from_secs(2));
    let sizes = if support::smoke() {
        vec![8, 16]
    } else {
        // Strict validation scales steeply with history depth. Routine measurements
        // stay small; thousand-block histories are explicitly opt-in.
        support::sizes(&[16, 32], &[16, 32, 100, 1_000, 4_000])
    };
    for n in sizes {
        for (name, config) in [
            (
                "validate_default_no_signature",
                ValidationConfig {
                    check_signature: false,
                    ..ValidationConfig::default()
                },
            ),
            ("validate_default_signed", ValidationConfig::default()),
            ("validate_strict_signed", ValidationConfig::strict()),
        ] {
            let fixture = OnceCell::new();
            group.bench_function(BenchmarkId::new(name, n), |b| {
                // Criterion applies its filter before invoking this callback.
                // Initialize once across warmup/sampling, outside the timed loop.
                let (dag, bonds, candidate) = fixture.get_or_init(|| {
                    eprintln!("fixture: block_validation/{name}/{n}");
                    let (dag, bonds, candidate) = support::signed_dag(n);
                    assert!(!candidate.identity.signature.is_empty());
                    let result = validate_block(&candidate, &dag, &bonds, &config);
                    assert!(result.is_valid(), "invalid fixture: {:?}", result.errors());
                    if config.check_signature {
                        let mut bad = candidate.clone();
                        bad.identity.signature = vec![0xFF];
                        assert!(
                            validate_block(&bad, &dag, &bonds, &config)
                                .errors()
                                .contains(&InvalidBlock::InvalidSignature)
                        );
                    }
                    (dag, bonds, candidate)
                });
                b.iter(|| {
                    validate_block(
                        black_box(candidate),
                        black_box(dag),
                        black_box(bonds),
                        black_box(&config),
                    )
                });
            });
        }
        let fixture = OnceCell::new();
        group.bench_function(BenchmarkId::new("validated_insert_signed", n), |b| {
            let (dag, bonds, candidate, ordered) = fixture.get_or_init(|| {
                eprintln!("fixture: block_validation/validated_insert_signed/{n}");
                let (dag, bonds, candidate) = support::signed_dag(n);
                let history = dag
                    .dom()
                    .iter()
                    .map(|id| dag.get(id).expect("signed history"))
                    .collect();
                let ordered =
                    cordial_miners_core::consensus::xsort(&history).expect("signed topology");
                let (mut checked, weights, mut value) =
                    fresh_history(&dag, &ordered, &bonds, &candidate);
                assert!(
                    validated_insert(
                        value.take().expect("signed candidate"),
                        &mut checked,
                        &weights,
                        &ValidationConfig::default()
                    )
                    .is_valid()
                );
                assert_eq!(checked.dom().len(), n + 1);
                support::assert_content_eq(
                    &checked
                        .get(&candidate.identity)
                        .expect("inserted candidate")
                        .content,
                    &candidate.content,
                );
                (dag, bonds, candidate, ordered)
            });
            b.iter_batched_ref(
                || fresh_history(dag, ordered, bonds, candidate),
                |(dag, bonds, candidate)| {
                    // Move the signed candidate without timing allocation/cloning.
                    let owned = candidate.take().expect("one insertion per fresh fixture");
                    black_box(validated_insert(
                        owned,
                        dag,
                        bonds,
                        &ValidationConfig::default(),
                    ))
                },
                BatchSize::PerIteration,
            );
        });
    }
    group.finish();
}
criterion_group!(benches, bench_validation);
criterion_main!(benches);
