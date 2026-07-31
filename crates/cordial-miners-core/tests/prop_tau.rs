//! Property tests for deterministic ordering.
//!
//! Covers:
//! - `xsort` determinism and topological validity over generated block sets.
//! - closure/ancestor consistency between `Blocklace::observe` and
//!   `Blocklace::precedes`.
//! - `tau` prefix preservation: computing `tau` again after each additional
//!   block is inserted must reproduce the earlier output as a strict prefix.
//!
//! ## Generator
//!
//! Rather than a rigid "one block per validator per round, referencing the
//! entire previous round" structure, the DAG here is generated as a flat
//! sequence of block-creation steps. Each step independently picks:
//! - a creator (no forced round-robin — a validator may create many blocks
//!   in a row or none at all);
//! - an arbitrary, possibly-empty subset of *earlier* steps as predecessors
//!   (sanitized to only reference already-created blocks, which guarantees
//!   the closure axiom without constraining the shape).
//!
//! Because predecessor indices are unconstrained beyond "strictly earlier",
//! this covers: validators missing "rounds" entirely, sparse predecessor
//! sets (referencing only a few, not all, prior tips), delayed/skipped
//! ancestor references (a block can reach far back instead of only to the
//! immediately preceding step), irregular/non-uniform round structure
//! (`depth` is purely structural here, not tied to generation order), and
//! disconnected-but-valid partial histories (an empty predecessor set at
//! any point starts a brand new, independent component).
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
//! - Divergent-predecessor equivocations. `equivocation_clones` (part of
//!   the shared generator shape, even though this file's assertions don't
//!   key off equivocations specifically) only ever clones an existing
//!   step's *exact* creator and predecessor set. A real equivocator can
//!   send branches with different predecessor sets — e.g. one cordial
//!   branch that references every known tip and one that hides a tip — and
//!   that shape is not generated here.
//!
//! On failure, proptest prints the shrunk `DagSpec` (and, for the tau test,
//! the wavelength/leader) that reproduces the failure, and persists it to
//! `crates/cordial-miners-core/proptest-regressions/prop_tau.txt` so CI
//! reruns replay the same case automatically.

use std::collections::{HashMap, HashSet};

use cordial_miners_core::blocklace::Blocklace;
use cordial_miners_core::consensus::{OrderingError, tau, xsort};
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
/// unsanitized set of "ideas" for predecessor indices. `Debug` is derived so
/// proptest failures print the exact spec needed to reproduce.
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
    /// — including several pointing at the same source — are allowed.
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

/// Materialize a [`DagSpec`] into an actual [`Blocklace`]. Each step's
/// predecessor picks are sanitized down to indices that are strictly
/// earlier than the step itself (deduped via the `HashSet`), which is what
/// guarantees every generated block satisfies the closure axiom.
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
    #![proptest_config(ProptestConfig::with_cases(64))]

    /// `xsort` must be deterministic (independent of the `HashSet`'s
    /// internal iteration order) and must produce a genuine topological
    /// order: every predecessor that is itself part of the sorted set must
    /// appear strictly before its dependent.
    #[test]
    fn xsort_is_deterministic_and_topological(spec in dag_spec_strategy()) {
        let dag = build_dag(&spec);
        let all_blocks: HashSet<Block> = dag.blocks.iter().cloned().collect();

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

        for block in &dag.blocks {
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

    /// `tau`'s output only ever grows: recomputing it after inserting the
    /// next block (in generation order, which — thanks to the "strictly
    /// earlier index" sanitization — is always a valid closure-respecting
    /// insertion order) must reproduce the previous output as a strict
    /// prefix. This is the safety property that lets nodes stream finalized
    /// order incrementally instead of recomputing from scratch.
    #[test]
    fn tau_output_is_prefix_stable_as_dag_grows(
        spec in dag_spec_strategy(),
        wavelength in 1u64..=3,
        leader_pick in 0u8..MAX_VALIDATORS,
    ) {
        let dag = build_dag(&spec);
        let n = spec.num_validators as usize;
        let f = fault_tolerance(n);
        let leader_id = 1 + (leader_pick % spec.num_validators);
        let leader_selection = move |_wave: u64| Some(node(leader_id));

        let mut blocklace = Blocklace::new();
        let mut previous_output: Vec<BlockIdentity> = Vec::new();

        for block in &dag.blocks {
            insert(&mut blocklace, block);

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
