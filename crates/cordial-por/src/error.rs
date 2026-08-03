use std::fmt;

/// Errors for the initial Proof-of-Reputation validation and batching stage.
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
        }
    }
}

impl std::error::Error for PorError {}
