use std::fmt;

/// Errors for Proof-of-Reputation validation and calculation stages.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PorError {
    InvalidConfiguration(String),
    InvalidRatingRound,
    SelfRating,
    RatingBelowMinimum,
    RatingAboveMaximum,
    MissingRatingSignature,
    DuplicateRating,
    DuplicateMatrixEntry,
    InvalidNormalizationScale,
    NormalizationOverflow,
    InvalidLiquidRankScale,
    MissingRaterReputation,
    DuplicateReputationEntry,
    UnsortedReputationVector,
    LiquidRankOverflow,
}

impl fmt::Display for PorError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidConfiguration(message) => {
                write!(f, "invalid Proof-of-Reputation configuration: {message}")
            }
            Self::InvalidRatingRound => {
                write!(f, "rating round does not match the target batch round")
            }
            Self::SelfRating => write!(f, "rating cannot be self-issued"),
            Self::RatingBelowMinimum => write!(f, "rating score is below the configured minimum"),
            Self::RatingAboveMaximum => write!(f, "rating score exceeds the configured maximum"),
            Self::MissingRatingSignature => write!(f, "rating signature is empty"),
            Self::DuplicateRating => write!(
                f,
                "duplicate rating for the same round, rater, and recipient"
            ),
            Self::DuplicateMatrixEntry => write!(
                f,
                "duplicate matrix entry for the same round, rater, and recipient"
            ),
            Self::InvalidNormalizationScale => {
                write!(f, "normalization scale must be greater than zero")
            }
            Self::NormalizationOverflow => write!(f, "normalization arithmetic overflowed"),
            Self::InvalidLiquidRankScale => {
                write!(f, "liquid-rank scale must be greater than zero")
            }
            Self::MissingRaterReputation => {
                write!(f, "previous reputation vector is missing a rater")
            }
            Self::DuplicateReputationEntry => {
                write!(f, "duplicate reputation entry for the same node")
            }
            Self::UnsortedReputationVector => {
                write!(f, "reputation vector entries must be sorted by node id")
            }
            Self::LiquidRankOverflow => write!(f, "liquid-rank arithmetic overflowed"),
        }
    }
}

impl std::error::Error for PorError {}
