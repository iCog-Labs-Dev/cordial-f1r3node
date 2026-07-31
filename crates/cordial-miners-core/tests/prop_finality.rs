//! Property tests for weighted ratification threshold behavior.
//!
//! Covers:
//! - `is_weighted_supermajority` matches the strict-two-thirds definition
//!   directly (support_weight * 3 > total_weight * 2) for arbitrary bond
//!   assignments — this part is pure combinatorics and doesn't involve a
//!   DAG generator at all.
//! - `is_weighted_supermajority` is monotonic: growing the support set can
//!   never turn a passing check into a failing one.
//! - Weighted finality implies unweighted finality on arbitrary generated
//!   DAGs when every validator is equally staked. The reverse does not hold
//!   in general — see the note on `weighted_finality_implies_unweighted_finality_under_equal_stake`
//!   below — so this is deliberately an implication, not the equivalence
//!   the fixed example `weighted_and_unweighted_agree_when_high_stake_ratifiers`
//!   in `test_finality.rs` might suggest.
//!
//! ## Generator
//!
//! The DAG-based test uses a flat sequence of block-creation steps rather
//! than a rigid round structure: each step independently picks a creator
//! and an arbitrary, possibly-empty subset of *earlier* steps as
//! predecessors. This covers validators missing "rounds" entirely, sparse
//! predecessor sets, delayed/skipped ancestor references, irregular round
//! structure, and disconnected-but-valid partial histories. The chosen
//! "leader" validator may end up producing zero blocks at all in a given
//! generated case — that's an intentional edge case, handled by discarding
//! (`prop_assume!`) rather than asserting anything about a leader that
//! never existed.
//!
//! Not covered:
//! - Pruning/checkpointed histories. `Blocklace::checkpoint()`,
//!   `checkpoint_order_prefix()`, and `checkpoint_weighted_order_prefix()`
//!   are never populated by this generator, even though
//!   `checkpoint_predecessor`'s short-circuit is load-bearing in `tau`'s
//!   recursion (see `ordering.rs`). This is a real, currently-untested code
//!   path, not just a lower-priority corner — it needs its own tracking
//!   issue (not filed yet) rather than staying an implicit gap referenced
//!   only in a comment.
//! - Divergent-predecessor equivocations. `equivocation_clones` (used by the
//!   shared generator shape, even though this file's assertions don't key
//!   off equivocations specifically) only ever clones an existing step's
//!   *exact* creator and predecessor set. A real equivocator can send
//!   branches with different predecessor sets — e.g. one cordial branch
//!   that references every known tip and one that hides a tip — and that
//!   shape is not generated here.
//!
//! Failing cases print the exact bonds map / validator subset / `DagSpec`
//! needed to reproduce, and are persisted under
//! `crates/cordial-miners-core/proptest-regressions/prop_finality.txt`.

use std::collections::{HashMap, HashSet};

use cordial_miners_core::blocklace::Blocklace;
use cordial_miners_core::consensus::{
    is_final_leader, is_weighted_final_leader, is_weighted_supermajority,
};
use cordial_miners_core::crypto::CryptoVerifier;
use cordial_miners_core::{Block, BlockContent, BlockIdentity, NodeId};

use proptest::prelude::*;

const MAX_VALIDATORS: u8 = 5;

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

fn bonds(ids: &[u8], stake: u64) -> HashMap<NodeId, u64> {
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

fn insert(blocklace: &mut Blocklace, block: &Block) {
    let verifier = MockVerifier;
    blocklace
        .insert(block.clone(), &verifier)
        .expect("generator must only ever produce closure-valid blocks");
}

/// One step in the generated creation sequence: a creator plus a raw,
/// unsanitized set of "ideas" for predecessor indices.
#[derive(Debug, Clone)]
struct RawStep {
    creator_raw: u8,
    predecessor_picks: Vec<usize>,
}

#[derive(Debug, Clone)]
struct DagSpec {
    num_validators: u8,
    steps: Vec<RawStep>,
    equivocation_clones: Vec<usize>,
}

struct GeneratedDag {
    blocklace: Blocklace,
    /// All blocks in the exact order they were created/inserted (base
    /// steps first, then equivocation clones).
    blocks: Vec<Block>,
}

/// A proptest strategy over [`DagSpec`]. Deliberately bounded (2-5
/// validators, 1-30 steps, 0-6 equivocation clones) so generated cases stay
/// fast enough for CI, while the *shape* of the DAG itself is unconstrained.
fn dag_spec_strategy() -> impl Strategy<Value = DagSpec> {
    let step_strategy = (any::<u8>(), prop::collection::vec(0usize..40, 0..=4)).prop_map(
        |(creator_raw, predecessor_picks)| RawStep {
            creator_raw,
            predecessor_picks,
        },
    );

    (
        2u8..=MAX_VALIDATORS,
        prop::collection::vec(step_strategy, 1..=30),
    )
        .prop_flat_map(|(num_validators, steps)| {
            let step_count = steps.len();
            (
                Just(num_validators),
                Just(steps),
                prop::collection::vec(0usize..step_count, 0..=6),
            )
        })
        .prop_map(|(num_validators, steps, equivocation_clones)| DagSpec {
            num_validators,
            steps,
            equivocation_clones,
        })
}

/// Materialize a [`DagSpec`] into an actual [`Blocklace`].
fn build_dag(spec: &DagSpec) -> GeneratedDag {
    let mut blocklace = Blocklace::new();
    let mut identities: Vec<BlockIdentity> = Vec::with_capacity(spec.steps.len());
    let mut blocks: Vec<Block> =
        Vec::with_capacity(spec.steps.len() + spec.equivocation_clones.len());
    let mut tag: u8 = 1;

    for (i, step) in spec.steps.iter().enumerate() {
        let creator = 1 + (step.creator_raw % spec.num_validators);
        let predecessors: HashSet<BlockIdentity> = step
            .predecessor_picks
            .iter()
            .copied()
            .filter(|&idx| idx < i)
            .map(|idx| identities[idx].clone())
            .collect();

        let block = make_block(creator, tag, predecessors);
        tag += 1;
        insert(&mut blocklace, &block);
        identities.push(block.identity.clone());
        blocks.push(block);
    }

    for &raw_source in &spec.equivocation_clones {
        if spec.steps.is_empty() {
            break;
        }
        let source_index = raw_source % spec.steps.len();
        let source_block = &blocks[source_index];
        let creator = source_block.identity.creator.0[0];
        let predecessors = source_block.content.predecessors.clone();

        let clone_block = make_block(creator, tag, predecessors);
        tag += 1;
        insert(&mut blocklace, &clone_block);
        blocks.push(clone_block);
    }

    GeneratedDag { blocklace, blocks }
}

/// Standard BFT fault tolerance for `n` validators: the largest `f` with
/// `n >= 3f + 1`.
fn fault_tolerance(n: usize) -> usize {
    n.saturating_sub(1) / 3
}

/// Manual reference implementation of the strict two-thirds threshold, kept
/// independent of the implementation under test.
fn manual_strict_two_thirds(support: &HashSet<NodeId>, bonds_map: &HashMap<NodeId, u64>) -> bool {
    let total: u128 = bonds_map.values().map(|w| u128::from(*w)).sum();
    if total == 0 {
        return false;
    }
    let support_weight: u128 = support
        .iter()
        .filter_map(|creator| bonds_map.get(creator))
        .map(|w| u128::from(*w))
        .sum();

    support_weight * 3 > total * 2
}

fn bonds_strategy() -> impl Strategy<Value = HashMap<NodeId, u64>> {
    prop::collection::vec((1u8..=8, 0u64..=1000), 1..=8).prop_map(|entries| {
        entries
            .into_iter()
            .map(|(id, weight)| (node(id), weight))
            .collect()
    })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(128))]

    /// `is_weighted_supermajority` must agree with the direct strict
    /// two-thirds definition for any bonds map and any subset of it chosen
    /// as the "support" set.
    #[test]
    fn weighted_supermajority_matches_manual_threshold(
        bonds_map in bonds_strategy(),
        support_mask in prop::collection::vec(any::<bool>(), 1..=8),
    ) {
        let creators: Vec<NodeId> = bonds_map.keys().cloned().collect();
        let support: HashSet<NodeId> = creators
            .iter()
            .zip(support_mask.iter().cycle())
            .filter(|(_, include)| **include)
            .map(|(creator, _)| creator.clone())
            .collect();

        let expected = manual_strict_two_thirds(&support, &bonds_map);
        let actual = is_weighted_supermajority(&support, &bonds_map);
        prop_assert_eq!(actual, expected);
    }

    /// Growing the support set can never turn passing supermajority into
    /// failing supermajority: weighted stake support is monotonic.
    #[test]
    fn weighted_supermajority_is_monotonic_in_support(
        bonds_map in bonds_strategy(),
        support_mask in prop::collection::vec(any::<bool>(), 1..=8),
        extra_mask in prop::collection::vec(any::<bool>(), 1..=8),
    ) {
        let creators: Vec<NodeId> = bonds_map.keys().cloned().collect();
        let small: HashSet<NodeId> = creators
            .iter()
            .zip(support_mask.iter().cycle())
            .filter(|(_, include)| **include)
            .map(|(creator, _)| creator.clone())
            .collect();

        // `large` is guaranteed to be a superset of `small` by construction.
        let mut large = small.clone();
        for (creator, include_extra) in creators.iter().zip(extra_mask.iter().cycle()) {
            if *include_extra {
                large.insert(creator.clone());
            }
        }

        if is_weighted_supermajority(&small, &bonds_map) {
            prop_assert!(
                is_weighted_supermajority(&large, &bonds_map),
                "growing support from {:?} to {:?} lost supermajority",
                small,
                large
            );
        }
    }

    /// On an arbitrary generated DAG where every validator is equally
    /// staked, weighted finality implies unweighted finality. The reverse
    /// does *not* hold in general: `is_supermajority`'s BFT quorum threshold
    /// `k > (n+f)/2` is always slightly weaker than the strict two-thirds
    /// stake threshold (the gap is `(n - 3f)/6 >= 1/6`), so at small `n` —
    /// e.g. n=3, f=0, where unweighted only needs 2 of 3 validators but
    /// weighted needs all 3 — a candidate can be unweighted-final without
    /// being weighted-final. This asymmetry is a genuine property of the
    /// two threshold formulas, not a generator artifact, so the assertion
    /// below is deliberately one-directional.
    #[test]
    fn weighted_finality_implies_unweighted_finality_under_equal_stake(
        spec in dag_spec_strategy(),
        wavelength in 1u64..=3,
        leader_pick in 0u8..MAX_VALIDATORS,
        stake in 1u64..=1000,
    ) {
        let dag = build_dag(&spec);
        let n = spec.num_validators as usize;
        let f = fault_tolerance(n);
        let leader_id = 1 + (leader_pick % spec.num_validators);
        let leader_selection = move |_wave: u64| Some(node(leader_id));

        // The chosen leader validator may not have produced any block at
        // all in this generated DAG — that's a legitimate, arbitrary-shape
        // edge case, not a bug, so discard rather than asserting on a
        // block that doesn't exist.
        prop_assume!(dag.blocks.iter().any(|b| b.identity.creator == node(leader_id)));
        let leader_identity = dag
            .blocks
            .iter()
            .find(|b| b.identity.creator == node(leader_id))
            .expect("checked above")
            .identity
            .clone();

        let equal_bonds = bonds(&(1..=spec.num_validators).collect::<Vec<_>>(), stake);

        let unweighted =
            is_final_leader(&dag.blocklace, &leader_identity, wavelength, n, f, leader_selection);
        let weighted = is_weighted_final_leader(
            &dag.blocklace,
            &leader_identity,
            wavelength,
            &equal_bonds,
            leader_selection,
        );

        if weighted {
            prop_assert!(
                unweighted,
                "weighted-final but not unweighted-final for n={}, f={}",
                n, f
            );
        }
    }
}
