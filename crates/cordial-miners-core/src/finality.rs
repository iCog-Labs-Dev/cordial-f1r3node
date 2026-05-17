//! Pure weighted finality predicates for Cordial Miners.
//!
//! This module implements the Definition 22 ratification operators over a
//! read-only blocklace view. It is intentionally independent from node,
//! storage, networking, cryptography, and runtime crates so later adapters can
//! feed it closed DAG data and active validator weights.

use std::collections::{HashMap, HashSet};
use std::hash::Hash;

/// Minimal block value needed by the Cordial Miners finality predicates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CordialBlock<VId, P, Id> {
    pub id: Id,
    pub creator: VId,
    pub round: u64,
    pub parents: Vec<Id>,
    pub payload: P,
}

/// Read-only view of a closed blocklace.
pub trait Blocklace<VId, P, Id> {
    /// Return a block by id, if it is present in the local view.
    fn block(&self, id: &Id) -> Option<&CordialBlock<VId, P, Id>>;

    /// Return the causal closure observed by `id`, including `id` itself.
    fn closure_ids(&self, id: &Id) -> Vec<Id>;
}

/// Active validator weights for the decision context being evaluated.
pub trait ValidatorSet<VId> {
    /// Return the active weight of a validator. Unknown validators should be 0.
    fn weight(&self, validator: &VId) -> u128;

    /// Return the total active validator weight for this decision context.
    fn total_weight(&self) -> u128;
}

/// Invalid threshold configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThresholdError {
    /// The denominator cannot be zero.
    ZeroDenominator,
    /// Thresholds must be strict proper fractions for finality support.
    NonStrictProperFraction,
}

/// Strict rational threshold used for weighted approval support.
///
/// Construct with [`ApprovalThreshold::try_new`] so invalid consensus
/// configuration is rejected before any finality query runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ApprovalThreshold {
    numerator: u128,
    denominator: u128,
}

impl ApprovalThreshold {
    pub fn try_new(numerator: u128, denominator: u128) -> Result<Self, ThresholdError> {
        if denominator == 0 {
            return Err(ThresholdError::ZeroDenominator);
        }

        if numerator >= denominator {
            return Err(ThresholdError::NonStrictProperFraction);
        }

        Ok(Self {
            numerator,
            denominator,
        })
    }

    pub const fn strict_two_thirds() -> Self {
        Self {
            numerator: 2,
            denominator: 3,
        }
    }

    pub const fn numerator(self) -> u128 {
        self.numerator
    }

    pub const fn denominator(self) -> u128 {
        self.denominator
    }

    /// Return true when `support_weight / total_weight` is strictly greater
    /// than this threshold.
    pub fn is_met(self, support_weight: u128, total_weight: u128) -> bool {
        if total_weight == 0 {
            return false;
        }

        let left = wide_mul(support_weight, self.denominator);
        let right = wide_mul(total_weight, self.numerator);
        left > right
    }
}

/// Per-query memoization for approval and ratification checks.
///
/// A memo is scoped to one immutable blocklace snapshot, one active validator
/// set, and one approval threshold. Reusing it across any of those contexts can
/// return stale ratification answers because the cache keys are block pairs.
#[derive(Debug, Clone)]
pub struct FinalityMemo<Id> {
    approve_cache: HashMap<(Id, Id), bool>,
    ratify_cache: HashMap<(Id, Id), bool>,
}

impl<Id> FinalityMemo<Id> {
    pub fn new() -> Self {
        Self {
            approve_cache: HashMap::new(),
            ratify_cache: HashMap::new(),
        }
    }

    pub fn approve_cache_len(&self) -> usize {
        self.approve_cache.len()
    }

    pub fn ratify_cache_len(&self) -> usize {
        self.ratify_cache.len()
    }
}

impl<Id> Default for FinalityMemo<Id> {
    fn default() -> Self {
        Self::new()
    }
}

/// Return true when `approver` approves `candidate`.
///
/// The predicate follows the ratification issue's base dependency:
/// the candidate must be in the approver's closure, and that closure must not
/// contain a different block from the same candidate creator at the same round.
pub fn approve<VId, P, Id, L>(
    blocklace: &L,
    approver: &CordialBlock<VId, P, Id>,
    candidate: &CordialBlock<VId, P, Id>,
) -> bool
where
    VId: Eq,
    Id: Clone + Eq + Hash,
    L: Blocklace<VId, P, Id>,
{
    compute_approval(blocklace, approver, candidate)
}

/// Memoized form of [`approve`].
pub fn approve_with_memo<VId, P, Id, L>(
    blocklace: &L,
    approver: &CordialBlock<VId, P, Id>,
    candidate: &CordialBlock<VId, P, Id>,
    memo: &mut FinalityMemo<Id>,
) -> bool
where
    VId: Eq,
    Id: Clone + Eq + Hash,
    L: Blocklace<VId, P, Id>,
{
    let key = (approver.id.clone(), candidate.id.clone());
    if let Some(result) = memo.approve_cache.get(&key) {
        return *result;
    }

    let result = compute_approval(blocklace, approver, candidate);
    memo.approve_cache.insert(key, result);
    result
}

/// Return true when a weighted strict supermajority of validators in
/// `closure(witness)` approve `candidate`.
pub fn ratify<VId, P, Id, L, VS>(
    blocklace: &L,
    witness: &CordialBlock<VId, P, Id>,
    candidate: &CordialBlock<VId, P, Id>,
    validators: &VS,
    threshold: ApprovalThreshold,
) -> bool
where
    VId: Clone + Eq + Hash,
    Id: Clone + Eq + Hash,
    L: Blocklace<VId, P, Id>,
    VS: ValidatorSet<VId>,
{
    let mut memo = FinalityMemo::new();
    ratify_with_memo(
        blocklace, witness, candidate, validators, threshold, &mut memo,
    )
}

/// Memoized form of [`ratify`].
pub fn ratify_with_memo<VId, P, Id, L, VS>(
    blocklace: &L,
    witness: &CordialBlock<VId, P, Id>,
    candidate: &CordialBlock<VId, P, Id>,
    validators: &VS,
    threshold: ApprovalThreshold,
    memo: &mut FinalityMemo<Id>,
) -> bool
where
    VId: Clone + Eq + Hash,
    Id: Clone + Eq + Hash,
    L: Blocklace<VId, P, Id>,
    VS: ValidatorSet<VId>,
{
    let key = (witness.id.clone(), candidate.id.clone());
    if let Some(result) = memo.ratify_cache.get(&key) {
        return *result;
    }

    let total_weight = validators.total_weight();
    if total_weight == 0
        || blocklace.block(&witness.id).is_none()
        || blocklace.block(&candidate.id).is_none()
    {
        memo.ratify_cache.insert(key, false);
        return false;
    }

    let mut supporters = HashSet::new();
    let mut support_weight = 0u128;

    for block_id in blocklace.closure_ids(&witness.id) {
        let Some(block) = blocklace.block(&block_id) else {
            continue;
        };

        if approve_with_memo(blocklace, block, candidate, memo)
            && supporters.insert(block.creator.clone())
        {
            let Some(next_weight) = support_weight.checked_add(validators.weight(&block.creator))
            else {
                memo.ratify_cache.insert(key, false);
                return false;
            };
            support_weight = next_weight;

            if threshold.is_met(support_weight, total_weight) {
                memo.ratify_cache.insert(key, true);
                return true;
            }
        }
    }

    memo.ratify_cache.insert(key, false);
    false
}

/// Return true when a weighted strict supermajority of validators represented
/// by `witness_ids` ratify `candidate`.
///
/// `witness_ids` must be selected by the caller for the relevant Cordial
/// Miners wave or round. This function does not select waves or leaders.
pub fn super_ratify<VId, P, Id, L, VS>(
    blocklace: &L,
    witness_ids: &[Id],
    candidate: &CordialBlock<VId, P, Id>,
    validators: &VS,
    threshold: ApprovalThreshold,
) -> bool
where
    VId: Clone + Eq + Hash,
    Id: Clone + Eq + Hash,
    L: Blocklace<VId, P, Id>,
    VS: ValidatorSet<VId>,
{
    let mut memo = FinalityMemo::new();
    super_ratify_with_memo(
        blocklace,
        witness_ids,
        candidate,
        validators,
        threshold,
        &mut memo,
    )
}

/// Memoized form of [`super_ratify`].
///
/// `witness_ids` must be selected by the caller for the relevant Cordial
/// Miners wave or round. This function does not select waves or leaders.
pub fn super_ratify_with_memo<VId, P, Id, L, VS>(
    blocklace: &L,
    witness_ids: &[Id],
    candidate: &CordialBlock<VId, P, Id>,
    validators: &VS,
    threshold: ApprovalThreshold,
    memo: &mut FinalityMemo<Id>,
) -> bool
where
    VId: Clone + Eq + Hash,
    Id: Clone + Eq + Hash,
    L: Blocklace<VId, P, Id>,
    VS: ValidatorSet<VId>,
{
    let total_weight = validators.total_weight();
    if total_weight == 0 || blocklace.block(&candidate.id).is_none() {
        return false;
    }

    let mut ratifying_validators = HashSet::new();
    let mut support_weight = 0u128;

    for witness_id in witness_ids {
        let Some(witness) = blocklace.block(witness_id) else {
            continue;
        };

        if ratify_with_memo(blocklace, witness, candidate, validators, threshold, memo)
            && ratifying_validators.insert(witness.creator.clone())
        {
            let Some(next_weight) = support_weight.checked_add(validators.weight(&witness.creator))
            else {
                return false;
            };
            support_weight = next_weight;

            if threshold.is_met(support_weight, total_weight) {
                return true;
            }
        }
    }

    false
}

fn compute_approval<VId, P, Id, L>(
    blocklace: &L,
    approver: &CordialBlock<VId, P, Id>,
    candidate: &CordialBlock<VId, P, Id>,
) -> bool
where
    VId: Eq,
    Id: Clone + Eq + Hash,
    L: Blocklace<VId, P, Id>,
{
    if blocklace.block(&approver.id).is_none() || blocklace.block(&candidate.id).is_none() {
        return false;
    }

    let mut observes_candidate = false;

    for block_id in blocklace.closure_ids(&approver.id) {
        if block_id == candidate.id {
            observes_candidate = true;
        }

        let Some(block) = blocklace.block(&block_id) else {
            continue;
        };

        if block.id != candidate.id
            && block.creator == candidate.creator
            && block.round == candidate.round
        {
            return false;
        }
    }

    observes_candidate
}

fn wide_mul(lhs: u128, rhs: u128) -> (u128, u128) {
    const MASK: u128 = u64::MAX as u128;

    let lhs_low = lhs & MASK;
    let lhs_high = lhs >> 64;
    let rhs_low = rhs & MASK;
    let rhs_high = rhs >> 64;

    let low_low = lhs_low * rhs_low;
    let low_high = lhs_low * rhs_high;
    let high_low = lhs_high * rhs_low;
    let high_high = lhs_high * rhs_high;

    let carry = (low_low >> 64) + (low_high & MASK) + (high_low & MASK);
    let low = (low_low & MASK) | ((carry & MASK) << 64);
    let high = high_high + (low_high >> 64) + (high_low >> 64) + (carry >> 64);

    (high, low)
}

#[cfg(test)]
mod tests;
