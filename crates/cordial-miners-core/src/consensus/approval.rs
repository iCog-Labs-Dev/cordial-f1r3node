//! Approval predicates for Cordial Miners.
//!
//! This module implements the approval relation that determines whether a block
//! approves a target block according to the Cordial Miners protocol.
//!
//! See Definition 18 of "Cordial Miners: Voluntary Participation in Blockchains"
//! (arXiv:2205.09174) for the formal specification.

use std::collections::HashSet;

use crate::block::Block;
use crate::blocklace::Blocklace;
use crate::consensus::cordiality::{equivocation_blocks_at_round, observed_block_ids};
use crate::consensus::round::depth;
use crate::types::BlockIdentity;

/// A block `approver` approves a `target` block if:
/// 1. The approver observes the target (i.e., target is in the approver's predecessor closure)
/// 2. The approver does not observe any equivocating sibling of the target
///
/// This implements Definition 18 from the Cordial Miners paper (arXiv:2205.09174).
/// A block approves a target it observes only when no equivocating sibling of that target
/// is also in the observed set.
pub fn approves(blocklace: &Blocklace, approver: &BlockIdentity, target: &BlockIdentity) -> bool {
    // Get the approver block from the blocklace
    let approver_block = match blocklace.get(approver) {
        Some(block) => block,
        None => return false,
    };

    // Get the set of blocks observed by the approver
    let observed = observed_block_ids(blocklace, &approver_block);

    // Check if target is in the observed set
    if !observed.contains(target) {
        return false;
    }

    // Get the target block to determine its creator and round
    let target_block = match blocklace.get(target) {
        Some(block) => block,
        None => return false,
    };

    // Get the round (depth) of the target block
    let target_round = match depth(blocklace, target) {
        Some(d) => d,
        None => return false,
    };

    // Get all equivocating siblings of target at its round
    let equivocations =
        equivocation_blocks_at_round(blocklace, &target_block.identity.creator, target_round);

    // If no equivocations exist at the target's round, the approver approves the target
    if equivocations.is_empty() {
        return true;
    }

    // Check if any equivocating sibling (other than the target) is in the observed set
    for equiv_block in equivocations {
        // Skip the target itself - we're only checking for OTHER equivocating blocks
        if equiv_block.identity == *target {
            continue;
        }

        if observed.contains(&equiv_block.identity) {
            // An equivocating sibling is observed, so the approver does not approve
            return false;
        }
    }

    // No equivocating sibling is observed, so the approver approves the target
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
