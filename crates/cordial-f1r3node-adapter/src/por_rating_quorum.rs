//! Deterministic weighted closure policy for PoR rating rounds.
//!
//! Participation is credited only after a rater's complete deterministic
//! block-production batch has entered the evidence-backed collector. The
//! default policy requires strictly more than two thirds of active reputation
//! weight. It contains no wall-clock behavior; an external finalized cutoff
//! may still choose the coordinator's explicit close path.

use std::fmt;

use cordial_miners_core::NodeId;

use crate::por_rating_collector::{BlockProductionRatingCollector, PorRatingCollectorError};

/// Default strict quorum ratio: completed weight must be greater than 2/3.
pub const DEFAULT_RATING_QUORUM_NUMERATOR: u64 = 2;
pub const DEFAULT_RATING_QUORUM_DENOMINATOR: u64 = 3;

/// Failures while constructing or evaluating a rating-round closure policy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PorRatingQuorumError {
    InvalidThreshold { numerator: u64, denominator: u64 },
    ZeroActiveWeight,
    WeightOverflow,
    Collector(PorRatingCollectorError),
}

impl fmt::Display for PorRatingQuorumError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidThreshold {
                numerator,
                denominator,
            } => write!(
                f,
                "PoR rating quorum must satisfy 0 < numerator < denominator, got {numerator}/{denominator}"
            ),
            Self::ZeroActiveWeight => {
                write!(
                    f,
                    "PoR rating quorum has zero total active reputation weight"
                )
            }
            Self::WeightOverflow => write!(f, "PoR rating quorum weight arithmetic overflowed"),
            Self::Collector(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for PorRatingQuorumError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Collector(error) => Some(error),
            _ => None,
        }
    }
}

impl From<PorRatingCollectorError> for PorRatingQuorumError {
    fn from(error: PorRatingCollectorError) -> Self {
        Self::Collector(error)
    }
}

/// Deterministic snapshot of weighted rating-round participation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PorRatingQuorumProgress {
    pub eligible_raters: usize,
    pub completed_raters: Vec<NodeId>,
    pub total_active_weight: u128,
    pub completed_weight: u128,
    pub required_weight: u128,
}

impl PorRatingQuorumProgress {
    pub fn is_reached(&self) -> bool {
        self.completed_weight >= self.required_weight
    }
}

/// A strict rational threshold over active reputation weight.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PorRatingRoundClosurePolicy {
    numerator: u64,
    denominator: u64,
}

impl PorRatingRoundClosurePolicy {
    pub fn new(numerator: u64, denominator: u64) -> Result<Self, PorRatingQuorumError> {
        if numerator == 0 || denominator == 0 || numerator >= denominator {
            return Err(PorRatingQuorumError::InvalidThreshold {
                numerator,
                denominator,
            });
        }

        Ok(Self {
            numerator,
            denominator,
        })
    }

    pub fn numerator(&self) -> u64 {
        self.numerator
    }

    pub fn denominator(&self) -> u64 {
        self.denominator
    }

    /// Evaluate complete-rater weight against the strict configured ratio.
    pub fn evaluate(
        &self,
        collector: &BlockProductionRatingCollector<'_>,
    ) -> Result<PorRatingQuorumProgress, PorRatingQuorumError> {
        let state = collector.reputation_state();
        let mut eligible_raters = 0usize;
        let mut completed_raters = Vec::new();
        let mut total_active_weight = 0u128;
        let mut completed_weight = 0u128;

        for entry in &state.reputation_list().entries {
            if entry.is_excluded || state.is_ejected(&entry.node_id) {
                continue;
            }

            eligible_raters = eligible_raters
                .checked_add(1)
                .ok_or(PorRatingQuorumError::WeightOverflow)?;
            total_active_weight = total_active_weight
                .checked_add(u128::from(entry.reputation))
                .ok_or(PorRatingQuorumError::WeightOverflow)?;

            if collector.is_rater_complete(&entry.node_id)? {
                completed_weight = completed_weight
                    .checked_add(u128::from(entry.reputation))
                    .ok_or(PorRatingQuorumError::WeightOverflow)?;
                completed_raters.push(entry.node_id.clone());
            }
        }

        if total_active_weight == 0 {
            return Err(PorRatingQuorumError::ZeroActiveWeight);
        }

        let required_weight =
            strict_required_weight(total_active_weight, self.numerator, self.denominator)?;

        Ok(PorRatingQuorumProgress {
            eligible_raters,
            completed_raters,
            total_active_weight,
            completed_weight,
            required_weight,
        })
    }
}

impl Default for PorRatingRoundClosurePolicy {
    fn default() -> Self {
        Self {
            numerator: DEFAULT_RATING_QUORUM_NUMERATOR,
            denominator: DEFAULT_RATING_QUORUM_DENOMINATOR,
        }
    }
}

fn strict_required_weight(
    total_weight: u128,
    numerator: u64,
    denominator: u64,
) -> Result<u128, PorRatingQuorumError> {
    let numerator = u128::from(numerator);
    let denominator = u128::from(denominator);
    let whole = (total_weight / denominator)
        .checked_mul(numerator)
        .ok_or(PorRatingQuorumError::WeightOverflow)?;
    let remainder = (total_weight % denominator)
        .checked_mul(numerator)
        .ok_or(PorRatingQuorumError::WeightOverflow)?
        / denominator;

    whole
        .checked_add(remainder)
        .and_then(|floor| floor.checked_add(1))
        .ok_or(PorRatingQuorumError::WeightOverflow)
}
