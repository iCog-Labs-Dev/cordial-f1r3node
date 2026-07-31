//! Property tests for equivocation exclusion.
//!
//! Covers:
//! - `all_equivocations` only ever reports well-formed equivocations: at
//!   least two branches, all created by the stated creator, all actually
//!   present in the blocklace.
//! - `hidden_equivocations` and `acknowledges_equivocation` agree with each
//!   other for every (block, detected equivocation) pair in the DAG: a
//!   block hides an equivocation if and only if it does not acknowledge it.
//! - a block that has acknowledged (observed every branch of) a given
//!   equivocation never has any of that equivocation's branches counted in
//!   its own `approved_blocks_for_leader` set — generalizing
//!   `approved_blocks_for_leader_excludes_blocks_not_approved_due_to_equivocation`
//!   in `test_ordering.rs`.
//! - if the elected wave leader's own blocks have acknowledged a detected
//!   equivocation, `tau`'s finalized output never surfaces that
//!   equivocation's branches.
//!
//! ## Generator
//!
//! The DAG is a flat sequence of block-creation steps: each step
//! independently picks a creator and an arbitrary, possibly-empty subset of
//! *earlier* steps as predecessors (so validators may skip "rounds", blocks
//! may reference only a sparse subset of prior tips, and disconnected
//! partial histories can arise). Separately, any number of steps may be
//! designated as equivocation clones — copying an earlier step's exact
//! creator and predecessor set — which is what actually produces same-round
//! equivocations. Because several clones can target different source steps
//! (by different creators), a single generated DAG can contain zero, one,
//! or several *simultaneous* equivocations. Every assertion here derives
//! its expectations from what `all_equivocations` actually finds in the
//! built blocklace, rather than from bookkeeping done by the generator
//! itself, so the tests stay meaningful regardless of how many
//! equivocations (if any) end up present.
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
//! - Divergent-predecessor equivocations. `equivocation_clones` only ever
//!   clones an existing step's *exact* creator and predecessor set, so every
//!   generated equivocation has two (or more) branches with identical
//!   predecessors. A real equivocator can send branches with different
//!   predecessor sets — e.g. one cordial branch that references every known
//!   tip and one that hides a tip — and that shape is not generated here.
//!   "Multiple equivocations" in the bullet above means multiple
//!   identical-predecessor forks, not divergent ones.
//!
//! Failing cases print the `DagSpec` needed to reproduce, and are persisted
//! under `crates/cordial-miners-core/proptest-regressions/prop_equivocation.txt`.

use std::collections::HashSet;

use cordial_miners_core::blocklace::Blocklace;
use cordial_miners_core::consensus::{
    acknowledges_equivocation, all_equivocations, approved_blocks_for_leader, hidden_equivocations,
    tau,
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
    /// Indices into `steps` to clone as extra equivocation branches (same
    /// creator, same predecessor set as the source step). Multiple entries
    /// — including several pointing at the same or different sources — are
    /// allowed, so a single spec can encode several simultaneous
    /// equivocations by different creators.
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

/// A [`DagSpec`] strategy that additionally guarantees at least one
/// equivocation clone is present, for the tests that specifically want to
/// exercise equivocation handling rather than (validly, but vacuously)
/// generating a fork-free DAG.
fn dag_with_equivocation_strategy() -> impl Strategy<Value = DagSpec> {
    dag_spec_strategy().prop_filter("need at least one equivocation clone", |spec| {
        !spec.equivocation_clones.is_empty() && !spec.steps.is_empty()
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

proptest! {
    #![proptest_config(ProptestConfig::with_cases(96))]

    /// Whatever `all_equivocations` reports must be internally well-formed:
    /// at least two branches, every branch actually created by the stated
    /// creator, and every branch actually present in the blocklace.
    #[test]
    fn all_equivocations_are_well_formed(spec in dag_spec_strategy()) {
        let dag = build_dag(&spec);
        let equivocations = all_equivocations(&dag.blocklace);

        for equivocation in &equivocations {
            prop_assert!(equivocation.blocks.len() >= 2);
            for id in &equivocation.blocks {
                prop_assert_eq!(&id.creator, &equivocation.creator);
                prop_assert!(dag.blocklace.get(id).is_some());
            }
        }
    }

    /// For every block and every detected equivocation, `hidden_equivocations`
    /// and `acknowledges_equivocation` must agree: the block hides the
    /// equivocation exactly when it does not acknowledge it. This holds
    /// regardless of how many equivocations are present, who created them,
    /// or how sparse/partial the block's own view of the DAG is.
    #[test]
    fn hidden_and_acknowledged_are_complementary(spec in dag_spec_strategy()) {
        let dag = build_dag(&spec);
        let equivocations = all_equivocations(&dag.blocklace);
        if equivocations.is_empty() {
            return Ok(());
        }

        for block in &dag.blocks {
            let hidden = hidden_equivocations(&dag.blocklace, block);
            let hidden_pairs: HashSet<(NodeId, u64)> = hidden
                .iter()
                .map(|h| (h.creator.clone(), h.round))
                .collect();

            for equivocation in &equivocations {
                let acknowledges = acknowledges_equivocation(
                    &dag.blocklace,
                    block,
                    &equivocation.creator,
                    equivocation.round,
                );
                let key = (equivocation.creator.clone(), equivocation.round);

                prop_assert_eq!(
                    hidden_pairs.contains(&key),
                    !acknowledges,
                    "hidden_equivocations and acknowledges_equivocation disagree for {:?} at round {} w.r.t. block {:?}",
                    equivocation.creator,
                    equivocation.round,
                    block.identity
                );
            }
        }
    }

    /// A block that has acknowledged (observed every branch of) a given
    /// equivocation never counts any of that equivocation's branches among
    /// its own `approved_blocks_for_leader` set.
    #[test]
    fn acknowledged_equivocations_are_excluded_from_approval(
        spec in dag_with_equivocation_strategy(),
    ) {
        let dag = build_dag(&spec);
        let equivocations = all_equivocations(&dag.blocklace);
        prop_assume!(!equivocations.is_empty());

        for block in &dag.blocks {
            let approved = approved_blocks_for_leader(&dag.blocklace, &block.identity);

            for equivocation in &equivocations {
                let acknowledges = acknowledges_equivocation(
                    &dag.blocklace,
                    block,
                    &equivocation.creator,
                    equivocation.round,
                );

                if acknowledges {
                    for branch_id in &equivocation.blocks {
                        prop_assert!(
                            !approved.iter().any(|b| &b.identity == branch_id),
                            "{:?} acknowledged the equivocation by {:?} at round {} but still approved branch {:?}",
                            block.identity,
                            equivocation.creator,
                            equivocation.round,
                            branch_id
                        );
                    }
                }
            }
        }
    }

    /// If the elected wave leader's own blocks have acknowledged a detected
    /// equivocation, `tau`'s finalized output must never surface that
    /// equivocation's branches.
    #[test]
    fn finalized_order_excludes_equivocations_the_leader_acknowledged(
        spec in dag_with_equivocation_strategy(),
        wavelength in 1u64..=3,
        leader_pick in 0u8..MAX_VALIDATORS,
    ) {
        let mut dag = build_dag(&spec);
        let equivocations = all_equivocations(&dag.blocklace);
        prop_assume!(!equivocations.is_empty());

        let n = spec.num_validators as usize;
        let f = fault_tolerance(n);
        let leader_id = 1 + (leader_pick % spec.num_validators);

        // Rather than hoping some existing leader block happens to have
        // observed both branches of some equivocation (true for only a tiny
        // fraction of randomly generated cases), force the acknowledgment:
        // insert one explicit witness block, created by the leader, whose
        // predecessors are exactly the equivocation's branch identities.
        // `acknowledges_equivocation` is then true by construction.
        let equivocation = &equivocations[0];
        let witness_predecessors: HashSet<BlockIdentity> =
            equivocation.blocks.iter().cloned().collect();
        let tag = dag.blocks.len() as u8 + 1;
        let witness = make_block(leader_id, tag, witness_predecessors);
        insert(&mut dag.blocklace, &witness);
        dag.blocks.push(witness);

        let leader_selection = move |_wave: u64| Some(node(leader_id));

        match tau(&dag.blocklace, wavelength, n, f, leader_selection) {
            Ok(ordered) => {
                for branch_id in &equivocation.blocks {
                    prop_assert!(!ordered.contains(branch_id));
                }
            }
            Err(_) => {
                // `tau` can legitimately report that no wave has finalized
                // yet for this (spec, wavelength, leader) combination.
                // That's a distinct outcome from the invariant under test,
                // which only claims that *when* tau does produce output it
                // excludes acknowledged equivocating branches. Reject the
                // case explicitly instead of silently treating the error as
                // a pass, so a genuine regression is still visible.
                prop_assume!(false, "tau did not reach finality for this spec/wavelength/leader");
            }
        }
    }
}
