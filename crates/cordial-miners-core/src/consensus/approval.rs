//! Approval predicates for Cordial Miners.
//!
//! This module implements the approval relation that determines whether a block
//! approves a target block according to the Cordial Miners protocol.
//!
//! See Definition 18 of "Cordial Miners: Voluntary Participation in Blockchains"
//! (arXiv:2205.09174) for the formal specification.

use std::collections::{BTreeSet, HashMap, HashSet};

use crate::block::Block;
use crate::blocklace::Blocklace;
use crate::types::{BlockIdentity, NodeId};

#[derive(Default)]
pub(crate) struct ApprovalMemo {
    pub(crate) observe_cache: HashMap<BlockIdentity, BTreeSet<BlockIdentity>>,
    approves_cache: HashMap<(BlockIdentity, BlockIdentity), bool>,
    creator_blocks_cache: HashMap<NodeId, Vec<Block>>,
}

/// A block `approver` approves a `target` block if:
/// 1. The approver observes the target (i.e., target is in the approver's predecessor closure)
/// 2. The approver does not observe any equivocating sibling of the target
///
/// This implements Definition 18 from the Cordial Miners paper (arXiv:2205.09174).
/// A block approves a target it observes only when no equivocating sibling of that target
/// is also in the observed set.
pub fn approves(blocklace: &Blocklace, approver: &BlockIdentity, target: &BlockIdentity) -> bool {
    let mut memo = ApprovalMemo::default();
    approves_with_memo(blocklace, approver, target, &mut memo)
}

pub(crate) fn approves_with_memo(
    blocklace: &Blocklace,
    approver: &BlockIdentity,
    target: &BlockIdentity,
    memo: &mut ApprovalMemo,
) -> bool {
    let cache_key = (approver.clone(), target.clone());
    if let Some(result) = memo.approves_cache.get(&cache_key) {
        return *result;
    }

    let result = approves_uncached(blocklace, approver, target, memo);
    memo.approves_cache.insert(cache_key, result);
    result
}

fn approves_uncached(
    blocklace: &Blocklace,
    approver: &BlockIdentity,
    target: &BlockIdentity,
    memo: &mut ApprovalMemo,
) -> bool {
    if blocklace.get(approver).is_none() {
        return false;
    }

    // Populate approver's observe cache entry and check target visibility in a
    // short block so the borrow on memo ends before we mutate observe_cache
    // again inside the creator-blocks loop below.
    {
        let observed = memo
            .observe_cache
            .entry(approver.clone())
            .or_insert_with(|| blocklace.observe(approver));

        if !observed.contains(target) {
            return false;
        }
    }

    // Get the target block to determine its creator.
    let target_block = match blocklace.get(target) {
        Some(block) => block,
        None => return false,
    };

    // Collect just the identities of other blocks by the same creator so we
    // can iterate without holding a borrow on memo.creator_blocks_cache while
    // mutating memo.observe_cache inside the loop.
    let creator = target_block.identity.creator.clone();
    if !memo.creator_blocks_cache.contains_key(&creator) {
        memo.creator_blocks_cache
            .insert(creator.clone(), blocklace.blocks_by(&creator));
    }
    let other_ids: Vec<BlockIdentity> = memo.creator_blocks_cache[&creator]
        .iter()
        .filter(|b| b.identity != *target)
        .map(|b| b.identity.clone())
        .collect();

    // Approval excludes any OTHER observed block by the same creator that is
    // incomparable with the target (i.e. an equivocating sibling).
    //
    // PERF: `precedes(a, b)` ≡ `a ∈ observe(b)`.  Both sides of the
    // comparability test are answered in O(log N) via the observe_cache rather
    // than with a fresh O(V×W) BFS per call.
    for other_id in &other_ids {
        // target_precedes_other  ≡  target ∈ observe(other)
        if !memo.observe_cache.contains_key(other_id) {
            memo.observe_cache
                .insert(other_id.clone(), blocklace.observe(other_id));
        }
        let target_precedes_other = memo.observe_cache[other_id].contains(target);

        // other_precedes_target  ≡  other ∈ observe(target)
        if !memo.observe_cache.contains_key(target) {
            memo.observe_cache
                .insert(target.clone(), blocklace.observe(target));
        }
        let other_precedes_target = memo.observe_cache[target].contains(other_id);

        // approver_observes_other — re-borrow after the mutations above.
        let approver_observes_other = memo.observe_cache[approver].contains(other_id);

        if !target_precedes_other && !other_precedes_target && approver_observes_other {
            return false;
        }
    }

    true
}

/// Return the set of all blocks in the blocklace that approve the target.
///
/// This function walks every block in the blocklace and collects those blocks for which
/// the `approves` predicate returns true with respect to the target.
///
/// See Definition 18 of "Cordial Miners: Voluntary Participation in Blockchains"
/// (arXiv:2205.09174) for the formal specification of approval.
pub fn approving_blocks(blocklace: &Blocklace, target: &BlockIdentity) -> HashSet<Block> {
    blocklace
        .dom()
        .into_iter()
        .filter_map(|block_id| blocklace.get(block_id))
        .filter(|block| approves(blocklace, &block.identity, target))
        .collect()
}

/// Return the bonded creators in `blocks` whose blocks approve `target`.
///
/// This helper does not redefine approval. It delegates to the paper-native
/// [`approves`] predicate and only changes support accounting from creator
/// cardinality to positive bonded stake.
pub fn weighted_approving_creators(
    blocklace: &Blocklace,
    blocks: &HashSet<Block>,
    target: &BlockIdentity,
    bonds: &HashMap<NodeId, u64>,
) -> HashSet<NodeId> {
    let mut memo = ApprovalMemo::default();
    weighted_approving_creators_with_memo(blocklace, blocks, target, bonds, &mut memo)
}

pub(crate) fn weighted_approving_creators_with_memo(
    blocklace: &Blocklace,
    blocks: &HashSet<Block>,
    target: &BlockIdentity,
    bonds: &HashMap<NodeId, u64>,
    memo: &mut ApprovalMemo,
) -> HashSet<NodeId> {
    blocks
        .iter()
        .filter(|block| approves_with_memo(blocklace, &block.identity, target, memo))
        .filter_map(|block| {
            let creator = &block.identity.creator;
            match bonds.get(creator).copied() {
                Some(weight) if weight > 0 => Some(creator.clone()),
                _ => None,
            }
        })
        .collect()
}
