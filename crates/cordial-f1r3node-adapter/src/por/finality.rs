//! Bridge from Cordial Miners finality to Proof-of-Reputation rating rounds.
//!
//! This module does not decide finality. It consumes the final leader anchor
//! already exported by [`OrderedFinalizedOutput`], derives its wave from the
//! leader's blocklace depth, and opens the following PoR rating round.

use std::fmt;

use cordial_miners_core::{
    Blocklace,
    consensus::{depth, wave_of_round},
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
    RatingRound(PorError),
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
            Self::RatingRound(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for PorFinalityError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::RatingRound(error) => Some(error),
            _ => None,
        }
    }
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
        let Some(final_leader) = output.anchor.as_ref() else {
            return Ok(None);
        };

        if output.wavelength == 0 {
            return Err(PorFinalityError::InvalidWavelength);
        }

        let leader_round =
            depth(blocklace, final_leader).ok_or(PorFinalityError::UnknownFinalLeader)?;
        let finalized_wave = wave_of_round(leader_round, output.wavelength)
            .ok_or(PorFinalityError::InvalidWavelength)?;

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
