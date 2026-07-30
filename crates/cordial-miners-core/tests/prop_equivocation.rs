//! Property tests for equivocation exclusion.
//!
//! Covers:
//! - `all_equivocations` reports exactly the injected equivocation (creator,
//!   round, and both branch identities).
//! - a block whose predecessor closure includes every branch of a known
//!   equivocation has no `hidden_equivocations`, and `acknowledges_equivocation`
//!   returns true for it.
//! - neither equivocating branch is ever included in `approved_blocks_for_leader`
//!   for a later leader that observed both branches, and (transitively) neither
//!   branch ever appears in `tau`'s finalized output — generalizing
//!   `approved_blocks_for_leader_excludes_blocks_not_approved_due_to_equivocation`
//!   in `test_ordering.rs` across randomly generated DAG shapes and
//!   equivocation points.
//!
//! Generates well-formed "round-based" blocklace DAGs: every validator
//! produces exactly one block per round, referencing *every* block produced
//! in the previous round — except at the injected equivocation point, where
//! the chosen validator produces two conflicting branch blocks instead of
//! one. Because a block's predecessors are always the previous round's
//! blocks — already inserted into the blocklace — every generated block
//! trivially satisfies the closure axiom, so `insert` can never fail here.
//!
//! Failing cases print the `DagSpec` (including the exact `(validator,
//! round)` equivocation point) needed to reproduce, and are persisted under
//! `crates/cordial-miners-core/proptest-regressions/prop_equivocation.txt`.

use std::collections::HashSet;

use cordial_miners_core::blocklace::Blocklace;
use cordial_miners_core::consensus::{
    acknowledges_equivocation, all_equivocations, approved_blocks_for_leader, hidden_equivocations,
    tau,
};
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

#[derive(Debug, Clone)]
struct EquivocationInfo {
    creator: NodeId,
    round: u64,
    branches: Vec<Block>,
}

struct GeneratedDag {
    blocklace: Blocklace,
    /// Blocks grouped by the round they were generated in (round 0 first).
    blocks_by_round: Vec<Vec<Block>>,
    equivocation: Option<EquivocationInfo>,
}

/// A proptest strategy over [`DagSpec`]: 2-5 validators, 0-4 extra rounds
/// beyond genesis, and an optional equivocation point.
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

/// A [`DagSpec`] strategy that always injects exactly one equivocation, and
/// leaves at least one full round after it so later blocks exist to
/// (cordially) observe both branches.
fn dag_with_equivocation_strategy() -> impl Strategy<Value = DagSpec> {
    dag_spec_strategy().prop_filter_map(
        "need an injected equivocation with at least one round after it",
        |spec| {
            let (creator, round) = spec.equivocation?;
            if round >= spec.max_round {
                return None;
            }
            Some(spec)
        },
    )
}

/// Materialize a [`DagSpec`] into an actual [`Blocklace`].
fn build_dag(spec: &DagSpec) -> GeneratedDag {
    let mut blocklace = Blocklace::new();
    let mut blocks_by_round: Vec<Vec<Block>> = Vec::new();
    let mut equivocation_info = None;
    let mut tag: u8 = 1;
    let mut previous_round_ids: HashSet<BlockIdentity> = HashSet::new();

    for round in 0..=spec.max_round {
        let mut this_round: Vec<Block> = Vec::new();

        for &validator in &spec.validators {
            let is_equivocator = spec.equivocation == Some((validator, round));
            let branch_count = if is_equivocator { 2 } else { 1 };
            let mut branches = Vec::new();

            for _ in 0..branch_count {
                let block = make_block(validator, tag, previous_round_ids.clone());
                tag += 1;
                insert(&mut blocklace, &block);
                branches.push(block.clone());
                this_round.push(block);
            }

            if is_equivocator {
                equivocation_info = Some(EquivocationInfo {
                    creator: node(validator),
                    round: u64::from(round),
                    branches,
                });
            }
        }

        previous_round_ids = this_round.iter().map(|b| b.identity.clone()).collect();
        blocks_by_round.push(this_round);
    }

    GeneratedDag {
        blocklace,
        blocks_by_round,
        equivocation: equivocation_info,
    }
}

/// Standard BFT fault tolerance for `n` validators: the largest `f` with
/// `n >= 3f + 1`.
fn fault_tolerance(n: usize) -> usize {
    n.saturating_sub(1) / 3
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    /// `all_equivocations` must report exactly the injected equivocation:
    /// the right creator, the right round, and both branch identities.
    #[test]
    fn all_equivocations_reports_the_injected_equivocation(spec in dag_with_equivocation_strategy()) {
        let dag = build_dag(&spec);
        let equivocation = dag.equivocation.as_ref().expect("filter guarantees this");

        let found = all_equivocations(&dag.blocklace);
        let matching: Vec<_> = found
            .iter()
            .filter(|e| e.creator == equivocation.creator && e.round == equivocation.round)
            .collect();

        prop_assert_eq!(matching.len(), 1);
        let reported = matching[0];
        for branch in &equivocation.branches {
            prop_assert!(reported.blocks.contains(&branch.identity));
        }
    }

    /// Every later-round block references the entire previous round —
    /// including both equivocation branches — so it must not hide the
    /// equivocation.
    #[test]
    fn later_blocks_acknowledge_the_equivocation(spec in dag_with_equivocation_strategy()) {
        let dag = build_dag(&spec);
        let equivocation = dag.equivocation.as_ref().expect("filter guarantees this");
        let equivocation_round_index = equivocation.round as usize;

        for round_blocks in dag.blocks_by_round.iter().skip(equivocation_round_index + 1) {
            for block in round_blocks {
                prop_assert!(hidden_equivocations(&dag.blocklace, block).is_empty());
                prop_assert!(acknowledges_equivocation(
                    &dag.blocklace,
                    block,
                    &equivocation.creator,
                    equivocation.round,
                ));
            }
        }
    }

    /// Neither equivocating branch is ever counted as approved by a later
    /// leader that has observed both branches, and neither branch identity
    /// ever appears in `tau`'s finalized output.
    #[test]
    fn equivocating_branches_are_excluded_from_finalized_order(
        spec in dag_with_equivocation_strategy(),
        wavelength in 1u64..=3,
    ) {
        let dag = build_dag(&spec);
        let equivocation = dag.equivocation.as_ref().expect("filter guarantees this");
        let branch_ids: Vec<BlockIdentity> =
            equivocation.branches.iter().map(|b| b.identity.clone()).collect();

        // Any block from a round after the equivocation observes both
        // branches, so neither branch should ever count as "approved" by it.
        let equivocation_round_index = equivocation.round as usize;
        for round_blocks in dag.blocks_by_round.iter().skip(equivocation_round_index + 1) {
            for leader_block in round_blocks {
                let approved = approved_blocks_for_leader(&dag.blocklace, &leader_block.identity);
                for branch_id in &branch_ids {
                    prop_assert!(!approved.iter().any(|b| &b.identity == branch_id));
                }
            }
        }

        // Finalized order (if any is reached) must likewise never surface an
        // equivocating branch.
        let n = spec.validators.len();
        let f = fault_tolerance(n);
        let leader_id = spec.validators[0];
        let leader_selection = move |_wave: u64| Some(node(leader_id));

        let mut blocklace = Blocklace::new();
        for round_blocks in &dag.blocks_by_round {
            for block in round_blocks {
                insert(&mut blocklace, block);
            }
        }

        if let Ok(ordered) = tau(&blocklace, wavelength, n, f, leader_selection) {
            for branch_id in &branch_ids {
                prop_assert!(!ordered.contains(branch_id));
            }
        }
    }
}