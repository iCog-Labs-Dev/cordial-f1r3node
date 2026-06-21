use std::collections::HashMap;
use std::collections::HashSet;

use crate::block::Block;
use crate::blocklace::Blocklace;
use crate::consensus::cordiality::{super_ratifies, weighted_super_ratifies};
use crate::consensus::round::{blocks_at_depth, compute_all_depths, depth};
use crate::consensus::wave::{last_round_of_wave, leader_blocks_of_wave, wave_of_round};
use crate::types::{BlockIdentity, NodeId};

type RoundIndex = HashMap<u64, Vec<Block>>;

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
/// is final if it is super-ratified within `B(r + w - 1)` — the prefix of
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

    let depths = compute_all_depths(blocklace);
    let max_round = depths.values().copied().max()?;
    let rounds = build_round_index(blocklace, &depths);
    let latest_wave = wave_of_round(max_round, wavelength)?;

    for wave in (0..=latest_wave).rev() {
        let Some(leader) =
            unique_leader_block_from_index(&rounds, wave, wavelength, &leader_selection)
        else {
            continue;
        };

        let Some(candidate_round) = depths.get(&leader.identity).copied() else {
            continue;
        };
        let Some(last_round) = last_round_of_wave(wave, wavelength) else {
            continue;
        };

        let witness_blocks = witness_blocks_from_index(&rounds, candidate_round, last_round);
        if super_ratifies(blocklace, &witness_blocks, &leader, n, f) {
            return Some(leader.identity);
        }
    }

    None
}

/// Check whether a leader block has achieved weighted finality within its wave.
///
/// This is the stake-weighted parallel to `is_final_leader`. It uses
/// `weighted_super_ratifies` instead of `super_ratifies`, meaning the
/// supermajority threshold is measured over bonded stake rather than
/// distinct creator count.
///
/// Use `is_final_leader` for paper-native unweighted verification.
/// Use `is_weighted_final_leader` for PoS stake-based consensus.
///
/// Returns `false` if:
/// - the candidate block is not in the blocklace
/// - the candidate is not one of the actual leader blocks for its wave
/// - the wave boundaries cannot be computed
/// - weighted super-ratification is not achieved
pub fn is_weighted_final_leader<F>(
    blocklace: &Blocklace,
    candidate: &BlockIdentity,
    wavelength: u64,
    bonds: &HashMap<NodeId, u64>,
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

    // Candidate must be one of the actual leader blocks for its wave
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

    // PERF/PAPER NOTE:
    // Same optimization as is_final_leader — blocks before candidate_round
    // cannot observe the candidate and therefore cannot ratify it.
    // Collecting from candidate_round..=last_round is mathematically
    // equivalent to B(r + w - 1) per Definition 24.
    let witness_blocks: HashSet<Block> = (candidate_round..=last_round)
        .flat_map(|round| blocks_at_depth(blocklace, round))
        .collect();

    weighted_super_ratifies(blocklace, &witness_blocks, &candidate_block, bonds)
}

/// Return the weighted final leader block for a wave, if one exists.
///
/// Resolves the unique leader block for the wave then checks whether
/// that block is weighted-final under bonded stake.
pub fn weighted_final_leader_for_wave<F>(
    blocklace: &Blocklace,
    wave: u64,
    wavelength: u64,
    bonds: &HashMap<NodeId, u64>,
    leader_selection: F,
) -> Option<BlockIdentity>
where
    F: Fn(u64) -> Option<NodeId> + Copy,
{
    let leader = leader_block_for_wave(blocklace, wave, wavelength, leader_selection)?;
    if is_weighted_final_leader(blocklace, &leader, wavelength, bonds, leader_selection) {
        Some(leader)
    } else {
        None
    }
}

/// Return the latest weighted final leader currently known in the blocklace.
///
/// Scans backward from the highest known wave, returning the newest wave
/// whose unique leader block achieves weighted finality.
pub fn latest_weighted_final_leader<F>(
    blocklace: &Blocklace,
    wavelength: u64,
    bonds: &HashMap<NodeId, u64>,
    leader_selection: F,
) -> Option<BlockIdentity>
where
    F: Fn(u64) -> Option<NodeId> + Copy,
{
    if wavelength == 0 || blocklace.dom().is_empty() {
        return None;
    }

    let depths = compute_all_depths(blocklace);
    let max_round = depths.values().copied().max()?;
    let rounds = build_round_index(blocklace, &depths);
    let latest_wave = wave_of_round(max_round, wavelength)?;

    for wave in (0..=latest_wave).rev() {
        let Some(leader) =
            unique_leader_block_from_index(&rounds, wave, wavelength, &leader_selection)
        else {
            continue;
        };

        let Some(candidate_round) = depths.get(&leader.identity).copied() else {
            continue;
        };
        let Some(last_round) = last_round_of_wave(wave, wavelength) else {
            continue;
        };

        let witness_blocks = witness_blocks_from_index(&rounds, candidate_round, last_round);
        if weighted_super_ratifies(blocklace, &witness_blocks, &leader, bonds) {
            return Some(leader.identity);
        }
    }

    None
}

fn build_round_index(blocklace: &Blocklace, depths: &HashMap<BlockIdentity, u64>) -> RoundIndex {
    let mut rounds = HashMap::new();

    for (id, round) in depths {
        if let Some(block) = blocklace.get(id) {
            rounds.entry(*round).or_insert_with(Vec::new).push(block);
        }
    }

    rounds
}

fn unique_leader_block_from_index<F>(
    rounds: &RoundIndex,
    wave: u64,
    wavelength: u64,
    leader_selection: F,
) -> Option<Block>
where
    F: Fn(u64) -> Option<NodeId>,
{
    let leader = leader_selection(wave)?;
    let leader_round = wave.checked_mul(wavelength)?;
    let mut leaders = rounds
        .get(&leader_round)?
        .iter()
        .filter(|block| block.identity.creator == leader)
        .cloned();

    let first = leaders.next()?;
    if leaders.next().is_some() {
        return None;
    }

    Some(first)
}

fn witness_blocks_from_index(
    rounds: &RoundIndex,
    start_round: u64,
    end_round: u64,
) -> HashSet<Block> {
    let mut witness_blocks = HashSet::new();

    for round in start_round..=end_round {
        if let Some(blocks) = rounds.get(&round) {
            witness_blocks.extend(blocks.iter().cloned());
        }
    }

    witness_blocks
}
