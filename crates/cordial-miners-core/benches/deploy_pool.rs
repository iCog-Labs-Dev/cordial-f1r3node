//! Deploy-pool bookkeeping only. Synthetic signature IDs are not cryptographic
//! proofs; real execution/signature cost is measured by the adapter suite.
use cordial_miners_core::bench_support as support;
use cordial_miners_core::execution::{
    Deploy, DeployPool, DeployPoolConfig, DeploySignatureAlgorithm, SignedDeploy,
};
use criterion::{BatchSize, BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use std::collections::HashSet;
use std::time::Duration;

fn deploy(index: usize) -> SignedDeploy {
    SignedDeploy {
        deploy: Deploy {
            term: vec![b'x'; 256],
            timestamp: index as u64,
            phlo_price: 1,
            phlo_limit: 100_000,
            valid_after_block_number: 1,
            shard_id: "root".into(),
            expiration_timestamp: None,
        },
        deployer: vec![1; 33],
        signature: (index as u64).to_le_bytes().to_vec(),
        signature_algorithm: DeploySignatureAlgorithm::Secp256k1,
    }
}
fn pool(values: &[SignedDeploy]) -> DeployPool {
    let mut pool = DeployPool::new(DeployPoolConfig::default());
    for value in values {
        pool.add(value.clone()).expect("unique fixture");
    }
    pool
}
fn bench_pool(c: &mut Criterion) {
    let mut group = c.benchmark_group("deploy_pool");
    group.sample_size(20);
    group.measurement_time(Duration::from_secs(2));
    for n in support::sizes(&[100, 1_000], &[100, 1_000, 10_000]) {
        let values: Vec<_> = (0..n).map(deploy).collect();
        let populated = pool(&values);
        assert_eq!(populated.len(), n);
        let in_scope: HashSet<_> = values
            .iter()
            .step_by(2)
            .map(|d| d.signature.clone())
            .collect();
        let selected = populated.select_for_block(2, 0, &in_scope);
        assert_eq!(selected.deploys.len(), 32.min(n / 2));
        assert!(
            selected
                .deploys
                .iter()
                .all(|d| !in_scope.contains(&d.signature))
        );
        let mut checked = pool(&values);
        assert_eq!(checked.prune_expired(100, 0).len(), n);
        assert!(checked.is_empty());
        group.throughput(Throughput::Elements(n as u64));
        group.bench_function(BenchmarkId::new("add_batch", n), |b| {
            b.iter_batched_ref(
                || (DeployPool::new(DeployPoolConfig::default()), values.clone()),
                |(pool, values)| {
                    for value in values.drain(..) {
                        pool.add(value).expect("add");
                    }
                },
                BatchSize::PerIteration,
            );
        });
        group.bench_function(BenchmarkId::new("select_with_ancestry_filter", n), |b| {
            b.iter(|| populated.select_for_block(2, 0, &in_scope));
        });
        group.bench_function(BenchmarkId::new("prune_expired", n), |b| {
            b.iter_batched_ref(
                || pool(&values),
                |pool| pool.prune_expired(100, 0),
                BatchSize::PerIteration,
            );
        });
    }
    group.finish();
}
criterion_group!(benches, bench_pool);
criterion_main!(benches);
