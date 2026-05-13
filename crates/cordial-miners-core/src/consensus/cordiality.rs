//! Equivocation and cordiality predicates for Cordial Miners.
//!
//! This module gathers the protocol-facing DAG predicates that sit between the
//! structural helpers (`round`, `wave`) and the enforcement layer
//! (`validation`).
//!
//! The paper distinguishes:
//! - equivocation: a validator produces multiple conflicting blocks
//! - cordiality: a block does not hide relevant information from the DAG view
//!
//! In this implementation, the "known" portion of "known equivocations" is
//! interpreted conservatively as "already present in the local blocklace".
//! That makes these predicates usable inside block validation, where the
//! creator's private local view is not available.

use std::collections::{HashMap, HashSet};

use crate::block::Block;
use crate::blocklace::Blocklace;
use crate::consensus::approval::approves;
use crate::consensus::round::{blocks_at_depth, depth};
use crate::types::{BlockIdentity, NodeId};

/// A same-round equivocation detected in the blocklace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Equivocation {
    pub creator: NodeId,
    pub round: u64,
    pub blocks: Vec<BlockIdentity>,
}

/// A globally known equivocation that a candidate block fails to acknowledge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HiddenEquivocation {
    pub creator: NodeId,
    pub round: u64,
    pub hidden: Vec<BlockIdentity>,
}

/// Return all blocks  created by 'creator' at exactly 'round' in the blocklace.
pub fn creator_blocks_at_round(
    blocklace: &Blocklace,
    creator: &NodeId,
    round: u64,
) -> HashSet<Block> {
    blocks_at_depth(blocklace, round)
        .into_iter()
        .filter(|b| b.identity.creator == *creator)
        .collect()
}

/// Return all same-round equivocation branches for `creator` at `round`.
/// Under the user story for this task, a validator equivocates when they create at least two different blocks in the exact same round.
pub fn equivocation_blocks_at_round(
    blocklace: &Blocklace,
    creator: &NodeId,
    round: u64,
) -> HashSet<Block> {
    let blocks = creator_blocks_at_round(blocklace, creator, round);
    if blocks.len() >= 2 {
        blocks
    } else {
        HashSet::new()
    }
}

/// Return every same-round equivocation currently present in the blocklace.
pub fn all_equivocations(blocklace: &Blocklace) -> Vec<Equivocation> {
    let Some(max_round) = blocklace
        .dom()
        .into_iter()
        .filter_map(|id| depth(blocklace, id))
        .max()
    else {
        return Vec::new();
    };
    let creators: HashSet<NodeId> = blocklace
        .dom()
        .iter()
        .map(|id| id.creator.clone())
        .collect();

    let mut equivocations = Vec::new();

    for creator in creators {
        for round in 0..=max_round {
            let mut blocks: Vec<BlockIdentity> =
                equivocation_blocks_at_round(blocklace, &creator, round)
                    .into_iter()
                    .map(|b| b.identity)
                    .collect();

            if blocks.len() >= 2 {
                blocks.sort();
                equivocations.push(Equivocation {
                    creator: creator.clone(),
                    round,
                    blocks,
                });
            }
        }
    }

    equivocations
}

/// Return the set of block ids acknowledged by a candidate block through its
/// predecessor closure.
///
/// This is the reconstructed DAG view induced by the candidate's declared
/// predecessors, without inserting the candidate into the blocklace.
pub fn observed_block_ids(blocklace: &Blocklace, block: &Block) -> HashSet<BlockIdentity> {
    let mut observed = HashSet::new();

    for pred_id in &block.content.predecessors {
        observed.extend(blocklace.observe(pred_id));
    }

    observed
}

/// Return whether `block` acknowledges every branch of the same-round
/// equivocation by `creator` at `round`.
pub fn acknowledges_equivocation(
    blocklace: &Blocklace,
    block: &Block,
    creator: &NodeId,
    round: u64,
) -> bool {
    let equivocation = equivocation_blocks_at_round(blocklace, creator, round);
    if equivocation.is_empty() {
        return true;
    }

    let observed = observed_block_ids(blocklace, block);
    equivocation
        .iter()
        .all(|equiv_block| observed.contains(&equiv_block.identity))
}

/// Return the globally known equivocations hidden by `block`.
///
/// This uses the local blocklace as the source of knowledge. If the blocklace
/// already contains a same-round equivocation, then a candidate block is
/// considered to hide it when its predecessor closure does not acknowledge all
/// branches.
pub fn hidden_equivocations(blocklace: &Blocklace, block: &Block) -> Vec<HiddenEquivocation> {
    let observed = observed_block_ids(blocklace, block);

    all_equivocations(blocklace)
        .into_iter()
        .filter_map(|equivocation| {
            let hidden: Vec<BlockIdentity> = equivocation
                .blocks
                .iter()
                .filter(|id| !observed.contains(*id))
                .cloned()
                .collect();

            if hidden.is_empty() {
                None
            } else {
                Some(HiddenEquivocation {
                    creator: equivocation.creator,
                    round: equivocation.round,
                    hidden,
                })
            }
        })
        .collect()
}

// Return the validator tips ommitted by 'block'.
pub fn missing_known_tips(
    block: &Block,
    known_tips: &HashMap<NodeId, BlockIdentity>,
) -> Vec<BlockIdentity> {
    let mut missing: Vec<BlockIdentity> = known_tips
        .values()
        .filter(|tip_id| {
            block.identity != **tip_id && !block.content.predecessors.contains(*tip_id)
        })
        .cloned()
        .collect();
    missing.sort();
    missing
}

/// Check whether a block is cordial with respect to:
/// - known validator tips, and
/// - globally known same-round equivocations already present in the blocklace.
pub fn is_cordial_block(
    blocklace: &Blocklace,
    block: &Block,
    known_tips: &HashMap<NodeId, BlockIdentity>,
) -> bool {
    missing_known_tips(block, known_tips).is_empty()
        && hidden_equivocations(blocklace, block).is_empty()
}

/// Check whether a block b ratifies block b' when:
/// closure of b includes a supermajority of blocks that approve b'.
///
/// Per Definition 22 from Cordial Miners paper: "A block b ratifies b' if the closure
/// of b includes a supermajority of blocks that approve b'"
pub fn ratifies(
    blocklace: &Blocklace,
    ratifier: &Block,
    target: &Block,
    n: usize,
    f: usize,
) -> bool {
    // Ratification is defined over the closure [b] of the ratifier block,
    // which is inclusive of the ratifier itself.
    let observed_ids = blocklace.observe(&ratifier.identity);

    // find all blocks in ratifier's closure that approve target
    let approving: HashSet<Block> = observed_ids
        .iter()
        .filter_map(|id| blocklace.get(id))
        .filter(|block| approves(blocklace, &block.identity, &target.identity))
        .collect();

    is_supermajority(&approving, n, f)
}

/// Check whether a set of blocks super-ratifies a block b' when:
///
/// A set B super-ratifies a block b' if it includes a supermajority of blocks that ratify b'.
pub fn super_ratifies(
    blocklace: &Blocklace,
    blocks: &HashSet<Block>,
    target: &Block,
    n: usize,
    f: usize,
) -> bool {
    let ratifying_blocks: HashSet<Block> = blocks
        .iter()
        .filter(|b| ratifies(blocklace, b, target, n, f))
        .cloned()
        .collect();

    is_supermajority(&ratifying_blocks, n, f)
}

/// Check if a set of blocks constitutes a supermajority.
///
/// Supermajority: > (n+f)/2 distinct creators
pub fn is_supermajority(blocks: &HashSet<Block>, n: usize, f: usize) -> bool {
    let distinct_creators: HashSet<_> =
        blocks.iter().map(|block| &block.identity.creator).collect();

    distinct_creators.len() > (n + f) / 2
}
