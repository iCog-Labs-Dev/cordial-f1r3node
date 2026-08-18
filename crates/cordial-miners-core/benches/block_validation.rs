//! Benchmark: block validation pipeline
//!
//! Measures performance of inbound peer block validation:
//!
//! * **`validate_block`** — closure check, chain axiom (equivocation check),
//!   content hash check under Default vs Strict (cordial condition) configs
//!   against DAGs of 100, 1 000, and 5 000 blocks.
//! * **`validated_insert`** — complete validation + DAG insertion path.

use std::collections::{HashMap, HashSet};

use cordial_miners_core::Block;
use cordial_miners_core::blocklace::Blocklace;
use cordial_miners_core::consensus::validation::{
    ValidationConfig, validate_block, validated_insert,
};
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

fn make_block(creator: &NodeId, tag: u64, predecessors: HashSet<BlockIdentity>) -> Block {
    let content = BlockContent {
        payload: tag.to_le_bytes().to_vec(),
        predecessors,
    };
    let content_hash = cordial_miners_core::crypto::hash_content(&content);
    Block {
        identity: BlockIdentity {
            content_hash,
            creator: creator.clone(),
            signature: tag.to_le_bytes().to_vec(),
        },
        content,
    }
}

fn build_dag(n: usize) -> (Blocklace, HashMap<NodeId, u64>, Block) {
    let validators: Vec<NodeId> = (1..=4u8).map(node).collect();
    let bonds: HashMap<NodeId, u64> = validators.iter().map(|v| (v.clone(), 100)).collect();
    let mut blocklace = Blocklace::new();
    let mut prev_round: Vec<Block> = Vec::new();
    let mut tag: u64 = 1;

    let rounds = n / validators.len();
    for _ in 0..rounds {
        let preds: HashSet<BlockIdentity> = prev_round.iter().map(|b| b.identity.clone()).collect();
        let mut this_round = Vec::new();
        for v in &validators {
            let block = make_block(v, tag, preds.clone());
            tag += 1;
            blocklace
                .insert(block.clone(), &MockVerifier)
                .expect("setup block insert");
            this_round.push(block);
        }
        prev_round = this_round;
    }

    // Candidate block referencing the latest round
    let preds: HashSet<BlockIdentity> = prev_round.iter().map(|b| b.identity.clone()).collect();
    let candidate = make_block(&validators[0], tag, preds);

    (blocklace, bonds, candidate)
}

// ── benchmarks ────────────────────────────────────────────────────────────────

fn bench_validate_block_default_config(c: &mut Criterion) {
    let sizes: &[usize] = &[100, 1_000, 5_000];

    let mut group = c.benchmark_group("block_validation/validate_default");
    group.sample_size(20);

    for &n in sizes {
        let (blocklace, bonds, candidate) = build_dag(n);
        let config = ValidationConfig {
            check_signature: false,
            ..Default::default()
        };

        group.bench_with_input(
            BenchmarkId::new(format!("dag_size_{n}"), ""),
            &(&candidate, &blocklace, &bonds, &config),
            |b, (cand, bl, bnd, cfg)| {
                b.iter(|| validate_block(cand, bl, bnd, cfg));
            },
        );
    }
    group.finish();
}

fn bench_validate_block_strict_config(c: &mut Criterion) {
    let sizes: &[usize] = &[50, 250, 1_000];

    let mut group = c.benchmark_group("block_validation/validate_strict");
    group.sample_size(20);

    for &n in sizes {
        let (blocklace, bonds, candidate) = build_dag(n);
        let config = ValidationConfig {
            check_signature: false,
            ..ValidationConfig::strict()
        };

        group.bench_with_input(
            BenchmarkId::new(format!("dag_size_{n}"), ""),
            &(&candidate, &blocklace, &bonds, &config),
            |b, (cand, bl, bnd, cfg)| {
                b.iter(|| validate_block(cand, bl, bnd, cfg));
            },
        );
    }
    group.finish();
}

fn bench_validated_insert(c: &mut Criterion) {
    let sizes: &[usize] = &[50, 250, 1_000];

    let mut group = c.benchmark_group("block_validation/validated_insert");
    group.sample_size(20);

    for &n in sizes {
        let config = ValidationConfig {
            check_signature: false,
            ..Default::default()
        };

        group.bench_with_input(
            BenchmarkId::new(format!("dag_size_{n}"), ""),
            &(),
            |b, _| {
                b.iter_batched(
                    || {
                        let (bl, bnd, cand) = build_dag(n);
                        (bl, cand, bnd, config.clone())
                    },
                    |(mut bl, cand, bnd, cfg)| {
                        let res = validated_insert(cand, &mut bl, &bnd, &cfg);
                        assert!(res.is_valid(), "insert must succeed");
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
    bench_validate_block_default_config,
    bench_validate_block_strict_config,
    bench_validated_insert,
);
criterion_main!(benches);
