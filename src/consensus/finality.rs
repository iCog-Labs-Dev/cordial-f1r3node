use std::collections::HashMap;

use crate::blocklace::Blocklace;
use crate::types::{BlockIdentity, NodeId};

/// The finality status of a block in the blocklace.
///
/// From the paper: a block is finalized when a supermajority (> 2/3) of
/// the total honest stake has that block in their causal history (ancestry).
///
/// This replaces CBC Casper's clique oracle, which requires NP-hard clique
/// enumeration bounded by MAX_CLIQUE_CANDIDATES. The blocklace structure
/// makes finality a simple stake summation.
#[derive(Debug, Clone, PartialEq)]
pub enum FinalityStatus {
    /// Block is finalized: > 2/3 of honest stake supports it.
    Finalized {
        /// Total stake of validators supporting this block.
        supporting_stake: u64,
        /// Total honest stake (excluding equivocators).
        total_honest_stake: u64,
    },

    /// Block is not yet finalized but could become so.
    Pending {
        /// Total stake of validators supporting this block so far.
        supporting_stake: u64,
        /// Total honest stake (excluding equivocators).
        total_honest_stake: u64,
    },

    /// Block is not known in the blocklace.
    Unknown,
}

impl FinalityStatus {
    pub fn is_finalized(&self) -> bool {
        matches!(self, FinalityStatus::Finalized { .. })
    }

    pub fn is_pending(&self) -> bool {
        matches!(self, FinalityStatus::Pending { .. })
    }
}

/// Check the finality status of a single block.
///
/// From the paper: a block `b` is finalized when the set of validators
/// whose tips have `b` in their ancestry holds > 2/3 of the total
/// honest stake.
///
/// Algorithm:
/// 1. Compute total honest stake (exclude equivocators)
/// 2. For each bonded, non-equivocating validator, check if their tip
///    has `b` in its ancestry (using `precedes_or_equals`)
/// 3. Sum the stake of supporting validators
/// 4. If supporting_stake * 3 > total_honest_stake * 2, block is finalized
pub fn check_finality(
    blocklace: &Blocklace,
    block_id: &BlockIdentity,
    bonds: &HashMap<NodeId, u64>,
) -> FinalityStatus {
    // Block must exist in the blocklace
    if blocklace.content(block_id).is_none() {
        return FinalityStatus::Unknown;
    }

    let equivocators = blocklace.find_equivacators();

    // Total honest stake = sum of all bonded validators minus equivocators
    let total_honest_stake: u64 = bonds
        .iter()
        .filter(|(node, _)| !equivocators.contains(node))
        .map(|(_, stake)| stake)
        .sum();

    if total_honest_stake == 0 {
        return FinalityStatus::Pending {
            supporting_stake: 0,
            total_honest_stake: 0,
        };
    }

    // Sum stake of validators whose tip has block_id in its ancestry
    let supporting_stake: u64 = bonds
        .iter()
        .filter(|(node, _)| !equivocators.contains(node))
        .filter(|(node, _)| {
            blocklace.tip_of(node).map_or(false, |tip| {
                blocklace.preceedes_or_equals(block_id, &tip.identity)
            })
        })
        .map(|(_, stake)| stake)
        .sum();

    // Supermajority: supporting > 2/3 of total
    // Use integer arithmetic to avoid floating point: s * 3 > t * 2
    if supporting_stake * 3 > total_honest_stake * 2 {
        FinalityStatus::Finalized {
            supporting_stake,
            total_honest_stake,
        }
    } else {
        FinalityStatus::Pending {
            supporting_stake,
            total_honest_stake,
        }
    }
}

/// Scan the blocklace for the most recent finalized block.
///
/// This replaces CBC Casper's `Finalizer::run()` which does a two-phase
/// scan with work budgets and clique oracle calls. In Cordial Miners,
/// we simply check each block for supermajority support.
///
/// Algorithm:
/// 1. Collect all blocks in the blocklace
/// 2. Check finality for each
/// 3. Among finalized blocks, return the one that is an ancestor of no
///    other finalized block (the "highest" finalized block)
///
/// Returns `None` if no block is finalized yet.
pub fn find_last_finalized(
    blocklace: &Blocklace,
    bonds: &HashMap<NodeId, u64>,
) -> Option<BlockIdentity> {
    let dom: Vec<BlockIdentity> = blocklace.dom().into_iter().cloned().collect();

    let finalized: Vec<BlockIdentity> = dom
        .into_iter()
        .filter(|id| check_finality(blocklace, id, bonds).is_finalized())
        .collect();

    if finalized.is_empty() {
        return None;
    }

    // Find the "highest" finalized block: the one not preceded by any
    // other finalized block (no other finalized block has it as ancestor)
    finalized.iter().find(|candidate| {
        !finalized.iter().any(|other| {
            other != *candidate && blocklace.precedes(candidate, other)
        })
    }).cloned()
}

/// Check if a block can still potentially be finalized, or if it has been
/// orphaned (impossible to reach 2/3 even if all remaining validators support it).
///
/// Analogous to CBC Casper's `cannot_be_orphaned` pre-filter, but inverted:
/// returns `true` if the block CAN still be finalized.
///
/// A block is orphaned when: supporting_stake + remaining_stake <= 2/3 * total
/// where remaining_stake is the stake of validators who haven't yet expressed
/// a view (no tip in the blocklace).
pub fn can_be_finalized(
    blocklace: &Blocklace,
    block_id: &BlockIdentity,
    bonds: &HashMap<NodeId, u64>,
) -> bool {
    if blocklace.content(block_id).is_none() {
        return false;
    }

    let equivocators = blocklace.find_equivacators();

    let total_honest_stake: u64 = bonds
        .iter()
        .filter(|(node, _)| !equivocators.contains(node))
        .map(|(_, stake)| stake)
        .sum();

    if total_honest_stake == 0 {
        return false;
    }

    // Current supporting stake
    let supporting_stake: u64 = bonds
        .iter()
        .filter(|(node, _)| !equivocators.contains(node))
        .filter(|(node, _)| {
            blocklace.tip_of(node).map_or(false, |tip| {
                blocklace.preceedes_or_equals(block_id, &tip.identity)
            })
        })
        .map(|(_, stake)| stake)
        .sum();

    // Stake from validators with no tip yet (could still support)
    let undecided_stake: u64 = bonds
        .iter()
        .filter(|(node, _)| !equivocators.contains(node))
        .filter(|(node, _)| blocklace.tip_of(node).is_none())
        .map(|(_, stake)| stake)
        .sum();

    // Can be finalized if max possible support > 2/3
    (supporting_stake + undecided_stake) * 3 > total_honest_stake * 2
}
