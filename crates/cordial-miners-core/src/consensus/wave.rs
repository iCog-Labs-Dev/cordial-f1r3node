//! Wave utilities for Cordial Miners.
//!
//! From the paper, the blocklace rounds are partitioned into fixed-length
//! waves, where the length is the wavelength. Each wave has one selected
//! leader, and any block by that leader in the first round of the wave is a
//! leader block.
//!
//! This module implements the wave structure from Definition A.10 and the
//! surrounding overview in Sections 3 and 4:
//! - waves are fixed-length groups of rounds
//! - the leader round of a wave is its first round
//! - an equivocating leader may have multiple leader blocks in that round

use std::collections::HashSet;
use std::ops::RangeInclusive;

use crate::block::Block;
use crate::blocklace::Blocklace;
use crate::consensus::round::blocks_at_depth;
use crate::types::NodeId;

/// Return the wave that contains `round`.
///
/// Rounds are zero-based and partitioned into contiguous chunks of size
/// `wavelength`.
pub fn wave_of_round(round: u64, wavelength: u64) -> Option<u64> {
    if wavelength == 0 {
        return None;
    }

    Some(round / wavelength)
}

/// Return the first round of `wave`.
pub fn first_round_of_wave(wave: u64, wavelength: u64) -> Option<u64> {
    if wavelength == 0 {
        return None;
    }

    wave.checked_mul(wavelength)
}

/// Return the last round of `wave`.
pub fn last_round_of_wave(wave: u64, wavelength: u64) -> Option<u64> {
    let first = first_round_of_wave(wave, wavelength)?;
    first.checked_add(wavelength - 1)
}

/// Return the inclusive round range of `wave`.
pub fn rounds_of_wave(wave: u64, wavelength: u64) -> Option<RangeInclusive<u64>> {
    let first = first_round_of_wave(wave, wavelength)?;
    let last = last_round_of_wave(wave, wavelength)?;
    Some(first..=last)
}

/// Return whether `round` is the first round of its wave.
pub fn is_first_round_of_wave(round: u64, wavelength: u64) -> bool {
    matches!(wave_of_round(round, wavelength), Some(wave) if first_round_of_wave(wave, wavelength) == Some(round))
}

/// Return whether `round` belongs to `wave`.
pub fn round_is_in_wave(round: u64, wave: u64, wavelength: u64) -> bool {
    wave_of_round(round, wavelength) == Some(wave)
}

/// The leader round of a wave is its first round.
pub fn leader_round_of_wave(wave: u64, wavelength: u64) -> Option<u64> {
    first_round_of_wave(wave, wavelength)
}

/// Return all leader blocks of `wave` in `blocklace`.
///
/// Per Definition A.10, a block is a leader block if:
/// - its creator is the miner selected as leader for the wave, and
/// - it appears in the first round of that wave.
///
/// If the selected leader equivocates in the leader round, there may be
/// multiple leader blocks.
pub fn leader_blocks_of_wave<F>(
    blocklace: &Blocklace,
    wave: u64,
    wavelength: u64,
    leader_selection: F,
) -> HashSet<Block>
where
    F: Fn(u64) -> Option<NodeId>,
{
    let Some(leader) = leader_selection(wave) else {
        return HashSet::new();
    };

    let Some(leader_round) = leader_round_of_wave(wave, wavelength) else {
        return HashSet::new();
    };

    blocks_at_depth(blocklace, leader_round)
        .into_iter()
        .filter(|block| block.identity.creator == leader)
        .collect()
}
