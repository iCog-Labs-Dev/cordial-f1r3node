//! Benchmark: leader lookup
//!
//! Measures [`latest_weighted_final_leader`] over synthetically constructed
//! blocklaces.  We vary:
//!
//! * **Wave count** — 1, 3, and 10 finalized waves.
//! * **Validator count** — 4, 10, and 50 validators.
//! * **Bond distribution** — uniform (every validator holds the same stake)
//!   and skewed (one "whale" validator holds 10 × the stake of all others).
//!
//! Each blocklace contains `waves × wavelength` rounds.  Within each round
//! every validator publishes one block that references all blocks from the
//! previous round, so the DAG is maximally connected and finality is
//! guaranteed after the very first wave.

use std::collections::{HashMap, HashSet};

use cordial_miners_core::Block;
use cordial_miners_core::blocklace::Blocklace;
use cordial_miners_core::consensus::latest_weighted_final_leader;
use cordial_miners_core::crypto::CryptoVerifier;
use cordial_miners_core::types::{BlockContent, BlockIdentity, NodeId};

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};

// ── minimal test verifier ─────────────────────────────────────────────────────

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

// ── block construction helpers ────────────────────────────────────────────────

fn node(id: u16) -> NodeId {
    NodeId(id.to_le_bytes().to_vec())
}

fn make_id(creator: &NodeId, tag: u64) -> BlockIdentity {
    let mut hash = [0u8; 32];
    hash[0..8].copy_from_slice(&tag.to_le_bytes());
    if creator.0.len() >= 2 {
        hash[8] = creator.0[0];
        hash[9] = creator.0[1];
    }
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

// ── DAG factory ───────────────────────────────────────────────────────────────

/// Build a fully-connected round-based blocklace.
///
/// Produces `waves × wavelength + 1` rounds (0-indexed). Every validator
/// publishes one block per round, each referencing every block from the
/// previous round. This guarantees finality from the very first wave.
fn build_blocklace(num_validators: usize, waves: u64, wavelength: u64) -> Blocklace {
    let validators: Vec<NodeId> = (0..num_validators as u16).map(node).collect();
    let mut blocklace = Blocklace::new();
    let total_rounds = waves * wavelength;
    let mut prev_round: Vec<Block> = Vec::new();
    let mut tag: u64 = 1;

    for _round in 0..=total_rounds {
        let predecessors: HashSet<BlockIdentity> =
            prev_round.iter().map(|b| b.identity.clone()).collect();

        let mut this_round = Vec::new();
        for v in &validators {
            let block = make_block(v, tag, predecessors.clone());
            tag += 1;
            insert(&mut blocklace, &block);
            this_round.push(block);
        }
        prev_round = this_round;
    }

    blocklace
}

// ── bond factories ────────────────────────────────────────────────────────────

fn uniform_bonds(num_validators: usize) -> HashMap<NodeId, u64> {
    (0..num_validators as u16)
        .map(|id| (node(id), 100))
        .collect()
}

/// Validator 0 holds 10× the stake of all others — heavily skewed.
fn skewed_bonds(num_validators: usize) -> HashMap<NodeId, u64> {
    (0..num_validators as u16)
        .map(|id| {
            let stake = if id == 0 { 1_000 } else { 100 };
            (node(id), stake)
        })
        .collect()
}

// ── leader selector ───────────────────────────────────────────────────────────

/// Round-robin leader: wave % n.
fn leader_fn(num_validators: usize) -> impl Fn(u64) -> Option<NodeId> + Copy {
    let n = num_validators as u16;
    move |wave: u64| Some(node((wave % u64::from(n)) as u16))
}

// ── benchmark body ────────────────────────────────────────────────────────────

fn bench_leader_lookup(c: &mut Criterion) {
    const WAVELENGTH: u64 = 3;
    let wave_counts: &[u64] = &[1, 3, 10];
    let validator_counts: &[usize] = &[4, 10, 50];

    // ── Uniform bonds ────────────────────────────────────────────────────────
    let mut group = c.benchmark_group("leader_lookup/uniform_bonds");
    group.sample_size(20);

    for &waves in wave_counts {
        for &validators in validator_counts {
            let blocklace = build_blocklace(validators, waves, WAVELENGTH);
            let bonds = uniform_bonds(validators);
            let lf = leader_fn(validators);

            group.bench_with_input(
                BenchmarkId::new(format!("w{waves}_v{validators}"), ""),
                &(),
                |b, _| {
                    b.iter(|| latest_weighted_final_leader(&blocklace, WAVELENGTH, &bonds, lf));
                },
            );
        }
    }
    group.finish();

    // ── Skewed bonds ─────────────────────────────────────────────────────────
    let mut group = c.benchmark_group("leader_lookup/skewed_bonds");
    group.sample_size(20);

    for &waves in wave_counts {
        for &validators in validator_counts {
            let blocklace = build_blocklace(validators, waves, WAVELENGTH);
            let bonds = skewed_bonds(validators);
            let lf = leader_fn(validators);

            group.bench_with_input(
                BenchmarkId::new(format!("w{waves}_v{validators}"), ""),
                &(),
                |b, _| {
                    b.iter(|| latest_weighted_final_leader(&blocklace, WAVELENGTH, &bonds, lf));
                },
            );
        }
    }
    group.finish();
}

criterion_group!(benches, bench_leader_lookup);
criterion_main!(benches);
