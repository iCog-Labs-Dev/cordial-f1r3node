//! Round / depth computation for the blocklace.
//!
//! From the paper  Definition 20 / Algorithm 1.: The **depth** (or round) of a block `b` is the
//! length of the longest path emanating from `b` back to any initial block.
//! Initial blocks have depth 0.
//!
//! Rounds partition the blocklace into layers: round `r` consists of all blocks
//! with depth exactly `r`.  The depth-`d` prefix `B(d)` is the set of all
//! blocks with depth ≤ `d`.
//!
//! Rounds are the foundation of the wave structure (Def. A.10) and the
//! ordering function τ (Def. 5.1).

use std::collections::{HashMap, HashSet};

use crate::block::Block;
use crate::blocklace::Blocklace;
use crate::types::BlockIdentity;

/// Compute the depth (round number) of a single block.
///
/// Depth is the length of the longest path from `block_id` back to any
/// initial (genesis) block in the blocklace.
///
/// - Initial blocks (no predecessors) have depth 0.
/// - For all other blocks: `depth(b) = 1 + max { depth(p) | p ∈ predecessors(b) }`
///
/// Returns `None` if the block is not in the blocklace.
pub fn depth(blocklace: &Blocklace, block_id: &BlockIdentity) -> Option<u64> {
    depth_recursive(blocklace, block_id, &mut HashMap::new())
}

/// Memoized recursive depth computation.
fn depth_recursive(
    blocklace: &Blocklace,
    block_id: &BlockIdentity,
    cache: &mut HashMap<BlockIdentity, u64>,
) -> Option<u64> {
    if let Some(&d) = cache.get(block_id) {
        return Some(d);
    }

    let content = blocklace.content(block_id)?;

    if content.predecessors.is_empty() {
        cache.insert(block_id.clone(), 0);
        return Some(0);
    }

    let mut max_pred_depth: u64 = 0;
    for pred_id in &content.predecessors {
        let pred_depth = depth_recursive(blocklace, pred_id, cache)?;
        max_pred_depth = max_pred_depth.max(pred_depth);
    }

    let d = max_pred_depth + 1;
    cache.insert(block_id.clone(), d);
    Some(d)
}

/// Compute depths for ALL blocks in the blocklace.
///
/// Returns a map from `BlockIdentity` to depth (round number).
/// This is more efficient than calling `depth()` individually for each block,
/// because it shares the memoization cache.
pub fn compute_all_depths(blocklace: &Blocklace) -> HashMap<BlockIdentity, u64> {
    let mut cache = HashMap::new();
    for id in blocklace.dom() {
        depth_recursive(blocklace, id, &mut cache);
    }
    cache
}

/// Return all blocks at exactly depth `d`.
///
/// From the paper: "a round of a blocklace consists of blocks of the same depth."
pub fn blocks_at_depth(blocklace: &Blocklace, d: u64) -> HashSet<Block> {
    let depths = compute_all_depths(blocklace);
    depths
        .iter()
        .filter(|(_, dep)| **dep == d)
        .filter_map(|(id, _)| blocklace.get(id))
        .collect()
}

/// Return the depth-d prefix B(d): all blocks with depth ≤ d.
///
/// From the paper  Definition 20 / Algorithm 1.: "The depth-d prefix of B, denoted B(d),
/// is the set of all blocks with depth less than or equal to d."
pub fn depth_prefix(blocklace: &Blocklace, d: u64) -> HashSet<Block> {
    let depths = compute_all_depths(blocklace);
    depths
        .iter()
        .filter(|(_, dep)| **dep <= d)
        .filter_map(|(id, _)| blocklace.get(id))
        .collect()
}

/// Return the depth-d suffix B̄(d): all blocks with depth > d.
///
/// From the paper  Definition 20 / Algorithm 1.: "The depth-d suffix of B, denoted B̄(d),
/// is the set of all blocks with depth greater than d."
pub fn depth_suffix(blocklace: &Blocklace, d: u64) -> HashSet<Block> {
    let depths = compute_all_depths(blocklace);
    depths
        .iter()
        .filter(|(_, dep)| **dep > d)
        .filter_map(|(id, _)| blocklace.get(id))
        .collect()
}

/// Compute the maximum depth (latest round) in the blocklace.
///
/// Returns `None` if the blocklace is empty.
pub fn max_depth(blocklace: &Blocklace) -> Option<u64> {
    let depths = compute_all_depths(blocklace);
    depths.values().copied().max()
}

/// Check if a round is **cordial**: it contains a supermajority of miners.
///
/// From the paper (Section 3): "A set of blocks by more than ½(n+f) miners
/// is termed a supermajority." Correct miners wait for round r to attain a
/// supermajority before contributing to round r+1.
///
/// `n` = total number of miners, `f` = maximum number of Byzantine miners.
///
/// A supermajority is > (n+f)/2 distinct miners.
pub fn is_round_cordial(blocklace: &Blocklace, round: u64, n: usize, f: usize) -> bool {
    let round_blocks = blocks_at_depth(blocklace, round);
    let distinct_miners: HashSet<_> = round_blocks.iter().map(|b| b.node().clone()).collect();
    // Supermajority: > (n + f) / 2
    // Using integer: distinct_miners * 2 > n + f
    distinct_miners.len() * 2 > n + f
}

/// Find the latest cordial round in the blocklace.
///
/// Returns `None` if no round is cordial.
pub fn latest_cordial_round(blocklace: &Blocklace, n: usize, f: usize) -> Option<u64> {
    let max_d = max_depth(blocklace)?;
    // Scan from latest round backward
    (0..=max_d)
        .rev()
        .find(|&d| is_round_cordial(blocklace, d, n, f));
    None
}
