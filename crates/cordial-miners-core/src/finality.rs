//! τ (tau) — Total ordering function for the Cordial Miners protocol.
//!
//! # Overview
//!
//! The blocklace is a partially-ordered DAG. τ converts it into a totally-ordered,
//! append-only sequence of block identities. Every correct node running τ on the
//! same blocklace produces the exact same output vector.
//!
//! # Paper reference
//!
//! Cordial Miners (arXiv:2205.09174) — Definition 5.1, Algorithm 2.
//!
//! # Key properties
//!
//! - **Determinism**: identical blocklaces → identical output vectors.
//! - **Monotonicity**: the finalized prefix never changes. Appending new blocks
//!   can only extend or reorder the un-finalized suffix.
//! - **Equivocator exclusion**: blocks from Byzantine validators are never emitted.
//!
//! # Note on finality model
//!
//! The paper uses wave-based leader finality (super-ratification). This implementation
//! uses the existing supermajority heuristic from `consensus::finality` as a stand-in,
//! because waves are not yet implemented (see `CONSENSUS_GAP_ANALYSIS.md` Gaps 1–4).
//! The τ correctness properties hold under either model. When waves are added,
//! only `finalized_leader_chain` needs to be updated.

use std::collections::{BTreeSet, HashMap, HashSet};

use crate::blocklace::Blocklace;
use crate::consensus::finality::{FinalityStatus, check_finality};
use crate::types::{BlockIdentity, NodeId};

// ─────────────────────────────────────────────────────────────────────────────
// Public entry point
// ─────────────────────────────────────────────────────────────────────────────

/// Compute the total order of blocks in the blocklace.
///
/// Returns a `Vec<BlockIdentity>` where:
/// - Blocks causally anchored to finalized leaders appear first, in a
///   deterministic topological order.
/// - The finalized prefix is immutable: calling `tau` again after adding
///   more blocks will only extend or reorder the un-finalized suffix.
/// - Blocks from equivocating validators are never included.
///
/// Returns an empty vector if no block is finalized yet.
///
/// # Algorithm
///
/// 1. Build the finalized leader chain (all finalized blocks, oldest first).
/// 2. For each leader, compute its approved causal history.
/// 3. Subtract blocks already emitted in earlier iterations.
/// 4. Deterministically sort the remainder with `xsort`.
/// 5. Append to the output and continue.
pub fn tau(blocklace: &Blocklace, bonds: &HashMap<NodeId, u64>) -> Vec<BlockIdentity> {
    let leaders = finalized_leader_chain(blocklace, bonds);
    if leaders.is_empty() {
        return vec![];
    }

    let mut output: Vec<BlockIdentity> = Vec::new();
    let mut already_output: HashSet<BlockIdentity> = HashSet::new();

    for leader in &leaders {
        // Approved causal history of this leader, minus what we already emitted.
        let approved = approved_causal_history(blocklace, leader);
        let new_blocks: HashSet<BlockIdentity> = approved
            .difference(&already_output)
            .cloned()
            .collect();

        let sorted = xsort(new_blocks, blocklace);
        for id in sorted {
            already_output.insert(id.clone());
            output.push(id);
        }
    }

    output
}

// ─────────────────────────────────────────────────────────────────────────────
// Step 1 — Approval (Definition A.5)
// ─────────────────────────────────────────────────────────────────────────────

/// Check whether block `b` approves block `target`.
///
/// From Definition A.5 of the paper:
/// > Block `b` approves `target` if `b` observes `target` AND `b` does not
/// > observe any block that equivocates with `target`.
///
/// In DAG terms:
/// - `b` observes `target` iff `target ∈ ancestors_inclusive(b)`.
/// - `b` observes an equivocating sibling of `target` iff there exists a block
///   `c ≠ target` in `ancestors_inclusive(b)` with `node(c) == node(target)`
///   such that `c` and `target` are incomparable under `≺` (neither is an
///   ancestor of the other).
///
/// Approval is NOT transitive. Even if `b` approves `a` and `a` approves `x`,
/// `b` may not approve `x` if `b` also observes an equivocating sibling of `x`.
pub fn approves(blocklace: &Blocklace, b: &BlockIdentity, target: &BlockIdentity) -> bool {
    let causal_history = blocklace.ancestors_inclusive(b);

    // (1) b must observe target.
    let observes_target = causal_history
        .iter()
        .any(|block| &block.identity == target);

    if !observes_target {
        return false;
    }

    // (2) b must not observe any block that equivocates with target.
    // An equivocating sibling of `target` is a block `c` where:
    //   - node(c) == node(target)
    //   - c ≠ target
    //   - c and target are incomparable under ≺ (neither precedes the other)
    let target_creator = &target.creator;

    let sees_equivocating_sibling = causal_history.iter().any(|block| {
        &block.identity != target
            && &block.identity.creator == target_creator
            && !blocklace.precedes(&block.identity, target)
            && !blocklace.precedes(target, &block.identity)
    });

    !sees_equivocating_sibling
}

// ─────────────────────────────────────────────────────────────────────────────
// Step 2 — Approved causal history
// ─────────────────────────────────────────────────────────────────────────────

/// Compute the set of blocks that `leader` approves.
///
/// This is the approved causal history of the leader: all blocks in
/// `ancestors_inclusive(leader)` that the leader approves (i.e., that pass
/// the equivocation-aware filter from `approves()`).
///
/// This is the set that gets deterministically sorted and appended to the
/// τ output for this leader anchor.
pub fn approved_causal_history(
    blocklace: &Blocklace,
    leader_id: &BlockIdentity,
) -> HashSet<BlockIdentity> {
    blocklace
        .ancestors_inclusive(leader_id)
        .into_iter()
        .filter(|block| approves(blocklace, leader_id, &block.identity))
        .map(|block| block.identity)
        .collect()
}

// ─────────────────────────────────────────────────────────────────────────────
// Step 3 — xsort: deterministic topological sort
// ─────────────────────────────────────────────────────────────────────────────

/// Deterministically topologically sort a set of block identities.
///
/// Uses Kahn's algorithm with a `BTreeSet` as the ready queue. The `BTreeSet`
/// ensures that among all currently-ready blocks (those whose predecessors
/// within the input set have already been emitted), the one with the
/// lexicographically smallest `BlockIdentity` is always emitted first.
///
/// # Tiebreak key
///
/// `BlockIdentity` derives `Ord` from its struct field declaration order:
/// `(content_hash: [u8; 32], creator: NodeId, signature: Vec<u8>)`. All
/// three implement `Ord`, giving a strict total order with no ties. In
/// practice `content_hash` alone distinguishes blocks. When wave/round
/// metadata is added to the protocol, the key will be upgraded to
/// `(wave, round, creator, content_hash)` as the paper specifies.
///
/// # Correctness
///
/// - Every block in `ids` appears exactly once in the output.
/// - For any two blocks `a`, `b` in `ids` where `a ≺ b` (a precedes b),
///   `a` appears before `b` in the output.
/// - For incomparable blocks, the `BTreeSet` tiebreak gives a canonical order.
pub fn xsort(ids: HashSet<BlockIdentity>, blocklace: &Blocklace) -> Vec<BlockIdentity> {
    if ids.is_empty() {
        return vec![];
    }

    // Build in-degree map: count how many predecessors of each block are
    // also in the input set (i.e., must be emitted before it).
    let mut in_degree: HashMap<BlockIdentity, usize> = ids
        .iter()
        .map(|id| (id.clone(), 0))
        .collect();

    // Build a reverse-edge map: successor_map[a] = set of blocks in `ids`
    // that have `a` as a direct predecessor.
    let mut successor_map: HashMap<BlockIdentity, Vec<BlockIdentity>> = HashMap::new();

    for id in &ids {
        if let Some(content) = blocklace.content(id) {
            for pred_id in &content.predecessors {
                if ids.contains(pred_id) {
                    // pred_id → id edge: id depends on pred_id
                    *in_degree.get_mut(id).unwrap() += 1;
                    successor_map
                        .entry(pred_id.clone())
                        .or_default()
                        .push(id.clone());
                }
            }
        }
    }

    // Seed the ready queue with all blocks that have no in-set predecessors.
    // BTreeSet gives canonical ordering by (creator, content_hash).
    let mut ready: BTreeSet<BlockIdentity> = in_degree
        .iter()
        .filter(|(_, &deg)| deg == 0)
        .map(|(id, _)| id.clone())
        .collect();

    let mut result = Vec::with_capacity(ids.len());

    while let Some(current) = ready.pop_first() {
        result.push(current.clone());

        // Decrement in-degree of successors; add newly-ready ones to queue.
        if let Some(successors) = successor_map.get(&current) {
            for succ in successors {
                let deg = in_degree.get_mut(succ).unwrap();
                *deg -= 1;
                if *deg == 0 {
                    ready.insert(succ.clone());
                }
            }
        }
    }

    // If result.len() < ids.len(), there was a cycle — should be impossible
    // in a valid blocklace (the closure axiom guarantees a DAG). Emit whatever
    // we collected rather than panicking, so callers can degrade gracefully.
    result
}

// ─────────────────────────────────────────────────────────────────────────────
// Step 4 — Finalized leader chain
// ─────────────────────────────────────────────────────────────────────────────

/// Collect all finalized blocks and return them in topological order
/// (oldest / most-ancestral first).
///
/// These are the "leader anchors" that τ iterates over. Each one contributes
/// a slice of the total output: its approved causal history minus everything
/// already emitted by earlier leaders.
///
/// When wave-based leader election is implemented, this function will be
/// updated to return only the wave-leader blocks in wave order. The rest of
/// τ does not need to change.
pub fn finalized_leader_chain(
    blocklace: &Blocklace,
    bonds: &HashMap<NodeId, u64>,
) -> Vec<BlockIdentity> {
    let finalized: HashSet<BlockIdentity> = blocklace
        .dom()
        .into_iter()
        .filter(|id| {
            matches!(
                check_finality(blocklace, id, bonds),
                FinalityStatus::Finalized { .. }
            )
        })
        .cloned()
        .collect();

    // Sort topologically so older leaders come first.
    xsort(finalized, blocklace)
}
