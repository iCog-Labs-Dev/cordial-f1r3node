//! Production LiveBlocklaceMirror bootstrap, including an actual trusted window
//! and reverse-arrival buffering. Strict disk recovery is in lmdb_persistence.
use cordial_f1r3node_adapter::live_ingress::{LiveBlocklaceMirror, MirrorDisposition};
use cordial_f1r3node_adapter::repository::{BlocklaceRepository, RSpaceBlocklaceRepository};
use cordial_miners_core::bench_support as support;
use criterion::{BatchSize, BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use std::time::Duration;

fn bench_startup(c: &mut Criterion) {
    let mut group = c.benchmark_group("startup_recovery/live_mirror");
    group.sample_size(10);
    group.measurement_time(Duration::from_secs(2));
    for n in support::sizes(&[100, 500], &[100, 1_000, 5_000]) {
        let history = support::chain(n + 1, 256);
        // Persist a retained window and its finalized cursor. A trusted manifest
        // supplies predecessor-first order; no private copy of recovery sorting.
        let directory = tempfile::tempdir().expect("window store");
        let repository =
            RSpaceBlocklaceRepository::open(directory.path(), 128 * 1024 * 1024).expect("store");
        for block in &history[1..] {
            repository.put_block(block).expect("persist window");
        }
        repository
            .put_finalized_cursor(&history[1].identity)
            .expect("checkpoint cursor");
        let restore_window = |mirror: &mut LiveBlocklaceMirror<support::FixtureVerifier>| {
            for expected in &history[1..] {
                let value = repository
                    .get_block(&expected.identity)
                    .expect("read retained history")
                    .expect("manifest entry");
                mirror
                    .ingest_with_trusted_boundary(value)
                    .expect("trusted production import");
            }
        };
        let mut restored = LiveBlocklaceMirror::new(support::FixtureVerifier);
        restore_window(&mut restored);
        assert_eq!(restored.blocklace().dom().len(), n);
        assert!(restored.blocklace().has_trusted_boundaries());
        assert!(
            !restored.blocklace().is_closed(),
            "missing history stays explicit"
        );
        assert!(
            restored.blocklace().checkpoint().is_none(),
            "partial import must not claim a restored protocol checkpoint"
        );
        let mut strict = cordial_miners_core::blocklace::Blocklace::new();
        // Production recovery is best effort: it reports the stored cursor even
        // when closure checks skip every orphan. A cursor alone is not recovery.
        let strict_cursor = repository
            .recover_into_engine(&mut strict, &support::FixtureVerifier)
            .expect("production strict recovery");
        assert_eq!(strict_cursor, Some(history[1].identity.clone()));
        assert!(
            strict.dom().is_empty(),
            "the strict insertion path must not admit an orphaned window"
        );
        assert_eq!(
            restored
                .blocklace()
                .trusted_boundary_predecessors(&history[1].identity)
                .expect("explicit boundary")
                .len(),
            1
        );
        for block in &history[1..] {
            support::assert_content_eq(
                &restored
                    .blocklace()
                    .get(&block.identity)
                    .expect("restored")
                    .content,
                &block.content,
            );
        }
        group.throughput(Throughput::Elements(n as u64));
        group.bench_function(BenchmarkId::new("trusted_window_from_store", n), |b| {
            b.iter_batched_ref(
                || LiveBlocklaceMirror::new(support::FixtureVerifier),
                restore_window,
                BatchSize::PerIteration,
            );
        });
        for mode in ["strict", "trusted_partial_window", "delayed_predecessors"] {
            let mut blocks = if mode == "trusted_partial_window" {
                history[1..].to_vec()
            } else {
                history[..n].to_vec()
            };
            if mode == "delayed_predecessors" {
                blocks.reverse();
            }
            let ingest = |mirror: &mut LiveBlocklaceMirror<support::FixtureVerifier>, block| {
                if mode == "trusted_partial_window" {
                    mirror.ingest_with_trusted_boundary(block)
                } else {
                    mirror.ingest(block)
                }
                .expect("production ingestion")
            };
            let mut checked = LiveBlocklaceMirror::new(support::FixtureVerifier);
            for block in &blocks {
                ingest(&mut checked, block.clone());
            }
            assert_eq!(checked.blocklace().dom().len(), n);
            assert!(checked.pending().is_empty());
            if mode == "trusted_partial_window" {
                assert_eq!(
                    checked
                        .blocklace()
                        .trusted_boundary_predecessors(&blocks[0].identity)
                        .expect("boundary recorded")
                        .len(),
                    1
                );
            }
            for block in &blocks {
                support::assert_content_eq(
                    &checked
                        .blocklace()
                        .get(&block.identity)
                        .expect("restored")
                        .content,
                    &block.content,
                );
            }
            if mode == "trusted_partial_window" {
                let mut strict = LiveBlocklaceMirror::new(support::FixtureVerifier);
                assert_eq!(
                    strict
                        .ingest(blocks[0].clone())
                        .expect("missing boundary")
                        .disposition,
                    MirrorDisposition::Buffered
                );
            }
            group.throughput(Throughput::Elements(n as u64));
            group.bench_function(BenchmarkId::new(mode, n), |b| {
                b.iter_batched_ref(
                    || {
                        (
                            LiveBlocklaceMirror::new(support::FixtureVerifier),
                            blocks.clone(),
                        )
                    },
                    |(mirror, blocks)| {
                        for block in blocks.drain(..) {
                            ingest(mirror, block);
                        }
                    },
                    BatchSize::PerIteration,
                );
            });
        }
    }
    group.finish();
}
// Epoch storage/replay uses retained graph history. The audited reputation
// checkpoint hash is supplied by the caller, as in the production API.
struct AcceptEpochBlocks;
impl cordial_f1r3node_adapter::grpc_ingest::BlocklaceAdapter<cordial_miners_core::BlockIdentity>
    for AcceptEpochBlocks
{
    fn on_block(&mut self, _: cordial_miners_core::Block) -> anyhow::Result<()> {
        Ok(())
    }
}

fn bench_epoch_recovery(c: &mut Criterion) {
    use cordial_f1r3node_adapter::{
        live_ingress::LiveIngress,
        por::{
            PorWeightEpochPolicy, PorWeightEpochRecord, PorWeightEpochStore,
            finalized_prefix_checkpoint,
        },
        shard_conf::CasperShardConf,
    };
    use cordial_miners_core::consensus::{depth, wave_of_round};
    use std::collections::{HashMap, HashSet};

    let mut group = c.benchmark_group("startup_recovery/persisted_epoch");
    group.sample_size(10);
    group.measurement_time(Duration::from_secs(2));
    for n in support::sizes(&[30, 90], &[30, 90, 300]) {
        let validator = support::node(0);
        let old = HashMap::from([(validator.clone(), 10)]);
        let new = HashMap::from([(validator.clone(), 20)]);
        let mut blocks = Vec::new();
        let mut predecessors = HashSet::new();
        for index in 0..n + 3 {
            let block = support::block(validator.clone(), index as u64, predecessors, 256);
            predecessors = HashSet::from([block.identity.clone()]);
            blocks.push(block);
        }
        let fresh = |count| {
            let mut ingress = LiveIngress::with_consensus_view(
                AcceptEpochBlocks,
                old.clone(),
                CasperShardConf::default(),
                "root",
            );
            for block in &blocks[..count] {
                ingress
                    .ingest_trusted_block(block.clone())
                    .expect("retained history");
            }
            ingress
                .latest_finalized_ordered_output(3)
                .expect("publish old-weight prefix");
            ingress
        };
        let mut original = fresh(n);
        let sealed = original
            .latest_finalized_ordered_output(3)
            .expect("sealed prefix");
        let source = finalized_prefix_checkpoint(&sealed, 1)
            .expect("source")
            .expect("sixteen finalized blocks");
        let source_wave = wave_of_round(
            depth(original.blocklace(), &source.terminal_block).expect("source depth"),
            3,
        )
        .expect("source wave");
        let boundary_wave = wave_of_round(
            depth(
                original.blocklace(),
                sealed.anchor.as_ref().expect("anchor"),
            )
            .expect("anchor depth"),
            3,
        )
        .expect("boundary wave");
        let committed_hash = [42; 32];
        let record = PorWeightEpochRecord::prepare(
            &original,
            &source,
            committed_hash,
            &new,
            PorWeightEpochPolicy::new(boundary_wave + 1 - source_wave).expect("future policy"),
        )
        .expect("prepared epoch");
        let directory = tempfile::tempdir().expect("epoch directory");
        let store = PorWeightEpochStore::open(directory.path()).expect("epoch store");
        store.persist(&record).expect("persist prepared epoch");
        drop(store);
        let restore = || {
            PorWeightEpochStore::open(directory.path())
                .expect("reopen store")
                .restore()
                .expect("read epoch")
                .expect("persisted record")
        };
        assert_eq!(restore(), record);
        assert!(
            record
                .verify_replay(
                    original.blocklace(),
                    std::slice::from_ref(&validator),
                    [7; 32]
                )
                .is_err(),
            "reject a different committed reputation checkpoint"
        );

        group.bench_function(BenchmarkId::new("reopen_and_restore_record", n), |b| {
            b.iter(restore);
        });
        for later_head in [false, true] {
            let count = n + if later_head { 3 } else { 0 };
            let prepare = || fresh(count);
            let apply = |ingress: &mut LiveIngress<AcceptEpochBlocks>| {
                ingress
                    .install_future_epoch(restore(), committed_hash)
                    .expect("restore and install");
            };
            let mut checked = prepare();
            let before = checked
                .latest_finalized_ordered_output(3)
                .expect("published prefix");
            apply(&mut checked);
            assert_eq!(checked.future_epoch(), Some(&record));
            assert_eq!(checked.bonds(), &new);
            let after = checked
                .latest_finalized_ordered_output(3)
                .expect("epoch output");
            assert_eq!(after.blocks, before.blocks);
            assert!(after.blocks.starts_with(&sealed.blocks));
            if later_head {
                assert!(after.blocks.len() > sealed.blocks.len());
            }
            group.bench_function(
                BenchmarkId::new(
                    if later_head {
                        "restore_replay_install_later_head"
                    } else {
                        "restore_replay_install_sealed_head"
                    },
                    n,
                ),
                |b| b.iter_batched_ref(prepare, apply, BatchSize::PerIteration),
            );
        }
    }
    group.finish();
}
criterion_group!(benches, bench_startup, bench_epoch_recovery);
criterion_main!(benches);
