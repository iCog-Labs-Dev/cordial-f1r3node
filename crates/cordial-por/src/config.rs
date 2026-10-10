use crate::types::{RatingScore, ReputationWeight};

/// How the transition treats a node that appears in only one of the two vectors
/// it blends.
///
/// A round is sparse when a node receives no ratings: Liquid Rank emits no
/// contribution entry for it. Naming the fallback keeps reputation from being
/// carried forward silently, which is what the original strict rejection
/// guarded against.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MissingEntryPolicy {
    /// Reject the round unless both vectors cover the same node set.
    Reject,

    /// Treat an unrated node's contribution as its previous reputation and take
    /// the finalized value from previous reputation, so it is unchanged.
    ///
    /// Absence of ratings is not evidence of inactivity: a node can be online
    /// and simply not interacted with. Punishing that belongs to the inactivity
    /// penalty stage, which knows the missed-round count. The clamp is skipped
    /// because it is not idempotent: applying it to an already-finalized value
    /// would decay reputation every sparse round. The previous value is copied
    /// rather than trusting the blended entry, so a hand-built blend cannot
    /// preserve an arbitrary unclamped value.
    #[default]
    CarryForward,

    /// Treat an unrated node's contribution as `initial_reputation`, drifting
    /// unrated nodes toward the configured baseline.
    Neutral,
}

/// Configuration parameters for PoR calculations and transitions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PorConfig {
    /// Fixed point scale.
    pub scale: ReputationWeight,

    /// Initial reputation.
    pub initial_reputation: ReputationWeight,

    /// Fixed-point alpha used to blend Liquid-Rank contribution with prior
    /// reputation.
    pub liquid_rank_alpha: ReputationWeight,

    /// Minimum accepted rating.
    pub minimum_rating: RatingScore,

    /// Maximum accepted rating.
    pub maximum_rating: RatingScore,

    /// Fallback applied when the contribution and previous reputation vectors
    /// cover different node sets.
    pub missing_entry_policy: MissingEntryPolicy,

    /// Fraction of total active reputation weight (in fixed-point units out of
    /// `scale`) above which a correlated equivocation event escalates from the
    /// base penalty to a full 100% slash.
    ///
    /// Default: `300_000_000` (30% of `DEFAULT_SCALE = 1_000_000_000`).
    pub correlation_threshold: ReputationWeight,

    /// Fixed-point penalty ratio applied to an equivocating validator when the
    /// correlated equivocation ratio is at or below `correlation_threshold`.
    ///
    /// Expressed in the same fixed-point units as `scale`.
    ///
    /// Default: `250_000_000` (25% of `DEFAULT_SCALE`).
    pub base_slash_penalty: ReputationWeight,

    /// Fixed-point decay factor applied to an inactive node's reputation each
    /// round it misses. The node's reputation is multiplied by
    /// `(scale - inactivity_decay_gamma) / scale`.
    ///
    /// A value of `0` disables inactivity decay.
    ///
    /// Default: `10_000_000` (1% of `DEFAULT_SCALE`).
    pub inactivity_decay_gamma: ReputationWeight,
}

/// Numerator of the default liquid-rank alpha ratio (3/5 = 60 %).
pub const DEFAULT_LIQUID_RANK_ALPHA_NUMERATOR: ReputationWeight = 3;

/// Denominator of the default liquid-rank alpha ratio (3/5 = 60 %).
pub const DEFAULT_LIQUID_RANK_ALPHA_DENOMINATOR: ReputationWeight = 5;

impl PorConfig {
    pub const DEFAULT_SCALE: ReputationWeight = 1_000_000_000;

    pub const DEFAULT_INITIAL_REPUTATION: ReputationWeight = 200_000_000;

    /// Default: 30% of `DEFAULT_SCALE`.
    pub const DEFAULT_CORRELATION_THRESHOLD: ReputationWeight = 300_000_000;

    /// Default: 25% of `DEFAULT_SCALE`.
    pub const DEFAULT_BASE_SLASH_PENALTY: ReputationWeight = 250_000_000;

    /// Default: 1% of `DEFAULT_SCALE` decay per missed round.
    pub const DEFAULT_INACTIVITY_DECAY_GAMMA: ReputationWeight = 10_000_000;

    /// Validate every protocol parameter before calculating or committing a round.
    pub fn validate(&self) -> Result<(), crate::PorError> {
        if self.scale == 0 {
            return Err(crate::PorError::InvalidConfiguration(
                "scale must be positive".into(),
            ));
        }
        if self.liquid_rank_alpha > self.scale || self.initial_reputation > self.scale {
            return Err(crate::PorError::InvalidConfiguration(
                "alpha and initial reputation must not exceed scale".into(),
            ));
        }
        if self.minimum_rating > self.maximum_rating {
            return Err(crate::PorError::InvalidConfiguration(
                "minimum rating must not exceed maximum rating".into(),
            ));
        }
        if self.correlation_threshold > self.scale
            || self.base_slash_penalty > self.scale
            || self.inactivity_decay_gamma > self.scale
        {
            return Err(crate::PorError::InvalidConfiguration(
                "penalty fractions must not exceed scale".into(),
            ));
        }
        Ok(())
    }

    pub fn new(scale: ReputationWeight, initial_reputation: ReputationWeight) -> Self {
        // Default alpha = 60 % of scale (3/5). This keeps liquid_rank_alpha <= scale
        // for every valid scale value, avoiding InvalidLiquidRankAlpha on first use.
        // When scale = DEFAULT_SCALE (1_000_000_000) the result is 600_000_000,
        // preserving existing behaviour exactly.
        let liquid_rank_alpha =
            ((u128::from(scale) * u128::from(DEFAULT_LIQUID_RANK_ALPHA_NUMERATOR))
                / u128::from(DEFAULT_LIQUID_RANK_ALPHA_DENOMINATOR)) as u64;

        Self {
            scale,
            initial_reputation,

            liquid_rank_alpha,

            minimum_rating: 0,

            maximum_rating: scale,

            missing_entry_policy: MissingEntryPolicy::default(),

            correlation_threshold: ((u128::from(scale)
                * u128::from(Self::DEFAULT_CORRELATION_THRESHOLD))
                / u128::from(Self::DEFAULT_SCALE)) as u64,
            base_slash_penalty: ((u128::from(scale) * u128::from(Self::DEFAULT_BASE_SLASH_PENALTY))
                / u128::from(Self::DEFAULT_SCALE)) as u64,
            inactivity_decay_gamma: ((u128::from(scale)
                * u128::from(Self::DEFAULT_INACTIVITY_DECAY_GAMMA))
                / u128::from(Self::DEFAULT_SCALE)) as u64,
        }
    }
}

impl Default for PorConfig {
    fn default() -> Self {
        Self::new(Self::DEFAULT_SCALE, Self::DEFAULT_INITIAL_REPUTATION)
    }
}
