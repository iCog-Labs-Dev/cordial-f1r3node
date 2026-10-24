//! Real LMDB transactions and production boot recovery. Reads are repeated
//! identity lookups over a prepopulated DB (warm OS cache), not cold disk reads.
//! Database open/close, fixture generation and engine destruction are untimed.
use cordial_f1r3node_adapter::live_ingress::LiveIngress;
use cordial_f1r3node_adapter::repository::{BlocklaceRepository, RSpaceBlocklaceRepository};
use cordial_f1r3node_adapter::shard_conf::CasperShardConf;
use cordial_miners_core::bench_support as support;
use cordial_miners_core::blocklace::Blocklace;
use cordial_miners_core::crypto::Secp256k1Scheme;
use criterion::{BatchSize, BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use std::time::Duration;
use tempfile::tempdir;
const MAP_SIZE: usize = 512 * 1024 * 1024;

// Close heed's registry-owned environment before removing its directory.
// Criterion drops this fixture after the timed routine on every iteration.
struct TemporaryRepository {
    repository: Option<RSpaceBlocklaceRepository>,
    _directory: tempfile::TempDir,
}
impl TemporaryRepository {
    fn new() -> Self {
        let directory = tempdir().expect("temporary database directory");
        let repository = RSpaceBlocklaceRepository::open(directory.path(), MAP_SIZE)
            .expect("open temporary database");
        Self {
            repository: Some(repository),
            _directory: directory,
        }
    }
    fn repo(&self) -> &RSpaceBlocklaceRepository {
        self.repository.as_ref().expect("live repository")
    }
}
impl Drop for TemporaryRepository {
    fn drop(&mut self) {
        if let Some(repository) = self.repository.take() {
            repository.close();
        }
    }
}

fn bench_storage(c: &mut Criterion) {
    let mut group = c.benchmark_group("lmdb_persistence");
    group.sample_size(10);
    group.measurement_time(Duration::from_secs(2));
    for n in support::sizes(&[100, 500], &[100, 1_000, 5_000]) {
        let blocks = support::chain(n, if support::large() { 4_096 } else { 256 });
        let fixture = TemporaryRepository::new();
        let repo = fixture.repo();
        for block in &blocks {
            repo.put_block(block).expect("persist");
        }
        let cursor = blocks[n / 2].identity.clone();
        repo.put_finalized_cursor(&cursor).expect("cursor");
        for block in &blocks {
            support::assert_content_eq(
                &repo
                    .get_block(&block.identity)
                    .expect("read")
                    .expect("exists")
                    .content,
                &block.content,
            );
        }
        let mut checked = Blocklace::new();
        assert_eq!(
            repo.recover_into_engine(&mut checked, &support::FixtureVerifier)
                .expect("recover"),
            Some(cursor.clone())
        );
        assert_eq!(checked.dom().len(), n);
        for block in &blocks {
            support::assert_content_eq(
                &checked.get(&block.identity).expect("recovered").content,
                &block.content,
            );
        }
        let (ingress, restored) = LiveIngress::with_persistent_store(
            (),
            support::bonds(4),
            CasperShardConf::default(),
            "root",
            repo,
            &support::FixtureVerifier,
        )
        .expect("production startup");
        assert_eq!(restored, Some(cursor));
        assert_eq!(ingress.blocklace().dom().len(), n);
        group.throughput(Throughput::Elements(n as u64));
        group.bench_function(BenchmarkId::new("put_blocks", n), |b| {
            b.iter_batched_ref(
                TemporaryRepository::new,
                |fixture| {
                    let repo = fixture.repo();
                    for block in &blocks {
                        repo.put_block(block).expect("persist");
                    }
                },
                BatchSize::PerIteration,
            );
        });
        group.bench_function(BenchmarkId::new("get_blocks_warm", n), |b| {
            b.iter(|| {
                for block in &blocks {
                    std::hint::black_box(repo.get_block(&block.identity).expect("read"));
                }
            });
        });
        group.bench_function(BenchmarkId::new("recover_into_engine", n), |b| {
            b.iter_batched_ref(
                Blocklace::new,
                |engine| {
                    repo.recover_into_engine(engine, &support::FixtureVerifier)
                        .expect("recover")
                },
                BatchSize::PerIteration,
            );
        });
        group.bench_function(BenchmarkId::new("live_ingress_startup", n), |b| {
            // Returning the ingress lets Criterion drop it after measurement.
            b.iter_batched(
                || (support::bonds(4), CasperShardConf::default()),
                |(bonds, conf)| {
                    LiveIngress::with_persistent_store(
                        (),
                        bonds,
                        conf,
                        "root",
                        repo,
                        &support::FixtureVerifier,
                    )
                    .expect("startup")
                },
                BatchSize::PerIteration,
            );
        });
    }
    // Separately named so real signature cost is never confused with trusted recovery.
    for n in support::sizes(&[100], &[100, 1_000]) {
        let (dag, bonds, _) = support::signed_dag(n);
        let blocks: Vec<_> = dag
            .dom()
            .into_iter()
            .map(|id| dag.get(id).expect("signed block"))
            .collect();
        let fixture = TemporaryRepository::new();
        let repo = fixture.repo();
        for block in &blocks {
            repo.put_block(block).expect("persist signed");
        }
        let mut checked = Blocklace::new();
        repo.recover_into_engine(&mut checked, &Secp256k1Scheme)
            .expect("signed recovery");
        assert_eq!(checked.dom().len(), n);
        assert_eq!(bonds.len(), 4);
        group.throughput(Throughput::Elements(n as u64));
        group.bench_function(BenchmarkId::new("recover_signed_history", n), |b| {
            b.iter_batched_ref(
                Blocklace::new,
                |engine| {
                    repo.recover_into_engine(engine, &Secp256k1Scheme)
                        .expect("signed recover")
                },
                BatchSize::PerIteration,
            );
        });
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
            let run = |repo: &RSpaceBlocklaceRepository, dag: &mut Blocklace| {
                let mut last = None;
                let mut removed = 0;
                for wave in layers.chunks(3) {
                    for block in wave.iter().flatten() {
                        repo.put_block(block).expect("durable growth");
                        dag.insert(block.clone(), &support::FixtureVerifier)
                            .expect("insert");
                    }
                    let report =
                        cordial_miners_core::consensus::pruning::checkpoint_after_weighted_finality(
                            dag, 3, &weights, support::leader,
                        )
                        .expect("checkpoint")
                        .expect("advances");
                    repo.put_finalized_cursor(&report.checkpoint)
                        .expect("durable cursor");
                    removed += report.removed.len();
                    last = Some(report.checkpoint);
                }
                (last, removed)
            };
            let fixture = TemporaryRepository::new();
            let repo = fixture.repo();
            let mut checked = Blocklace::new();
            let (checkpoint, removed) = run(repo, &mut checked);
            let last_leader =
                &layers[(cycles - 1) * 3][if reclaimable { 0 } else { (cycles - 1) % 4 }];
            assert_eq!(checkpoint, Some(last_leader.identity.clone()));
            assert_eq!(checked.checkpoint(), checkpoint.as_ref());
            assert_eq!(repo.finalized_cursor().expect("cursor"), checkpoint);
            assert!(checked.is_closed());
            assert_eq!(checked.dom().len() + removed, inserted);
            if reclaimable {
                assert_eq!(inserted, cycles * 9);
                assert_eq!(
                    checked.dom().len(),
                    9,
                    "only the latest wave remains in memory"
                );
                assert_eq!(removed, (cycles - 1) * 9);
                assert!(checked.get(&layers[0][0].identity).is_none());
            } else {
                assert_eq!(inserted, cycles * 12);
                assert_eq!(removed, 0, "peer references keep earlier history alive");
                assert_eq!(checked.dom().len(), inserted);
            }
            for block in layers.iter().flatten() {
                support::assert_content_eq(
                    &repo
                        .get_block(&block.identity)
                        .expect("durable history")
                        .expect("disk history is retained")
                        .content,
                    &block.content,
                );
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
            eprintln!(
                "durable fixture: shape={shape} cycles={cycles} persisted_blocks={inserted} removed_from_memory={removed} retained_in_memory={}",
                checked.dom().len(),
            );
            group.throughput(Throughput::Elements(inserted as u64));
            group.bench_function(
                BenchmarkId::new(
                    format!("durable_growth_and_periodic_checkpoint_{shape}"),
                    cycles,
                ),
                |b| {
                    b.iter_batched_ref(
                        || (TemporaryRepository::new(), Blocklace::new()),
                        |(fixture, dag)| run(fixture.repo(), dag),
                        BatchSize::PerIteration,
                    );
                },
            );
        }
    }
    group.finish();
}
criterion_group!(benches, bench_storage);
criterion_main!(benches);
