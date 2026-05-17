use std::collections::{HashMap, HashSet};

use crate::block::Block;
use crate::blocklace::Blocklace;
use crate::consensus::cordiality::super_ratifies;
use crate::consensus::fork_choice::collect_validator_tips;
use crate::consensus::round::{blocks_at_depth, depth};
use crate::consensus::wave::{last_round_of_wave, leader_blocks_of_wave, wave_of_round};
use crate::types::{BlockIdentity, NodeId};

/// Finality status for the f1r3node compatibility detector.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FinalityStatus {
    Finalized {
        supporting_stake: u64,
        total_honest_stake: u64,
    },
    Pending {
        supporting_stake: u64,
        total_honest_stake: u64,
    },
    Unknown,
}

/// Check whether a block is finalized by weighted ancestry support.
///
/// This compatibility detector is separate from the paper-native leader
/// finality predicate below. It mirrors f1r3node's need for a single last
/// finalized block by treating a block as finalized when more than two thirds
/// of non-equivocating bonded stake has that block in its validator tip
/// ancestry.
pub fn check_finality(
    blocklace: &Blocklace,
    block_id: &BlockIdentity,
    bonds: &HashMap<NodeId, u64>,
) -> FinalityStatus {
    if blocklace.get(block_id).is_none() {
        return FinalityStatus::Unknown;
    }

    let total_honest_stake = total_honest_stake(blocklace, bonds);
    if total_honest_stake == 0 {
        return FinalityStatus::Unknown;
    }

    let supporting_stake = supporting_stake(blocklace, block_id, bonds);
    if strict_two_thirds(supporting_stake, total_honest_stake) {
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

/// Return the highest finalized block in the current blocklace.
///
/// If several finalized blocks have the same depth, the lexicographically
/// lowest content hash is selected for deterministic adapter behavior.
pub fn find_last_finalized(
    blocklace: &Blocklace,
    bonds: &HashMap<NodeId, u64>,
) -> Option<BlockIdentity> {
    let mut finalized: Vec<(u64, BlockIdentity)> = blocklace
        .dom()
        .into_iter()
        .filter_map(|id| {
            if matches!(
                check_finality(blocklace, id, bonds),
                FinalityStatus::Finalized { .. }
            ) {
                depth(blocklace, id).map(|d| (d, id.clone()))
            } else {
                None
            }
        })
        .collect();

    finalized.sort_by(|(depth_a, id_a), (depth_b, id_b)| {
        depth_b
            .cmp(depth_a)
            .then_with(|| id_a.content_hash.cmp(&id_b.content_hash))
    });

    finalized.into_iter().map(|(_, id)| id).next()
}

/// Return true if `block_id` either is finalized now or can still become
/// finalized once validators without current tips publish blocks.
pub fn can_be_finalized(
    blocklace: &Blocklace,
    block_id: &BlockIdentity,
    bonds: &HashMap<NodeId, u64>,
) -> bool {
    if blocklace.get(block_id).is_none() {
        return false;
    }

    let total_honest_stake = total_honest_stake(blocklace, bonds);
    if total_honest_stake == 0 {
        return false;
    }

    let current_support = supporting_stake(blocklace, block_id, bonds);
    let validator_tips = collect_validator_tips(blocklace, bonds);
    let equivocators = blocklace.find_equivacators();
    let undecided_stake: u64 = bonds
        .iter()
        .filter(|(validator, _)| {
            !equivocators.contains(*validator) && !validator_tips.contains_key(*validator)
        })
        .map(|(_, stake)| *stake)
        .sum();

    strict_two_thirds(
        current_support.saturating_add(undecided_stake),
        total_honest_stake,
    )
}

/// Return the unique leader block for a wave when exactly one exists.
///
/// Per Definition A.10 of arXiv:2205.09174, a leader block is a block by
/// the elected leader validator in the first round of the wave.
///
/// Returns `None` if:
/// - the wavelength is zero
/// - the leader round has no block by the elected leader
/// - the elected leader equivocated and produced multiple leader blocks
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
    let mut leader_blocks: Vec<BlockIdentity> =
        leader_blocks_of_wave(blocklace, wave, wavelength, leader_selection)
            .into_iter()
            .map(|block| block.identity)
            .collect();

    if leader_blocks.len() != 1 {
        return None;
    }

    leader_blocks.pop()
}

/// Check whether a leader block has achieved finality within its wave.
///
/// Per Definition 24 of arXiv:2205.09174, a leader block `b` of round `r`
/// is final if it is super-ratified within `B(r + w - 1)` -- the prefix of
/// the blocklace up to the last round of the wave.
///
/// Returns `false` if:
/// - the candidate block is not in the blocklace
/// - the candidate is not one of the actual leader blocks for its wave
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
    let candidate_block = match blocklace.get(candidate) {
        Some(block) => block,
        None => return false,
    };

    let candidate_round = match depth(blocklace, candidate) {
        Some(d) => d,
        None => return false,
    };

    let wave = match wave_of_round(candidate_round, wavelength) {
        Some(w) => w,
        None => return false,
    };

    let leader_blocks = leader_blocks_of_wave(blocklace, wave, wavelength, &leader_selection);
    if !leader_blocks
        .iter()
        .any(|leader_block| leader_block.identity == *candidate)
    {
        return false;
    }

    let last_round = match last_round_of_wave(wave, wavelength) {
        Some(r) => r,
        None => return false,
    };

    // Def. 24 checks B(r + w - 1). Restricting the witness set to rounds from
    // the candidate round through the end of the wave is equivalent for the
    // candidate, because earlier rounds cannot observe and ratify it.
    let witness_blocks: HashSet<Block> = (candidate_round..=last_round)
        .flat_map(|round| blocks_at_depth(blocklace, round))
        .collect();

    super_ratifies(blocklace, &witness_blocks, &candidate_block, n, f)
}

/// Return the final leader block for a wave, if one exists.
///
/// This first resolves the unique leader block for the wave, then checks
/// whether that block is final under Definition 24.
pub fn final_leader_for_wave<F>(
    blocklace: &Blocklace,
    wave: u64,
    wavelength: u64,
    n: usize,
    f: usize,
    leader_selection: F,
) -> Option<BlockIdentity>
where
    F: Fn(u64) -> Option<NodeId> + Copy,
{
    let leader = leader_block_for_wave(blocklace, wave, wavelength, leader_selection)?;
    if is_final_leader(blocklace, &leader, wavelength, n, f, leader_selection) {
        Some(leader)
    } else {
        None
    }
}

/// Return the latest final leader currently known in the blocklace.
///
/// This scans backward from the highest known round, returning the newest wave
/// whose unique leader block is final.
pub fn latest_final_leader<F>(
    blocklace: &Blocklace,
    wavelength: u64,
    n: usize,
    f: usize,
    leader_selection: F,
) -> Option<BlockIdentity>
where
    F: Fn(u64) -> Option<NodeId> + Copy,
{
    if wavelength == 0 || blocklace.dom().is_empty() {
        return None;
    }

    let max_round = blocklace
        .dom()
        .iter()
        .filter_map(|id| depth(blocklace, id))
        .max()?;
    let latest_wave = wave_of_round(max_round, wavelength)?;

    for wave in (0..=latest_wave).rev() {
        if let Some(leader) =
            final_leader_for_wave(blocklace, wave, wavelength, n, f, leader_selection)
        {
            return Some(leader);
        }
    }

    None
}

fn total_honest_stake(blocklace: &Blocklace, bonds: &HashMap<NodeId, u64>) -> u64 {
    let equivocators = blocklace.find_equivacators();
    bonds
        .iter()
        .filter(|(validator, _)| !equivocators.contains(*validator))
        .map(|(_, stake)| *stake)
        .sum()
}

fn supporting_stake(
    blocklace: &Blocklace,
    block_id: &BlockIdentity,
    bonds: &HashMap<NodeId, u64>,
) -> u64 {
    collect_validator_tips(blocklace, bonds)
        .into_iter()
        .filter(|(_, tip_id)| blocklace.observe(tip_id).contains(block_id))
        .filter_map(|(validator, _)| bonds.get(&validator))
        .copied()
        .sum()
}

fn strict_two_thirds(supporting_stake: u64, total_stake: u64) -> bool {
    u128::from(supporting_stake) * 3 > u128::from(total_stake) * 2
}
