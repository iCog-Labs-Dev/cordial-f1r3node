//! Finalized-wave cutoff policy for PoR rating-round closure.
//!
//! A cutoff advances only through Cordial finality, never wall-clock time. The
//! default waits for the wave immediately following the one that produced the
//! ratings, ensuring a stalled rating quorum cannot keep a round open forever.

use std::fmt;

use crate::por::finality::FinalizedRatingRound;

/// Default number of additional finalized waves allowed before cutoff.
pub const DEFAULT_RATING_CUTOFF_WAVE_LAG: u64 = 1;

/// Failures while constructing or evaluating a finalized-wave cutoff.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PorRatingCutoffError {
    ZeroWaveLag,
    RequiredWaveOverflow {
        finalized_wave: u64,
        wave_lag: u64,
    },
    CutoffNotReached {
        observed_finalized_wave: u64,
        required_finalized_wave: u64,
    },
}

impl fmt::Display for PorRatingCutoffError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ZeroWaveLag => write!(f, "PoR rating cutoff wave lag must be non-zero"),
            Self::RequiredWaveOverflow {
                finalized_wave,
                wave_lag,
            } => write!(
                f,
                "PoR rating cutoff overflows for finalized wave {finalized_wave} and lag {wave_lag}"
            ),
            Self::CutoffNotReached {
                observed_finalized_wave,
                required_finalized_wave,
            } => write!(
                f,
                "PoR rating cutoff requires finalized wave {required_finalized_wave}, but only wave {observed_finalized_wave} was observed"
            ),
        }
    }
}

impl std::error::Error for PorRatingCutoffError {}

/// Deterministic finalized-wave lag used as the rating quorum fallback.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PorRatingRoundCutoffPolicy {
    wave_lag: u64,
}

impl PorRatingRoundCutoffPolicy {
    pub fn new(wave_lag: u64) -> Result<Self, PorRatingCutoffError> {
        if wave_lag == 0 {
            return Err(PorRatingCutoffError::ZeroWaveLag);
        }

        Ok(Self { wave_lag })
    }

    pub fn wave_lag(&self) -> u64 {
        self.wave_lag
    }

    pub fn required_finalized_wave(
        &self,
        opened: FinalizedRatingRound,
    ) -> Result<u64, PorRatingCutoffError> {
        opened.finalized_wave.checked_add(self.wave_lag).ok_or(
            PorRatingCutoffError::RequiredWaveOverflow {
                finalized_wave: opened.finalized_wave,
                wave_lag: self.wave_lag,
            },
        )
    }

    pub fn ensure_reached(
        &self,
        opened: FinalizedRatingRound,
        observed_finalized_wave: u64,
    ) -> Result<u64, PorRatingCutoffError> {
        let required_finalized_wave = self.required_finalized_wave(opened)?;
        if observed_finalized_wave < required_finalized_wave {
            return Err(PorRatingCutoffError::CutoffNotReached {
                observed_finalized_wave,
                required_finalized_wave,
            });
        }

        Ok(required_finalized_wave)
    }
}

impl Default for PorRatingRoundCutoffPolicy {
    fn default() -> Self {
        Self {
            wave_lag: DEFAULT_RATING_CUTOFF_WAVE_LAG,
        }
    }
}
