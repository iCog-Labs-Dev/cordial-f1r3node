//! Property test: PoR-exported weight is monotonic in supporting rater
//! agreement, end to end through the full pipeline (RatingRecord ->
//! RatingBatch -> RatingMatrix -> NormalizedRatingMatrix -> Liquid-Rank
//! contribution -> alpha blend -> clamp -> ReputationState ->
//! reputation_weights()).
//!
//! Property under test: for a fixed target validator and a fixed previous
//! reputation vector, if raters in `small` all unanimously rate the target
//! at the maximum score, and `large` is any superset of `small` whose extra
//! raters also unanimously rate the target at the maximum score, then the
//! target's exported PoR weight with `large` must be >= its weight with
//! `small`.
//!
//! This holds by construction, not just empirically:
//! - `compute_liquid_rank_contribution` sums normalized_score *
//!   rater_previous_reputation over raters (see `liquid_rank.rs`), so
//!   adding a rater with non-negative reputation only adds a non-negative
//!   term.
//! - `blend_reputation_transition` is affine increasing in the contribution
//!   for a fixed previous value (see `transition.rs`).
//! - `clamp_reputation_value` (`r * s / sqrt(s^2 + r^2)`) is monotonic
//!   non-decreasing in `r` for `r >= 0` (see `clamp.rs`).
//!
//! Because every rater in both sets agrees unanimously on the maximum
//! score, this sidesteps `normalize_recipient_group`'s per-recipient
//! relative normalization entirely (unanimous agreement always normalizes
//! to `scale`, regardless of group size), rather than depending on it.
//!
//! Failing cases print the exact previous-reputation vector and rater
//! subsets needed to reproduce, and are persisted under
//! `crates/cordial-por/proptest-regressions/prop_por_weight_monotonicity.txt`.

use std::collections::{HashMap, HashSet};

use cordial_miners_core::NodeId;
use cordial_por::{
    MissingEntryPolicy, PorConfig, RatingRecord, ReputationEntry, ReputationState,
    ReputationVector, replay_reputation_transition, reputation_weights,
};

use proptest::prelude::*;

const MAX_RATERS: u8 = 6;
const TARGET_ID: u8 = 1;

fn node(id: u8) -> NodeId {
    NodeId(vec![id])
}

fn config() -> PorConfig {
    PorConfig {
        scale: 100,
        initial_reputation: 50,
        liquid_rank_alpha: 60,
        minimum_rating: 0,
        maximum_rating: 100,
        missing_entry_policy: MissingEntryPolicy::CarryForward,
    }
}

/// Previous reputation for the target (node 1) plus every candidate rater
/// (nodes 2..=MAX_RATERS+1), each independently in [1, 1000].
fn previous_reputation_strategy() -> impl Strategy<Value = ReputationVector> {
    prop::collection::vec(1u64..=1000, (MAX_RATERS as usize) + 1).prop_map(|values| {
        let mut entries: Vec<ReputationEntry> = values
            .into_iter()
            .enumerate()
            .map(|(i, rep)| ReputationEntry::new(node(TARGET_ID + i as u8), rep))
            .collect();
        entries.sort_by(|left, right| left.node_id.cmp(&right.node_id));
        ReputationVector {
            round: 0,
            values: entries,
        }
    })
}

fn weight_of(previous: &ReputationVector, raters: &HashSet<NodeId>) -> u64 {
    let cfg = config();
    let target = node(TARGET_ID);

    let ratings: Vec<RatingRecord> = raters
        .iter()
        .map(|rater| {
            RatingRecord::new(
                1,
                rater.clone(),
                target.clone(),
                cfg.maximum_rating,
                vec![0xAB],
            )
        })
        .collect();

    let list = replay_reputation_transition(previous, &ratings, 1, &cfg)
        .expect("transition succeeds for a non-empty, valid rater set");

    let mut state = ReputationState::new(0);
    state
        .apply_reputation_vector(ReputationVector {
            round: list.round,
            values: list.entries,
        })
        .expect("canonical vector");

    let weights: HashMap<NodeId, u64> = reputation_weights(&state);
    *weights
        .get(&target)
        .expect("target was rated, so it must have a contribution entry")
}

proptest! {
    /// Adding more unanimous, maximum-score raters for a validator can
    /// never decrease that validator's exported PoR weight.
    #[test]
    fn por_weight_is_monotonic_in_supporting_raters(
        previous in previous_reputation_strategy(),
        support_mask in prop::collection::vec(any::<bool>(), 1..=MAX_RATERS as usize),
        extra_mask in prop::collection::vec(any::<bool>(), 1..=MAX_RATERS as usize),
    ) {
        let candidates: Vec<NodeId> = (0..MAX_RATERS).map(|i| node(TARGET_ID + 1 + i)).collect();

        let small: HashSet<NodeId> = candidates
            .iter()
            .zip(support_mask.iter().cycle())
            .filter(|(_, include)| **include)
            .map(|(id, _)| id.clone())
            .collect();

        prop_assume!(!small.is_empty());

        let mut large = small.clone();
        for (id, include_extra) in candidates.iter().zip(extra_mask.iter().cycle()) {
            if *include_extra {
                large.insert(id.clone());
            }
        }

        let weight_small = weight_of(&previous, &small);
        let weight_large = weight_of(&previous, &large);

        prop_assert!(
            weight_large >= weight_small,
            "growing raters from {:?} to {:?} decreased target's weight: {} -> {}",
            small,
            large,
            weight_small,
            weight_large
        );
    }
}
