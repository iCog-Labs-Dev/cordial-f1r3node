use crate::blocklace::Blocklace;
use crate::types::{BlockIdentity, NodeId};
use std::collections::HashMap;
use std::collections::HashSet;

use crate::block::Block;
use crate::consensus::cordiality::super_ratifies;
use crate::consensus::round::blocks_at_depth;
use crate::consensus::wave::{last_round_of_wave, leader_round_of_wave};

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
            blocklace
                .tip_of(node)
                .is_some_and(|tip| blocklace.preceedes_or_equals(block_id, &tip.identity))
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
    finalized
        .iter()
        .find(|candidate| {
            !finalized
                .iter()
                .any(|other| other != *candidate && blocklace.precedes(candidate, other))
        })
        .cloned()
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
            blocklace
                .tip_of(node)
                .is_some_and(|tip| blocklace.preceedes_or_equals(block_id, &tip.identity))
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

/// Return the single deterministic leader block for a wave.
///
/// Per Definition A.10 of arXiv:2205.09174, a leader block is a block by
/// the elected leader validator in the first round of the wave.
///
/// If the leader equivocated (produced multiple blocks in the leader round),
/// tie-break deterministically by selecting the block with the
/// lexicographically lowest `content_hash` byte value.
///
/// Returns `None` if:
/// - the wavelength is zero
/// - the leader round has no block by the elected leader
/// - `leader_selection` returns `None` for this wave
pub fn leader_block_for_wave<F>(
    blocklace: &Blocklace,
    wave: u64,
    wavelength: u64,
    leader_selection: F,
) -> Option<BlockIdentity>
where
    F: Fn(u64) -> Option<NodeId>,
{
    // Find the elected leader for this wave
    let leader = leader_selection(wave)?;

    // Find the first round of the wave — that is where leader blocks live
    let leader_round = leader_round_of_wave(wave, wavelength)?;

    // Collect all blocks by the leader in the leader round
    let mut leader_blocks: Vec<Block> = blocks_at_depth(blocklace, leader_round)
        .into_iter()
        .filter(|block| block.identity.creator == leader)
        .collect();

    if leader_blocks.is_empty() {
        return None;
    }

    // Deterministic tie-break: lowest content_hash byte value
    // This ensures identical inputs always produce identical output
    // regardless of iteration order, even when the leader equivocated.
    leader_blocks.sort_by_key(|a| a.identity.content_hash);

    Some(leader_blocks[0].identity.clone())
}

/// Check whether a leader block has achieved finality within its wave.
///
/// Per Definition 24 of arXiv:2205.09174, a leader block `b` of round `r`
/// is final if it is super-ratified within `B(r + w - 1)` — the prefix of
/// the blocklace up to the last round of the wave.
///
/// This wraps `super_ratifies` using the set of all blocks within the
/// wave's boundary as the witness set.
///
/// Returns `false` if:
/// - the candidate block is not in the blocklace
/// - the wave boundaries cannot be computed
/// - super-ratification is not achieved
pub fn is_final_leader<F>(
    blocklace: &Blocklace,
    candidate: &BlockIdentity,
    wavelength: u64,
    n: usize,
    f: usize,
    leader_selection: F,
) -> bool
where
    F: Fn(u64) -> Option<NodeId>,
{
    // Get the candidate block
    let candidate_block = match blocklace.get(candidate) {
        Some(block) => block,
        None => return false,
    };

    // Find the round of the candidate block
    let candidate_round = match crate::consensus::round::depth(blocklace, candidate) {
        Some(d) => d,
        None => return false,
    };

    // The candidate must actually be the leader block for its wave
    let wave = match crate::consensus::wave::wave_of_round(candidate_round, wavelength) {
        Some(w) => w,
        None => return false,
    };

    // Verify this is actually the leader block for this wave
    let expected_leader_id = leader_block_for_wave(blocklace, wave, wavelength, &leader_selection);
    if expected_leader_id.as_ref() != Some(candidate) {
        return false;
    }

    // Collect all blocks within the wave boundary B(r + w - 1)
    let last_round = match last_round_of_wave(wave, wavelength) {
        Some(r) => r,
        None => return false,
    };

    // PERF/PAPER NOTE:
    // Def 24 requires checking the prefix B(r + w - 1).
    // However, blocks created *before* candidate_round physically cannot
    // observe the candidate, and therefore cannot ratify it.
    // We only collect witness blocks from candidate_round up to the end
    // of the wave, which is mathematically equivalent to B(r + w - 1).
    let witness_blocks: HashSet<Block> = (candidate_round..=last_round)
        .flat_map(|round| blocks_at_depth(blocklace, round))
        .collect();

    // A final leader is super-ratified within its wave
    super_ratifies(blocklace, &witness_blocks, &candidate_block, n, f)
}
