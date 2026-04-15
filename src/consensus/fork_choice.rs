//! Fork choice and cordial condition.
//!
//! This module contains two kinds of functions:
//!
//! **Paper-native** (used by the Cordial Miners protocol):
//! - `is_cordial()` — the cordial condition from the paper
//! - `collect_validator_tips()` — per-validator latest block
//!
//! **f1r3node compatibility shim** (NOT used by the protocol):
//! - `fork_choice()` and `ForkChoice` — LMD-GHOST-style ranked tips
//! - `compute_lca()`, `walk_to_lca()` — helpers for the shim
//!
//! The Cordial Miners paper does not define a traditional fork choice rule.
//! Validators reference ALL known tips (cordial condition) instead of picking
//! one "preferred" fork. The `fork_choice()` function exists only to satisfy
//! f1r3node's `Casper::estimator()` trait method when we build the adapter
//! in Phase 3. It will likely move to `src/bridge/` at that point.

use std::collections::{HashMap, HashSet};

use crate::block::Block;
use crate::blocklace::Blocklace;
use crate::types::{BlockIdentity, NodeId};

/// The result of running the fork choice rule over a blocklace.
///
/// Analogous to f1r3node's `ForkChoice { tips, lca }` from `estimator.rs`,
/// but derived from the blocklace structure rather than a separate justification model.
///
/// From the paper (§4): the fork choice is determined by the weighted tips
/// of the blocklace, where each validator's tip carries its stake weight.
#[derive(Debug, Clone, PartialEq)]
pub struct ForkChoice {
    /// Tips ranked by descending stake weight.
    /// The first element is the "preferred" fork.
    pub tips: Vec<BlockIdentity>,

    /// Lowest Common Ancestor of all tips — the most recent block
    /// agreed upon by all validators.
    pub lca: BlockIdentity,

    /// Score map: each block between LCA and tips gets the sum of
    /// stake from validators whose tips have that block in their ancestry.
    pub scores: HashMap<BlockIdentity, u64>,
}

/// Compute the global fork choice over the blocklace.
///
/// This replaces CBC Casper's LMD-GHOST estimator. The algorithm:
///
/// 1. Collect each validator's tip (most recent block)
/// 2. Exclude Byzantine equivocators (chain axiom violators)
/// 3. Weight each tip by its validator's stake from the bonds map
/// 4. Compute the LCA of all weighted tips
/// 5. Build a score map: for each validator, walk from their tip
///    back to the LCA, accumulating their stake on each block
/// 6. Rank tips by their score (descending), break ties by hash (ascending)
///
/// Returns `None` if the blocklace is empty or no valid tips exist.
pub fn fork_choice(
    blocklace: &Blocklace,
    bonds: &HashMap<NodeId, u64>,
) -> Option<ForkChoice> {
    // 1. Collect tips from all bonded validators
    let equivocators = blocklace.find_equivacators();

    let validator_tips: Vec<(NodeId, Block)> = bonds
        .keys()
        .filter(|node| !equivocators.contains(node))
        .filter_map(|node| {
            blocklace.tip_of(node).map(|tip| (node.clone(), tip))
        })
        .collect();

    if validator_tips.is_empty() {
        return None;
    }

    // 2. Compute LCA of all tips
    let tip_ids: Vec<&BlockIdentity> = validator_tips
        .iter()
        .map(|(_, block)| &block.identity)
        .collect();

    let lca = compute_lca(blocklace, &tip_ids)?;

    // 3. Build score map: walk from each tip back to LCA,
    //    accumulating the validator's stake on each block
    let mut scores: HashMap<BlockIdentity, u64> = HashMap::new();

    for (node, tip) in &validator_tips {
        let stake = bonds.get(node).copied().unwrap_or(0);
        if stake == 0 {
            continue;
        }

        // Accumulate stake on every block from tip down to (and including) LCA
        let supporting_chain = walk_to_lca(blocklace, &tip.identity, &lca);
        for block_id in supporting_chain {
            *scores.entry(block_id).or_insert(0) += stake;
        }
    }

    // 4. Rank tips by score (descending), break ties by content_hash (ascending)
    let mut ranked_tips: Vec<BlockIdentity> = validator_tips
        .iter()
        .map(|(_, block)| block.identity.clone())
        .collect();

    // Deduplicate (multiple validators might share the same tip)
    let mut seen = HashSet::new();
    ranked_tips.retain(|id| seen.insert(id.clone()));

    ranked_tips.sort_by(|a, b| {
        let score_a = scores.get(a).copied().unwrap_or(0);
        let score_b = scores.get(b).copied().unwrap_or(0);
        score_b.cmp(&score_a) // descending by score
            .then_with(|| a.content_hash.cmp(&b.content_hash)) // ascending by hash
    });

    Some(ForkChoice {
        tips: ranked_tips,
        lca,
        scores,
    })
}

/// Compute the Lowest Common Ancestor of a set of block identities.
///
/// Algorithm (analogous to f1r3node's `lowest_universal_common_ancestor_many`):
/// - Compute the ancestor set (inclusive) of each tip
/// - Intersect all ancestor sets
/// - Return the block in the intersection that no other intersection member precedes
///   (i.e., the "highest" common ancestor)
///
/// Returns `None` if no common ancestor exists.
fn compute_lca(
    blocklace: &Blocklace,
    tips: &[&BlockIdentity],
) -> Option<BlockIdentity> {
    if tips.is_empty() {
        return None;
    }

    if tips.len() == 1 {
        return Some(tips[0].clone());
    }

    // Compute ancestor sets (inclusive) for each tip
    let ancestor_sets: Vec<HashSet<BlockIdentity>> = tips
        .iter()
        .map(|tip| {
            blocklace
                .ancestors_inclusive(tip)
                .into_iter()
                .map(|b| b.identity)
                .collect()
        })
        .collect();

    // Intersect all ancestor sets
    let mut common: HashSet<BlockIdentity> = ancestor_sets[0].clone();
    for set in &ancestor_sets[1..] {
        common = common.intersection(set).cloned().collect();
    }

    if common.is_empty() {
        return None;
    }

    // Find the "highest" block in the intersection: the one that
    // no other common ancestor precedes it from below.
    // In other words, no other block in `common` has this block as an ancestor.
    let common_vec: Vec<BlockIdentity> = common.iter().cloned().collect();
    common_vec.into_iter().find(|candidate| {
        !common.iter().any(|other| {
            other != candidate && blocklace.precedes(candidate, other)
        })
    })
}

/// Walk from `start` back toward `target` (the LCA), collecting all block
/// identities on the path (inclusive of both endpoints).
///
/// Uses BFS over the predecessor graph, stopping when we reach the LCA
/// or a block that is not an ancestor of the start.
fn walk_to_lca(
    blocklace: &Blocklace,
    start: &BlockIdentity,
    target: &BlockIdentity,
) -> Vec<BlockIdentity> {
    let mut result = Vec::new();
    let mut visited = HashSet::new();
    let mut queue = vec![start.clone()];

    while let Some(current) = queue.pop() {
        if !visited.insert(current.clone()) {
            continue;
        }

        result.push(current.clone());

        // Stop expanding past the LCA
        if current == *target {
            continue;
        }

        // Walk predecessors
        if let Some(content) = blocklace.content(&current) {
            for pred_id in &content.predecessors {
                if !visited.contains(pred_id)
                    && blocklace.preceedes_or_equals(target, pred_id)
                {
                    queue.push(pred_id.clone());
                }
            }
        }
    }

    result
}

/// Collect tips from all bonded, non-equivocating validators.
///
/// This is a utility used by fork_choice but also useful standalone
/// for the cordial condition check.
pub fn collect_validator_tips(
    blocklace: &Blocklace,
    bonds: &HashMap<NodeId, u64>,
) -> HashMap<NodeId, BlockIdentity> {
    let equivocators = blocklace.find_equivacators();

    bonds
        .keys()
        .filter(|node| !equivocators.contains(node))
        .filter_map(|node| {
            blocklace
                .tip_of(node)
                .map(|tip| (node.clone(), tip.identity))
        })
        .collect()
}

/// Check if a block satisfies the cordial condition:
/// its predecessors include all tips the creator should have seen.
///
/// From the paper: a block is "cordial" if it references all known
/// validator tips at the time of creation.
pub fn is_cordial(
    block: &Block,
    known_tips: &HashMap<NodeId, BlockIdentity>,
) -> bool {
    known_tips.values().all(|tip_id| {
        block.content.predecessors.contains(tip_id)
            || block.identity == *tip_id
    })
}
