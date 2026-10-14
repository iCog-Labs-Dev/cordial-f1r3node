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
    types::BlockIdentity,
};
use cordial_por::{PorError, ReputationRound, rating_round_from_finalized_wave};

use crate::ordered_output::OrderedFinalizedOutput;

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
