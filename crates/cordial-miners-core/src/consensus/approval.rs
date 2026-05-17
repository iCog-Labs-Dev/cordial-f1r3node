//! Approval predicates for Cordial Miners.
//!
//! This module implements the approval relation that determines whether a block
//! approves a target block according to the Cordial Miners protocol.
//!
//! See Definition 18 of "Cordial Miners: Voluntary Participation in Blockchains"
//! (arXiv:2205.09174) for the formal specification.

use std::collections::{HashMap, HashSet};

use crate::block::Block;
use crate::blocklace::Blocklace;
use crate::types::{BlockIdentity, NodeId};

/// A block `approver` approves a `target` block if:
/// 1. The approver observes the target (i.e., target is in the approver's predecessor closure)
/// 2. The approver does not observe any equivocating sibling of the target
///
/// This implements Definition 18 from the Cordial Miners paper (arXiv:2205.09174).
/// A block approves a target it observes only when no equivocating sibling of that target
/// is also in the observed set.
pub fn approves(blocklace: &Blocklace, approver: &BlockIdentity, target: &BlockIdentity) -> bool {
    if blocklace.get(approver).is_none() {
        return false;
    }

    // Observation in the blocklace is inclusive: a block observes itself and
    // everything in its predecessor closure.
    let observed = blocklace.observe(approver);

    // Check if target is in the observed set
    if !observed.contains(target) {
        return false;
    }

    // Get the target block to determine its creator
    let target_block = match blocklace.get(target) {
        Some(block) => block,
        None => return false,
    };

    // Approval excludes any OTHER observed block by the same creator that is
    // incomparable with the target. This follows the paper's "does not observe
    // any equivocating block of the target" relation more closely than a
    // same-round-only filter.
    for other_block in blocklace.blocks_by(&target_block.identity.creator) {
        if other_block.identity == *target {
            continue;
        }

        let target_precedes_other = blocklace.precedes(target, &other_block.identity);
        let other_precedes_target = blocklace.precedes(&other_block.identity, target);

        if !target_precedes_other
            && !other_precedes_target
            && observed.contains(&other_block.identity)
        {
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

/// Return bonded creators from `blocks` that approve `target`.
///
/// This is the approval-side helper for weighted ratification. It preserves the
/// existing paper-native `approves` predicate, then filters support to validators
/// with positive bond weight so unknown and zero-weight creators cannot
/// contribute stake.
pub fn weighted_approving_creators(
    blocklace: &Blocklace,
    blocks: &HashSet<Block>,
    target: &BlockIdentity,
    bonds: &HashMap<NodeId, u64>,
) -> HashSet<NodeId> {
    blocks
        .iter()
        .filter(|block| approves(blocklace, &block.identity, target))
        .filter_map(|block| {
            let creator = &block.identity.creator;
            match bonds.get(creator).copied() {
                Some(weight) if weight > 0 => Some(creator.clone()),
                _ => None,
            }
        })
        .collect()
}
