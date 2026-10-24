//! Benchmark finalized-ordering poll paths as mirrored history grows.
//!
//! Run with:
//! `cargo bench --locked -p cordial-f1r3node-adapter --bench por_ordering`
//!
//! Changed-poll samples time only the poll; fixture construction, baseline
//! publication, and mirror growth happen in Criterion's untimed setup.

use std::collections::{HashMap, HashSet};
use std::time::Duration;

use cordial_f1r3node_adapter::grpc_ingest::BlocklaceAdapter;
use cordial_f1r3node_adapter::live_ingress::LiveIngress;
use cordial_f1r3node_adapter::shard_conf::CasperShardConf;
use cordial_miners_core::Block;
use cordial_miners_core::consensus::weighted_tau;
use cordial_miners_core::crypto::hash_content;
use cordial_miners_core::execution::{BlockState, CordialBlockPayload};
use cordial_miners_core::types::{BlockContent, BlockIdentity, NodeId};
use criterion::{BenchmarkId, Criterion, black_box, criterion_group, criterion_main};

const WAVELENGTH: u64 = 3;
const VALIDATOR_COUNT: u8 = 4;
const HISTORY_WAVES: &[u64] = &[1, 4, 10];

#[derive(Default)]
struct BenchAdapter;

impl BlocklaceAdapter<BlockIdentity> for BenchAdapter {
    fn on_block(&mut self, _block: Block) -> anyhow::Result<()> {
        Ok(())
    }
}

fn node(id: u8) -> NodeId {
    NodeId(vec![id])
}

fn bonds() -> HashMap<NodeId, u64> {
    (0..VALIDATOR_COUNT).map(|id| (node(id), 100)).collect()
}

fn make_block(creator: &NodeId, tag: u64, predecessors: &[BlockIdentity]) -> Block {
    let predecessors: HashSet<_> = predecessors.iter().cloned().collect();
    let payload = CordialBlockPayload {
        state: BlockState {
            pre_state_hash: vec![0; 32],
            post_state_hash: vec![1; 32],
            bonds: Vec::new(),
            block_number: tag,
        },
        deploys: Vec::new(),
        rejected_deploys: Vec::new(),
        system_deploys: Vec::new(),
    };
    let content = BlockContent {
        payload: payload.to_bytes(),
        predecessors,
    };
    Block {
        identity: BlockIdentity {
            content_hash: hash_content(&content),
            creator: creator.clone(),
            signature: tag.to_le_bytes().to_vec(),
        },
        content,
    }
}

fn append_round(
    ingress: &mut LiveIngress<BenchAdapter>,
    previous_round: &[BlockIdentity],
    tag: &mut u64,
) -> Vec<BlockIdentity> {
    let mut current_round = Vec::with_capacity(usize::from(VALIDATOR_COUNT));
    for validator in 0..VALIDATOR_COUNT {
        let block = make_block(&node(validator), *tag, previous_round);
        *tag += 1;
        current_round.push(block.identity.clone());
        ingress
            .ingest_trusted_block(block)
            .expect("benchmark block should be accepted");
    }
    current_round
}

fn build_ingress(waves: u64) -> (LiveIngress<BenchAdapter>, Vec<BlockIdentity>, u64) {
    let mut ingress = LiveIngress::with_consensus_view(
        BenchAdapter,
        bonds(),
        CasperShardConf::default(),
        "por-ordering-bench",
    );
    let mut previous_round = Vec::new();
    let mut tag = 1;
    for _ in 0..=(waves * WAVELENGTH) {
        previous_round = append_round(&mut ingress, &previous_round, &mut tag);
    }
    (ingress, previous_round, tag)
}

fn grow_mirror(
    ingress: &mut LiveIngress<BenchAdapter>,
    previous_round: Vec<BlockIdentity>,
    first_round: u64,
    tag: &mut u64,
) {
    let mut previous = previous_round;
    for _ in first_round..first_round + WAVELENGTH {
        previous = append_round(ingress, &previous, tag);
    }
}

fn independent_full_output(ingress: &LiveIngress<BenchAdapter>) -> Vec<BlockIdentity> {
    let mut validators: Vec<_> = ingress.bonds().keys().cloned().collect();
    validators.sort();
    let leader_count = validators.len();
    weighted_tau(ingress.blocklace(), WAVELENGTH, ingress.bonds(), |wave| {
        let index = usize::try_from(wave).ok()? % leader_count;
        Some(validators[index].clone())
    })
    .expect("full weighted tau should succeed")
}

fn report_diagnostics(label: &str, history_blocks: usize, ingress: &LiveIngress<BenchAdapter>) {
    let diagnostics = ingress.ordering_diagnostics();
    eprintln!(
        "{label}: history_blocks={history_blocks}, path={:?}, fallback={:?}, cache_entries={}, \
         unchanged_polls={}, verified_prefix_extensions={}, full_recomputations={}, \
         full_fallbacks={}, fallbacks_by_reason={:?}",
        diagnostics.last_path,
        diagnostics.last_fallback_reason,
        diagnostics.cache_entries,
        diagnostics.unchanged_polls,
        diagnostics.verified_prefix_extensions,
        diagnostics.full_recomputations,
        diagnostics.full_fallbacks,
        diagnostics.fallbacks_by_reason,
    );
}

fn bench_live_ingress_ordering(c: &mut Criterion) {
    let mut group = c.benchmark_group("por_ordering/live_poll");
    group.sample_size(10);
    group.measurement_time(Duration::from_secs(1));
    group.warm_up_time(Duration::from_millis(500));

    for &waves in HISTORY_WAVES {
        let (mut unchanged_ingress, _, _) = build_ingress(waves);
        let unchanged_output = unchanged_ingress
            .latest_finalized_ordered_output(WAVELENGTH)
            .expect("baseline ordering should succeed");
        let repeated = unchanged_ingress
            .latest_finalized_ordered_output(WAVELENGTH)
            .expect("unchanged poll should succeed");
        assert_eq!(repeated, unchanged_output);
        assert_eq!(
            unchanged_ingress.ordering_diagnostics().last_path,
            Some("unchanged")
        );
        report_diagnostics(
            "unchanged",
            unchanged_ingress.blocklace().dom().len(),
            &unchanged_ingress,
        );
        group.bench_with_input(
            BenchmarkId::new("unchanged", format!("waves_{waves}")),
            &waves,
            |b, &waves| {
                b.iter_batched(
                    || {
                        let (mut ingress, _, _) = build_ingress(waves);
                        ingress
                            .latest_finalized_ordered_output(WAVELENGTH)
                            .expect("baseline ordering should succeed");
                        ingress
                    },
                    |mut ingress| {
                        black_box(
                            ingress
                                .latest_finalized_ordered_output(WAVELENGTH)
                                .expect("unchanged poll should succeed"),
                        );
                    },
                    criterion::BatchSize::LargeInput,
                );
            },
        );

        let (mut incremental_check, previous_round, mut tag) = build_ingress(waves);
        let _ = incremental_check
            .latest_finalized_ordered_output(WAVELENGTH)
            .expect("baseline ordering should succeed");
        let first_new_round = waves * WAVELENGTH + 1;
        grow_mirror(
            &mut incremental_check,
            previous_round.clone(),
            first_new_round,
            &mut tag,
        );
        let extended_output = incremental_check
            .latest_finalized_ordered_output(WAVELENGTH)
            .expect("changed-mirror ordering should succeed");
        assert_eq!(
            extended_output.blocks,
            independent_full_output(&incremental_check)
        );
        assert_eq!(
            incremental_check.ordering_diagnostics().last_path,
            Some("verified_prefix"),
            "benchmark fixture must exercise verified-prefix extension"
        );
        report_diagnostics(
            "changed_verified_prefix",
            incremental_check.blocklace().dom().len(),
            &incremental_check,
        );
        group.bench_with_input(
            BenchmarkId::new("changed_verified_prefix", format!("waves_{waves}")),
            &waves,
            |b, &waves| {
                b.iter_batched(
                    || {
                        let (mut ingress, previous_round, mut tag) = build_ingress(waves);
                        ingress
                            .latest_finalized_ordered_output(WAVELENGTH)
                            .expect("baseline ordering should succeed");
                        grow_mirror(
                            &mut ingress,
                            previous_round,
                            waves * WAVELENGTH + 1,
                            &mut tag,
                        );
                        ingress
                    },
                    |mut ingress| {
                        black_box(
                            ingress
                                .latest_finalized_ordered_output(WAVELENGTH)
                                .expect("changed-mirror poll should succeed"),
                        );
                    },
                    criterion::BatchSize::LargeInput,
                );
            },
        );

        let (mut full_check, previous_round, mut tag) = build_ingress(waves);
        let _ = full_check
            .latest_finalized_ordered_output(WAVELENGTH)
            .expect("baseline ordering should succeed");
        let mut changed_bonds = bonds();
        changed_bonds.insert(node(0), 101);
        full_check.set_bonds(changed_bonds);
        full_check.set_bonds(bonds());
        grow_mirror(
            &mut full_check,
            previous_round.clone(),
            first_new_round,
            &mut tag,
        );
        let full_output = full_check
            .latest_finalized_ordered_output(WAVELENGTH)
            .expect("full fallback ordering should succeed");
        assert_eq!(full_output.blocks, independent_full_output(&full_check));
        assert_eq!(full_check.ordering_diagnostics().last_path, Some("full"));
        report_diagnostics(
            "changed_full_fallback",
            full_check.blocklace().dom().len(),
            &full_check,
        );
        group.bench_with_input(
            BenchmarkId::new("changed_full_fallback", format!("waves_{waves}")),
            &waves,
            |b, &waves| {
                b.iter_batched(
                    || {
                        let (mut ingress, previous_round, mut tag) = build_ingress(waves);
                        ingress
                            .latest_finalized_ordered_output(WAVELENGTH)
                            .expect("baseline ordering should succeed");
                        let mut changed_bonds = bonds();
                        changed_bonds.insert(node(0), 101);
                        ingress.set_bonds(changed_bonds);
                        ingress.set_bonds(bonds());
                        grow_mirror(
                            &mut ingress,
                            previous_round,
                            waves * WAVELENGTH + 1,
                            &mut tag,
                        );
                        ingress
                    },
                    |mut ingress| {
                        black_box(
                            ingress
                                .latest_finalized_ordered_output(WAVELENGTH)
                                .expect("changed-mirror poll should succeed"),
                        );
                    },
                    criterion::BatchSize::LargeInput,
                );
            },
        );
    }
    group.finish();
}

criterion_group!(benches, bench_live_ingress_ordering);
criterion_main!(benches);
