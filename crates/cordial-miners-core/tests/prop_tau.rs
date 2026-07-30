//! Property tests for deterministic ordering.
//!
//! Covers:
//! - `xsort` determinism and topological validity over generated block sets.
//! - closure/ancestor consistency between `Blocklace::observe` and
//!   `Blocklace::precedes`.
//! - `tau` prefix preservation: computing `tau` again after the blocklace has
//!   grown must reproduce the earlier output as a strict prefix.
//!
//! Generates well-formed "round-based" blocklace DAGs: every validator
//! produces exactly one block per round, referencing *every* block produced
//! in the previous round (mirroring the fixed hand-built DAGs used in
//! `test_ordering.rs` / `test_finality.rs`, just parameterized). Because a
//! block's predecessors are always the previous round's blocks — already
//! inserted into the blocklace — every generated block trivially satisfies
//! the closure axiom, so `insert` can never fail here.
//!
//! On failure, proptest prints the shrunk `DagSpec` (and, for the tau test,
//! the wavelength) that reproduces the failure, and persists it to
//! `crates/cordial-miners-core/proptest-regressions/prop_tau.txt` so CI
//! reruns replay the same case automatically.

use std::collections::{HashMap, HashSet};

use cordial_miners_core::blocklace::Blocklace;
use cordial_miners_core::consensus::{tau, xsort, OrderingError};
use cordial_miners_core::crypto::CryptoVerifier;
use cordial_miners_core::{Block, BlockContent, BlockIdentity, NodeId};

use proptest::prelude::*;

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

fn make_block(creator_id: u8, tag: u8, predecessors: HashSet<BlockIdentity>) -> Block {
    let mut content_hash = [0u8; 32];
    content_hash[0] = creator_id;
    content_hash[1] = tag;

    Block {
        identity: BlockIdentity {
            content_hash,
            creator: node(creator_id),
            signature: vec![],
        },
        content: BlockContent {
            payload: vec![tag],
            predecessors,
        },
    }
}

fn insert(blocklace: &mut Blocklace, block: &Block) {
    let verifier = MockVerifier;
    blocklace
        .insert(block.clone(), &verifier)
        .expect("generator must only ever produce closure-valid blocks");
}

/// Parameters for a generated round-based DAG. `Debug` is derived so that a
/// proptest failure prints the exact spec needed to reproduce the DAG.
#[derive(Debug, Clone)]
struct DagSpec {
    validators: Vec<u8>,
    /// Highest round index present in the DAG (rounds run `0..=max_round`).
    max_round: u8,
    /// Optional single equivocation point: (validator id, round).
    equivocation: Option<(u8, u8)>,
}

struct GeneratedDag {
    blocklace: Blocklace,
    /// Blocks grouped by the round they were generated in (round 0 first).
    blocks_by_round: Vec<Vec<Block>>,
}

/// A proptest strategy over [`DagSpec`]: 2-5 validators, 0-4 extra rounds
/// beyond genesis, and an optional equivocation point. Bounded deliberately
/// small so generated cases stay fast enough for CI.
fn dag_spec_strategy() -> impl Strategy<Value = DagSpec> {
    (2usize..=5, 0u8..=4).prop_flat_map(|(num_validators, max_round)| {
        let validators: Vec<u8> = (1..=num_validators as u8).collect();
        let validators_for_equiv = validators.clone();

        (
            Just(validators),
            Just(max_round),
            prop::option::of((prop::sample::select(validators_for_equiv), 0u8..=max_round)),
        )
            .prop_map(|(validators, max_round, equivocation)| DagSpec {
                validators,
                max_round,
                equivocation,
            })
    })
}

/// Materialize a [`DagSpec`] into an actual [`Blocklace`].
fn build_dag(spec: &DagSpec) -> GeneratedDag {
    let mut blocklace = Blocklace::new();
    let mut blocks_by_round: Vec<Vec<Block>> = Vec::new();
    let mut tag: u8 = 1;
    let mut previous_round_ids: HashSet<BlockIdentity> = HashSet::new();

    for round in 0..=spec.max_round {
        let mut this_round: Vec<Block> = Vec::new();

        for &validator in &spec.validators {
            let is_equivocator = spec.equivocation == Some((validator, round));
            let branch_count = if is_equivocator { 2 } else { 1 };

            for _ in 0..branch_count {
                let block = make_block(validator, tag, previous_round_ids.clone());
                tag += 1;
                insert(&mut blocklace, &block);
                this_round.push(block);
            }
        }

        previous_round_ids = this_round.iter().map(|b| b.identity.clone()).collect();
        blocks_by_round.push(this_round);
    }

    GeneratedDag {
        blocklace,
        blocks_by_round,
    }
}

/// Standard BFT fault tolerance for `n` validators: the largest `f` with
/// `n >= 3f + 1`.
fn fault_tolerance(n: usize) -> usize {
    n.saturating_sub(1) / 3
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    /// `xsort` must be deterministic (independent of the `HashSet`'s
    /// internal iteration order) and must produce a genuine topological
    /// order: every predecessor that is itself part of the sorted set must
    /// appear strictly before its dependent.
    #[test]
    fn xsort_is_deterministic_and_topological(spec in dag_spec_strategy()) {
        let dag = build_dag(&spec);
        let all_blocks: HashSet<Block> = dag.blocks_by_round.iter().flatten().cloned().collect();

        // Rebuild an equivalent-but-differently-ordered HashSet to make sure
        // xsort's output doesn't depend on hash iteration order.
        let reshuffled: HashSet<Block> = all_blocks.iter().cloned().collect();

        let first: Result<Vec<BlockIdentity>, OrderingError> = xsort(&all_blocks);
        let second: Result<Vec<BlockIdentity>, OrderingError> = xsort(&reshuffled);
        prop_assert_eq!(&first, &second);

        if let Ok(ordered) = &first {
            prop_assert_eq!(ordered.len(), all_blocks.len());

            let position: HashMap<BlockIdentity, usize> = ordered
                .iter()
                .enumerate()
                .map(|(i, id)| (id.clone(), i))
                .collect();

            for block in &all_blocks {
                for pred in &block.content.predecessors {
                    if let (Some(&pred_pos), Some(&block_pos)) =
                        (position.get(pred), position.get(&block.identity))
                    {
                        prop_assert!(
                            pred_pos < block_pos,
                            "predecessor {:?} must sort before {:?}",
                            pred,
                            block.identity
                        );
                    }
                }
            }
        }
    }

    /// Every block's predecessor closure (`observe`) consists of itself plus
    /// only blocks that genuinely precede it, and the blocklace stays closed
    /// (no dangling predecessors) throughout construction.
    #[test]
    fn observe_closure_is_consistent_with_precedes(spec in dag_spec_strategy()) {
        let dag = build_dag(&spec);
        prop_assert!(dag.blocklace.is_closed());

        for round_blocks in &dag.blocks_by_round {
            for block in round_blocks {
                let closure = dag.blocklace.observe(&block.identity);
                prop_assert!(closure.contains(&block.identity));

                for ancestor in &closure {
                    if ancestor != &block.identity {
                        prop_assert!(
                            dag.blocklace.precedes(ancestor, &block.identity),
                            "{:?} is in the closure of {:?} but does not precede it",
                            ancestor,
                            block.identity
                        );
                    }
                }
            }
        }
    }

    /// `tau`'s output only ever grows: recomputing it after inserting the
    /// next round of blocks must reproduce the previous output as a strict
    /// prefix. This is the safety property that lets nodes stream finalized
    /// order incrementally instead of recomputing from scratch.
    #[test]
    fn tau_output_is_prefix_stable_as_dag_grows(
        spec in dag_spec_strategy(),
        wavelength in 1u64..=3,
    ) {
        let n = spec.validators.len();
        let f = fault_tolerance(n);
        let leader_id = spec.validators[0];
        let leader_selection = move |_wave: u64| Some(node(leader_id));

        let dag = build_dag(&spec);
        let mut blocklace = Blocklace::new();
        let mut previous_output: Vec<BlockIdentity> = Vec::new();

        for round_blocks in &dag.blocks_by_round {
            for block in round_blocks {
                insert(&mut blocklace, block);
            }

            if let Ok(current) = tau(&blocklace, wavelength, n, f, leader_selection) {
                prop_assert!(
                    current.starts_with(&previous_output),
                    "tau output shrank or reordered a previously finalized prefix"
                );
                previous_output = current;
            }
        }
    }
}