use std::fmt;

/// Errors for Proof-of-Reputation validation and calculation stages.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PorError {
    InvalidConfiguration(String),
    InvalidRatingRound,
    RatingRoundOverflow,
    InvalidInteractionRound,
    InvalidInteractionStateRound,
    SelfInteraction,
    MissingInteractionReference,
    UnknownInteractionRater,
    UnknownInteractionRecipient,
    EjectedInteractionRater,
    EjectedInteractionRecipient,
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
    InvalidTransitionScale,
    InvalidLiquidRankAlpha,
    MissingPreviousReputation,
    ReputationTransitionOverflow,
    MissingContributionEntry,
    InvalidTransitionRound,
    // Clamp-specific errors
    InvalidClampScale,
    ClampOverflow,
    // Reputation-block-specific errors
    UnsupportedReputationBlockVersion(u16),
    MissingReputationBlockShardId,
    ReputationBlockShardIdTooLong,
    InvalidReputationBlockSourceWave,
    InvalidReputationBlockRound,
    InvalidPreviousReputationBlockRound,
    PreviousReputationBlockShardMismatch,
    ReputationBlockShardMismatch,
    ReputationBlockSourceWaveMismatch,
    ReputationBlockPreviousHashMismatch,
    ReputationBlockConfigHashMismatch,
    ReputationBlockRatingsHashMismatch,
    ReputationBlockRootMismatch,
    ReputationExclusionMismatch,
    CommitmentLengthOverflow,
    // Durable-state snapshot errors
    UnsupportedReputationStateSnapshotVersion(u16),
    ReputationStateSnapshotTooLarge,
    MalformedReputationStateSnapshot,
    ReputationStateSnapshotChecksumMismatch,
    ReputationStateSnapshotRoundMismatch,
    ReputationStateSnapshotExclusionMismatch,
    ReputationStateSnapshotBlockRoundMismatch,
    ReputationStateSnapshotHasPendingRatings,
    // Audit-replay-specific errors
    MissingReputationBlockEntry,
    UnexpectedReputationBlockEntry,
    ReputationValueMismatch,
    // Key-ejection errors
    /// The requested node is not present in the current `ReputationState`.
    UnknownNode,
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
            Self::RatingRoundOverflow => {
                write!(f, "finalized wave cannot advance to a rating round")
            }
            Self::InvalidInteractionRound => {
                write!(f, "interaction round does not follow its finalized wave")
            }
            Self::InvalidInteractionStateRound => write!(
                f,
                "interaction round must immediately follow the current reputation state"
            ),
            Self::SelfInteraction => {
                write!(f, "interaction rater and recipient must be distinct")
            }
            Self::MissingInteractionReference => {
                write!(f, "interaction evidence reference is empty")
            }
            Self::UnknownInteractionRater => {
                write!(f, "interaction rater is not present in reputation state")
            }
            Self::UnknownInteractionRecipient => {
                write!(
                    f,
                    "interaction recipient is not present in reputation state"
                )
            }
            Self::EjectedInteractionRater => {
                write!(f, "ejected validator cannot submit interaction evidence")
            }
            Self::EjectedInteractionRecipient => {
                write!(
                    f,
                    "ejected validator cannot receive ordinary interaction ratings"
                )
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
            Self::InvalidTransitionScale => {
                write!(f, "reputation transition scale must be greater than zero")
            }
            Self::InvalidLiquidRankAlpha => {
                write!(f, "liquid-rank alpha must not exceed the fixed-point scale")
            }
            Self::MissingPreviousReputation => {
                write!(
                    f,
                    "previous reputation vector is missing a contribution node"
                )
            }
            Self::ReputationTransitionOverflow => {
                write!(f, "reputation transition arithmetic overflowed")
            }
            Self::MissingContributionEntry => {
                write!(
                    f,
                    "contribution vector is missing a previous reputation node"
                )
            }
            Self::InvalidTransitionRound => {
                write!(
                    f,
                    "contribution round must immediately follow the previous reputation round"
                )
            }
            Self::InvalidClampScale => write!(f, "clamp scale must be greater than zero"),
            Self::ClampOverflow => write!(f, "clamp arithmetic overflowed"),
            Self::UnsupportedReputationBlockVersion(version) => {
                write!(f, "unsupported reputation block version {version}")
            }
            Self::MissingReputationBlockShardId => {
                write!(f, "reputation block shard id is empty")
            }
            Self::ReputationBlockShardIdTooLong => {
                write!(f, "reputation block shard id exceeds the protocol limit")
            }
            Self::InvalidReputationBlockSourceWave => write!(
                f,
                "reputation block round does not immediately follow its finalized source wave"
            ),
            Self::InvalidReputationBlockRound => write!(
                f,
                "reputation block header round does not match the reputation list round"
            ),
            Self::InvalidPreviousReputationBlockRound => write!(
                f,
                "previous reputation block does not immediately precede the proposed block"
            ),
            Self::PreviousReputationBlockShardMismatch => {
                write!(f, "previous reputation block belongs to a different shard")
            }
            Self::ReputationBlockShardMismatch => {
                write!(
                    f,
                    "reputation block shard id does not match the audit context"
                )
            }
            Self::ReputationBlockSourceWaveMismatch => write!(
                f,
                "reputation block source wave does not match the audit context"
            ),
            Self::ReputationBlockPreviousHashMismatch => {
                write!(
                    f,
                    "reputation block does not extend the expected previous block"
                )
            }
            Self::ReputationBlockConfigHashMismatch => {
                write!(
                    f,
                    "reputation block configuration commitment does not match"
                )
            }
            Self::ReputationBlockRatingsHashMismatch => {
                write!(f, "reputation block rating-batch commitment does not match")
            }
            Self::ReputationBlockRootMismatch => {
                write!(f, "reputation block list commitment does not match")
            }
            Self::ReputationExclusionMismatch => {
                write!(f, "reputation block exclusion flag does not match replay")
            }
            Self::CommitmentLengthOverflow => {
                write!(f, "canonical reputation commitment input is too large")
            }
            Self::UnsupportedReputationStateSnapshotVersion(version) => {
                write!(f, "unsupported reputation state snapshot version {version}")
            }
            Self::ReputationStateSnapshotTooLarge => {
                write!(f, "reputation state snapshot exceeds the protocol limit")
            }
            Self::MalformedReputationStateSnapshot => {
                write!(f, "reputation state snapshot is malformed")
            }
            Self::ReputationStateSnapshotChecksumMismatch => {
                write!(f, "reputation state snapshot checksum does not match")
            }
            Self::ReputationStateSnapshotRoundMismatch => {
                write!(f, "reputation state snapshot rounds do not match")
            }
            Self::ReputationStateSnapshotExclusionMismatch => {
                write!(
                    f,
                    "reputation state snapshot exclusion registry is inconsistent"
                )
            }
            Self::ReputationStateSnapshotBlockRoundMismatch => {
                write!(
                    f,
                    "reputation state snapshot latest block is from another round"
                )
            }
            Self::ReputationStateSnapshotHasPendingRatings => {
                write!(
                    f,
                    "reputation state with pending ratings cannot be snapshotted"
                )
            }
            Self::MissingReputationBlockEntry => {
                write!(f, "reputation block is missing a replayed reputation entry")
            }
            Self::UnexpectedReputationBlockEntry => {
                write!(
                    f,
                    "reputation block contains an unexpected reputation entry"
                )
            }
            Self::ReputationValueMismatch => write!(
                f,
                "reputation block entry does not match the replayed reputation value"
            ),
            Self::UnknownNode => write!(f, "node is not present in the current reputation state"),
        }
    }
}

impl std::error::Error for PorError {}
