//! Shared helpers for property-based tests.
//!
//! Generates well-formed "round-based" blocklace DAGs: every validator in
//! the spec produces exactly one block per round, referencing *every* block
//! produced in the previous round (mirroring the fixed hand-built DAGs used
//! throughout `test_ordering.rs` / `test_finality.rs`, just parameterized).
//! Because a block's predecessors are always the previous round's blocks —
//! already inserted into the blocklace — every generated block trivially
//! satisfies the closure axiom, so `insert` can never fail. This keeps the
//! generator itself out of the way of the invariants under test.
//!
//! Optionally, a single `(validator, round)` pair can be designated as an
//! equivocation point: that validator produces two conflicting blocks at
//! that round (both referencing the same previous-round predecessors)
//! instead of one. All subsequent rounds therefore reference *both*
//! branches, exactly like `predecessor_closure_can_acknowledge_equivocation`
//! in `test_cordiality.rs`.

use std::collections::HashSet;

use cordial_miners_core::blocklace::Blocklace;
use cordial_miners_core::crypto::CryptoVerifier;
use cordial_miners_core::{Block, BlockContent, BlockIdentity, NodeId};

use proptest::prelude::*;

pub struct MockVerifier;

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

pub fn node(id: u8) -> NodeId {
    NodeId(vec![id])
}

pub fn bonds(ids: &[u8], stake: u64) -> std::collections::HashMap<NodeId, u64> {
    ids.iter().map(|id| (node(*id), stake)).collect()
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

/// Insert a block that is known (by construction) to satisfy the closure
/// axiom. Panics on failure, since that would indicate a bug in the
/// generator rather than in the code under test.
pub fn insert(blocklace: &mut Blocklace, block: &Block) {
    let verifier = MockVerifier;
    blocklace
        .insert(block.clone(), &verifier)
        .expect("generator must only ever produce closure-valid blocks");
}

/// Parameters for a generated round-based DAG. `Debug` is derived so that a
/// proptest failure prints the exact spec needed to reproduce the DAG.
#[derive(Debug, Clone)]
pub struct DagSpec {
    pub validators: Vec<u8>,
    /// Highest round index present in the DAG (rounds run `0..=max_round`).
    pub max_round: u8,
    /// Optional single equivocation point: (validator id, round).
    pub equivocation: Option<(u8, u8)>,
}

#[derive(Debug, Clone)]
pub struct EquivocationInfo {
    pub creator: NodeId,
    pub round: u64,
    pub branches: Vec<Block>,
}

pub struct GeneratedDag {
    pub blocklace: Blocklace,
    /// Blocks grouped by the round they were generated in (round 0 first).
    pub blocks_by_round: Vec<Vec<Block>>,
    pub equivocation: Option<EquivocationInfo>,
}

/// A proptest strategy over [`DagSpec`]: 2-5 validators, 0-4 extra rounds
/// beyond genesis, and an optional equivocation point. Bounded deliberately
/// small so generated cases stay fast enough for CI.
pub fn dag_spec_strategy() -> impl Strategy<Value = DagSpec> {
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
pub fn build_dag(spec: &DagSpec) -> GeneratedDag {
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
pub fn fault_tolerance(n: usize) -> usize {
    n.saturating_sub(1) / 3
}
