//! Bridge from Cordial Miners finality to Proof-of-Reputation rating rounds.
//!
//! This module does not decide finality. It consumes the final leader anchor
//! already exported by [`OrderedFinalizedOutput`], derives its wave from the
//! leader's blocklace depth, and opens the following PoR rating round.

use std::{collections::HashMap, fmt};

use cordial_miners_core::{
    Blocklace, NodeId,
    consensus::{
        OrderingError, depth, wave_of_round, weighted_final_leader_for_wave,
        weighted_tau_at_leader, xsort,
    },
    crypto::{Blake2b256Hasher, Hasher},
    types::BlockIdentity,
};
use cordial_por::{PorError, ReputationRound, rating_round_from_finalized_wave};

use crate::ordered_output::OrderedFinalizedOutput;

/// Number of canonical finalized blocks in one checkpoint-driven PoR window.
/// This value is part of the source-round protocol and must be versioned if changed.
pub const POR_FINALIZED_BLOCKS_PER_CHECKPOINT: u64 = 16;
const POR_FINALIZED_PREFIX_HASH_DOMAIN: &[u8] = b"cordial-por:finalized-prefix:v1";

/// A source checkpoint selected from a fixed position in Cordial's finalized
/// block prefix. Its identity is independent of when an observer polls.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FinalizedPrefixCheckpoint {
    pub por_round: ReputationRound,
    pub finalized_prefix_len: u64,
    /// Rolling commitment to every full block identity in the prefix.
    pub prefix_hash: [u8; 32],
    pub terminal_block: BlockIdentity,
}

/// A PoR rating round opened by a finalized Cordial Miners wave.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FinalizedRatingRound {
    pub finalized_wave: u64,
    pub rating_round: ReputationRound,
}

/// Errors while translating finalized output into a PoR rating round.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PorFinalityError {
    InvalidWavelength,
    UnknownFinalLeader,
    UnanchoredFinalizedPrefix,
    InvalidPrefixCheckpointRound,
    PrefixCheckpointMismatch,
    ConflictingFinalLeader { wave: u64 },
    MissingFinalLeaderForWave { wave: u64 },
    HistoricalPrefixMismatch { wave: u64 },
    Ordering(OrderingError),
    RatingRound(PorError),
}

/// Derive the wave of the latest Cordial final leader without opening a PoR round.
///
/// Observers can use this for reporting even when they do not have the signed
/// rating exchange required to advance reputation state.
pub fn finalized_wave_from_output(
    blocklace: &Blocklace,
    output: &OrderedFinalizedOutput,
) -> Result<Option<u64>, PorFinalityError> {
    let Some(final_leader) = output.anchor.as_ref() else {
        return Ok(None);
    };

    if output.wavelength == 0 {
        return Err(PorFinalityError::InvalidWavelength);
    }

    let leader_round =
        depth(blocklace, final_leader).ok_or(PorFinalityError::UnknownFinalLeader)?;
    wave_of_round(leader_round, output.wavelength)
        .map(Some)
        .ok_or(PorFinalityError::InvalidWavelength)
}

impl fmt::Display for PorFinalityError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidWavelength => write!(f, "finalized output wavelength must be non-zero"),
            Self::UnknownFinalLeader => {
                write!(f, "final leader anchor is not present in the blocklace")
            }
            Self::UnanchoredFinalizedPrefix => {
                write!(f, "non-empty finalized prefix has no final leader anchor")
            }
            Self::InvalidPrefixCheckpointRound => {
                write!(f, "invalid or overflowing PoR source checkpoint round")
            }
            Self::PrefixCheckpointMismatch => {
                write!(
                    f,
                    "PoR source checkpoint does not match the finalized prefix"
                )
            }
            Self::ConflictingFinalLeader { wave } => {
                write!(f, "conflicting final leader observed for wave {wave}")
            }
            Self::MissingFinalLeaderForWave { wave } => {
                write!(f, "no final leader can be replayed for wave {wave}")
            }
            Self::HistoricalPrefixMismatch { wave } => {
                write!(
                    f,
                    "finalized output for wave {wave} is not a prefix of the latest output"
                )
            }
            Self::Ordering(error) => write!(f, "cannot replay weighted ordering: {error:?}"),
            Self::RatingRound(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for PorFinalityError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::RatingRound(error) => Some(error),
            Self::Ordering(_) => None,
            _ => None,
        }
    }
}

/// Select the checkpoint for `por_round` from a canonical finalized prefix.
///
/// Round one uses blocks 1..=16, round two 17..=32, and so on. A poll that
/// receives several windows at once therefore selects the same checkpoints as
/// a poll that observed every intermediate finalized output. Genesis alone
/// does not open a round. The caller must still verify that successive Cordial
/// outputs preserve the previously published prefix.
pub fn finalized_prefix_checkpoint(
    output: &OrderedFinalizedOutput,
    por_round: ReputationRound,
) -> Result<Option<FinalizedPrefixCheckpoint>, PorFinalityError> {
    if por_round == 0 {
        return Err(PorFinalityError::InvalidPrefixCheckpointRound);
    }
    let end = por_round
        .checked_mul(POR_FINALIZED_BLOCKS_PER_CHECKPOINT)
        .ok_or(PorFinalityError::InvalidPrefixCheckpointRound)?;
    let index =
        usize::try_from(end - 1).map_err(|_| PorFinalityError::InvalidPrefixCheckpointRound)?;
    if output.anchor.is_none() {
        if output.blocks.is_empty() {
            return Ok(None);
        }
        return Err(PorFinalityError::UnanchoredFinalizedPrefix);
    }
    Ok(output
        .blocks
        .get(index)
        .cloned()
        .map(|terminal_block| FinalizedPrefixCheckpoint {
            por_round,
            finalized_prefix_len: end,
            prefix_hash: finalized_prefix_hash(&output.blocks[..=index]),
            terminal_block,
        }))
}

fn finalized_prefix_hash(blocks: &[BlockIdentity]) -> [u8; 32] {
    let mut hash = Blake2b256Hasher.hash(POR_FINALIZED_PREFIX_HASH_DOMAIN);
    for (index, block) in blocks.iter().enumerate() {
        let mut bytes = Vec::with_capacity(
            POR_FINALIZED_PREFIX_HASH_DOMAIN.len()
                + 32
                + 8
                + 32
                + 8
                + block.creator.0.len()
                + 8
                + block.signature.len(),
        );
        bytes.extend_from_slice(POR_FINALIZED_PREFIX_HASH_DOMAIN);
        bytes.extend_from_slice(&hash);
        bytes.extend_from_slice(&(index as u64).to_be_bytes());
        bytes.extend_from_slice(&block.content_hash);
        bytes.extend_from_slice(&(block.creator.0.len() as u64).to_be_bytes());
        bytes.extend_from_slice(&block.creator.0);
        bytes.extend_from_slice(&(block.signature.len() as u64).to_be_bytes());
        bytes.extend_from_slice(&block.signature);
        hash = Blake2b256Hasher.hash(&bytes);
    }
    hash
}

/// Confirm that a retained checkpoint still identifies the same canonical
/// finalized prefix after replay or a newer final leader.
pub fn validate_finalized_prefix_checkpoint(
    output: &OrderedFinalizedOutput,
    checkpoint: &FinalizedPrefixCheckpoint,
) -> Result<(), PorFinalityError> {
    let expected = finalized_prefix_checkpoint(output, checkpoint.por_round)?;
    if expected.as_ref() == Some(checkpoint) {
        Ok(())
    } else {
        Err(PorFinalityError::PrefixCheckpointMismatch)
    }
}

/// Replay the finalized output at `wave` from a later cumulative output.
///
/// The historical ordered blocks must be an exact prefix of the published
/// latest output. This rejects a leader that was final under the current
/// bonds but did not belong to the latest Cordial ordering chain.
pub fn finalized_output_for_wave(
    blocklace: &Blocklace,
    latest: &OrderedFinalizedOutput,
    bonds: &HashMap<NodeId, u64>,
    wave: u64,
) -> Result<Option<OrderedFinalizedOutput>, PorFinalityError> {
    let Some(latest_wave) = finalized_wave_from_output(blocklace, latest)? else {
        return Ok(None);
    };
    if wave > latest_wave {
        return Ok(None);
    }
    if wave == latest_wave {
        return Ok(Some(latest.clone()));
    }
    if bonds.is_empty() {
        return Err(PorFinalityError::MissingFinalLeaderForWave { wave });
    }

    let mut validators: Vec<NodeId> = bonds.keys().cloned().collect();
    validators.sort();
    let leader_selection = |selected_wave: u64| {
        let index = usize::try_from(selected_wave).ok()? % validators.len();
        Some(validators[index].clone())
    };
    let leader =
        weighted_final_leader_for_wave(blocklace, wave, latest.wavelength, bonds, leader_selection)
            .ok_or(PorFinalityError::MissingFinalLeaderForWave { wave })?;
    let blocks = if validators.len() == 1 && blocklace.checkpoint().is_none() {
        let observed = blocklace
            .observe(&leader)
            .into_iter()
            .filter_map(|id| blocklace.get(&id))
            .collect();
        xsort(&observed).map_err(PorFinalityError::Ordering)?
    } else {
        weighted_tau_at_leader(
            blocklace,
            &leader,
            latest.wavelength,
            bonds,
            leader_selection,
        )
        .map_err(PorFinalityError::Ordering)?
    };
    if !latest.blocks.starts_with(&blocks) || !blocks.contains(&leader) {
        return Err(PorFinalityError::HistoricalPrefixMismatch { wave });
    }

    Ok(Some(
        OrderedFinalizedOutput::new(
            blocks,
            Some(leader),
            latest.wavelength,
            latest.bond_count,
            latest.total_mirrored_blocks,
        )
        .with_timestamp(latest.computed_at_ns),
    ))
}

/// Process-local cursor over final leaders already used to open PoR rounds.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PorFinalityTracker {
    last_opened_wave: Option<u64>,
    last_final_leader: Option<BlockIdentity>,
}

impl PorFinalityTracker {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn last_opened_wave(&self) -> Option<u64> {
        self.last_opened_wave
    }

    /// Open the rating round following the output's finalized leader wave.
    ///
    /// Outputs without an anchor have no established finality and return
    /// `Ok(None)`. Repeated and older finalized outputs are idempotent. A
    /// different final leader for the most recently opened wave is rejected.
    pub fn observe_finalized_output(
        &mut self,
        blocklace: &Blocklace,
        output: &OrderedFinalizedOutput,
    ) -> Result<Option<FinalizedRatingRound>, PorFinalityError> {
        let Some(finalized_wave) = finalized_wave_from_output(blocklace, output)? else {
            return Ok(None);
        };
        let final_leader = output.anchor.as_ref().expect("wave requires an anchor");

        if let Some(last_opened_wave) = self.last_opened_wave {
            if finalized_wave < last_opened_wave {
                return Ok(None);
            }

            if finalized_wave == last_opened_wave {
                if self.last_final_leader.as_ref() == Some(final_leader) {
                    return Ok(None);
                }

                return Err(PorFinalityError::ConflictingFinalLeader {
                    wave: finalized_wave,
                });
            }
        }

        let rating_round = rating_round_from_finalized_wave(finalized_wave)
            .map_err(PorFinalityError::RatingRound)?;

        self.last_opened_wave = Some(finalized_wave);
        self.last_final_leader = Some(final_leader.clone());

        Ok(Some(FinalizedRatingRound {
            finalized_wave,
            rating_round,
        }))
    }
}
