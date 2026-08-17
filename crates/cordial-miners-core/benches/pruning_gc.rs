//! Benchmark: checkpoint pruning and garbage collection
//!
//! Measures memory-bounding operations on the in-memory blocklace:
//!
//! * **`prune_below_checkpoint`** — explicit GC pass removing finalized history
//!   while retaining the structural closure at 500, 2 000, and 5 000 blocks.
//! * **`checkpoint_after_weighted_finality`** — end-to-end leader discovery,
//!   prefix export, and checkpoint pruning pass.
//! * **Incremental GC loop** — continuous chain growth with periodic checkpointing.

use std::collections::{HashMap, HashSet};

use cordial_miners_core::Block;
use cordial_miners_core::blocklace::Blocklace;
use cordial_miners_core::consensus::pruning::{CheckpointGc, checkpoint_after_weighted_finality};
use cordial_miners_core::crypto::CryptoVerifier;
use cordial_miners_core::types::{BlockContent, BlockIdentity, NodeId};

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};

// ── helpers ───────────────────────────────────────────────────────────────────

struct MockVerifier;

impl CryptoVerifier for MockVerifier {
    type Error = String;

    fn verify_block(
        &self,
        _content: &BlockContent,
        _sig: &[u8],
        _creator: &NodeId,
    ) -> Result<(), Self::Error> {
        Ok(())
    }
}

fn node(id: u8) -> NodeId {
    NodeId(vec![id])
}

fn make_id(creator: &NodeId, tag: u64) -> BlockIdentity {
    let mut hash = [0u8; 32];
    hash[0..8].copy_from_slice(&tag.to_le_bytes());
    hash[8] = creator.0[0];

    BlockIdentity {
        content_hash: hash,
        creator: creator.clone(),
        signature: tag.to_le_bytes().to_vec(),
    }
}

fn make_block(creator: &NodeId, tag: u64, predecessors: HashSet<BlockIdentity>) -> Block {
    Block {
        identity: make_id(creator, tag),
        content: BlockContent {
            payload: tag.to_le_bytes().to_vec(),
            predecessors,
        },
    }
}

fn insert(blocklace: &mut Blocklace, block: &Block) {
    blocklace
        .insert(block.clone(), &MockVerifier)
        .expect("bench block should insert");
}

fn build_multi_wave_dag(
    num_validators: usize,
    waves: u64,
    wavelength: u64,
) -> (Blocklace, BlockIdentity) {
    let validators: Vec<NodeId> = (0..num_validators as u8).map(node).collect();
    let mut blocklace = Blocklace::new();
    let total_rounds = waves * wavelength;
    let mut prev_round: Vec<Block> = Vec::new();
    let mut tag: u64 = 1;
    let mut midpoint_checkpoint = None;

    let target_checkpoint_round = (waves / 2) * wavelength;

    for round in 0..=total_rounds {
        let preds: HashSet<BlockIdentity> = prev_round.iter().map(|b| b.identity.clone()).collect();
        let mut this_round = Vec::new();
        for v in &validators {
            let block = make_block(v, tag, preds.clone());
            tag += 1;
            insert(&mut blocklace, &block);
            this_round.push(block);
        }
        if round == target_checkpoint_round {
            midpoint_checkpoint = this_round.first().map(|b| b.identity.clone());
        }
        prev_round = this_round;
    }

    (blocklace, midpoint_checkpoint.unwrap())
}

fn uniform_bonds(num_validators: usize) -> HashMap<NodeId, u64> {
    (0..num_validators as u8)
        .map(|id| (node(id), 100))
        .collect()
}

fn leader_fn(wave: u64) -> Option<NodeId> {
    Some(node((wave % 4) as u8))
}

// ── benchmarks ────────────────────────────────────────────────────────────────

fn bench_prune_below_checkpoint(c: &mut Criterion) {
    let wave_configs: &[(usize, u64)] = &[(4, 10), (4, 30), (4, 60)]; // produces ~120, ~360, ~720 blocks
    const WAVELENGTH: u64 = 3;

    let mut group = c.benchmark_group("pruning_gc/prune_below_checkpoint");
    group.sample_size(20);

    for &(validators, waves) in wave_configs {
        let total_blocks = (validators as u64) * (waves * WAVELENGTH + 1);

        group.bench_with_input(
            BenchmarkId::new(format!("blocks_{total_blocks}"), ""),
            &(),
            |b, _| {
                b.iter_batched(
                    || build_multi_wave_dag(validators, waves, WAVELENGTH),
                    |(mut blocklace, checkpoint)| {
                        blocklace
                            .prune_below_checkpoint(&checkpoint)
                            .expect("checkpoint pruning must succeed");
                    },
                    criterion::BatchSize::SmallInput,
                );
            },
        );
    }
    group.finish();
}

fn bench_checkpoint_after_weighted_finality(c: &mut Criterion) {
    let wave_configs: &[(usize, u64)] = &[(4, 5), (4, 15), (4, 30)];
    const WAVELENGTH: u64 = 3;

    let mut group = c.benchmark_group("pruning_gc/checkpoint_after_weighted_finality");
    group.sample_size(20);

    for &(validators, waves) in wave_configs {
        let bonds = uniform_bonds(validators);
        let total_blocks = (validators as u64) * (waves * WAVELENGTH + 1);

        group.bench_with_input(
            BenchmarkId::new(format!("blocks_{total_blocks}"), ""),
            &(),
            |b, _| {
                b.iter_batched(
                    || {
                        let (blocklace, _) = build_multi_wave_dag(validators, waves, WAVELENGTH);
                        (blocklace, bonds.clone())
                    },
                    |(mut blocklace, bonds)| {
                        checkpoint_after_weighted_finality(
                            &mut blocklace,
                            WAVELENGTH,
                            &bonds,
                            leader_fn,
                        )
                        .expect("weighted finality checkpoint must not error");
                    },
                    criterion::BatchSize::SmallInput,
                );
            },
        );
    }
    group.finish();
}

criterion_group!(
    benches,
    bench_prune_below_checkpoint,
    bench_checkpoint_after_weighted_finality,
);
criterion_main!(benches);
