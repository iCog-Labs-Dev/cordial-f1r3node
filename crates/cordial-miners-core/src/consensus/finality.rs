use std::collections::HashSet;

use crate::block::Block;
use crate::blocklace::Blocklace;
use crate::consensus::cordiality::super_ratifies;
use crate::consensus::round::{blocks_at_depth, depth};
use crate::consensus::wave::{last_round_of_wave, leader_blocks_of_wave, wave_of_round};
use crate::types::{BlockIdentity, NodeId};

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
