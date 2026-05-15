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

/// Strict rational threshold used for weighted approval support.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ApprovalThreshold {
    pub numerator: u128,
    pub denominator: u128,
}

impl ApprovalThreshold {
    pub const fn new(numerator: u128, denominator: u128) -> Self {
        Self {
            numerator,
            denominator,
        }
    }

    pub const fn strict_two_thirds() -> Self {
        Self::new(2, 3)
    }

    /// Return true when `support_weight / total_weight` is strictly greater
    /// than this threshold.
    pub fn is_met(self, support_weight: u128, total_weight: u128) -> bool {
        if self.denominator == 0 || total_weight == 0 {
            return false;
        }

        let left = wide_mul(support_weight, self.denominator);
        let right = wide_mul(total_weight, self.numerator);
        left > right
    }
}

/// Per-query memoization for approval and ratification checks.
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
    approve_uncached(blocklace, approver, candidate)
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

    let result = approve_uncached(blocklace, approver, candidate);
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
    if total_weight == 0 || threshold.denominator == 0 {
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
            support_weight = support_weight.saturating_add(validators.weight(&block.creator));

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
    if total_weight == 0 || threshold.denominator == 0 {
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
            support_weight = support_weight.saturating_add(validators.weight(&witness.creator));

            if threshold.is_met(support_weight, total_weight) {
                return true;
            }
        }
    }

    false
}

fn approve_uncached<VId, P, Id, L>(
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
mod tests {
    use super::*;
    use std::collections::{BTreeSet, HashMap, VecDeque};

    type TestBlock = CordialBlock<&'static str, (), &'static str>;

    #[derive(Default)]
    struct MemoryBlocklace {
        blocks: HashMap<&'static str, TestBlock>,
    }

    impl MemoryBlocklace {
        fn insert(&mut self, block: TestBlock) {
            self.blocks.insert(block.id, block);
        }

        fn block_ref(&self, id: &'static str) -> &TestBlock {
            self.blocks.get(id).expect("test block must exist")
        }
    }

    impl Blocklace<&'static str, (), &'static str> for MemoryBlocklace {
        fn block(&self, id: &&'static str) -> Option<&TestBlock> {
            self.blocks.get(id)
        }

        fn closure_ids(&self, id: &&'static str) -> Vec<&'static str> {
            let mut visited = BTreeSet::new();
            let mut queue = VecDeque::from([*id]);

            while let Some(current_id) = queue.pop_front() {
                if !visited.insert(current_id) {
                    continue;
                }

                if let Some(block) = self.blocks.get(current_id) {
                    for parent in &block.parents {
                        queue.push_back(*parent);
                    }
                }
            }

            visited.into_iter().collect()
        }
    }

    struct StaticValidators {
        weights: HashMap<&'static str, u128>,
        total: u128,
    }

    impl StaticValidators {
        fn weighted_abcd() -> Self {
            Self {
                weights: HashMap::from([("A", 4), ("B", 3), ("C", 2), ("D", 1), ("E", 0)]),
                total: 10,
            }
        }
    }

    impl ValidatorSet<&'static str> for StaticValidators {
        fn weight(&self, validator: &&'static str) -> u128 {
            self.weights.get(validator).copied().unwrap_or(0)
        }

        fn total_weight(&self) -> u128 {
            self.total
        }
    }

    fn block(
        id: &'static str,
        creator: &'static str,
        round: u64,
        parents: Vec<&'static str>,
    ) -> TestBlock {
        CordialBlock {
            id,
            creator,
            round,
            parents,
            payload: (),
        }
    }

    fn threshold() -> ApprovalThreshold {
        ApprovalThreshold::strict_two_thirds()
    }

    #[test]
    fn ratify_succeeds_with_weighted_supermajority_approvals() {
        let validators = StaticValidators::weighted_abcd();
        let mut blocklace = MemoryBlocklace::default();

        blocklace.insert(block("x", "A", 0, vec![]));
        blocklace.insert(block("w_b", "B", 1, vec!["x"]));

        assert!(ratify(
            &blocklace,
            blocklace.block_ref("w_b"),
            blocklace.block_ref("x"),
            &validators,
            threshold(),
        ));
    }

    #[test]
    fn ratify_fails_below_weighted_supermajority() {
        let validators = StaticValidators::weighted_abcd();
        let mut blocklace = MemoryBlocklace::default();

        blocklace.insert(block("x", "A", 0, vec![]));
        blocklace.insert(block("w_c", "C", 1, vec!["x"]));

        assert!(!ratify(
            &blocklace,
            blocklace.block_ref("w_c"),
            blocklace.block_ref("x"),
            &validators,
            threshold(),
        ));
    }

    #[test]
    fn super_ratify_succeeds_with_weighted_supermajority_ratifiers() {
        let validators = StaticValidators::weighted_abcd();
        let mut blocklace = MemoryBlocklace::default();

        blocklace.insert(block("x", "A", 0, vec![]));
        blocklace.insert(block("approver_b", "B", 1, vec!["x"]));
        blocklace.insert(block("ratifier_a", "A", 2, vec!["approver_b"]));
        blocklace.insert(block("ratifier_b", "B", 2, vec!["approver_b"]));

        assert!(super_ratify(
            &blocklace,
            &["ratifier_a", "ratifier_b"],
            blocklace.block_ref("x"),
            &validators,
            threshold(),
        ));
    }

    #[test]
    fn super_ratify_fails_below_weighted_supermajority() {
        let validators = StaticValidators::weighted_abcd();
        let mut blocklace = MemoryBlocklace::default();

        blocklace.insert(block("x", "A", 0, vec![]));
        blocklace.insert(block("approver_b", "B", 1, vec!["x"]));
        blocklace.insert(block("ratifier_b", "B", 2, vec!["approver_b"]));
        blocklace.insert(block("ratifier_c", "C", 2, vec!["approver_b"]));
        blocklace.insert(block("ratifier_d", "D", 2, vec!["approver_b"]));

        assert!(!super_ratify(
            &blocklace,
            &["ratifier_b", "ratifier_c", "ratifier_d"],
            blocklace.block_ref("x"),
            &validators,
            threshold(),
        ));
    }

    #[test]
    fn same_validator_is_counted_once_for_approval_and_ratification_support() {
        let validators = StaticValidators::weighted_abcd();
        let mut blocklace = MemoryBlocklace::default();

        blocklace.insert(block("x", "A", 0, vec![]));
        blocklace.insert(block("a1", "A", 1, vec!["x"]));
        blocklace.insert(block("a2", "A", 2, vec!["a1"]));
        blocklace.insert(block("approver_b", "B", 1, vec!["x"]));
        blocklace.insert(block("ratifier_a1", "A", 2, vec!["approver_b"]));
        blocklace.insert(block("ratifier_a2", "A", 3, vec!["ratifier_a1"]));

        assert!(!ratify(
            &blocklace,
            blocklace.block_ref("a2"),
            blocklace.block_ref("x"),
            &validators,
            threshold(),
        ));

        assert!(!super_ratify(
            &blocklace,
            &["ratifier_a1", "ratifier_a2"],
            blocklace.block_ref("x"),
            &validators,
            threshold(),
        ));
    }

    #[test]
    fn unknown_and_zero_weight_validators_do_not_contribute() {
        let validators = StaticValidators::weighted_abcd();
        let mut blocklace = MemoryBlocklace::default();

        blocklace.insert(block("x", "A", 0, vec![]));
        blocklace.insert(block("unknown_z", "Z", 1, vec!["x"]));
        blocklace.insert(block("zero_e", "E", 2, vec!["unknown_z"]));

        assert!(!ratify(
            &blocklace,
            blocklace.block_ref("zero_e"),
            blocklace.block_ref("x"),
            &validators,
            threshold(),
        ));
    }

    #[test]
    fn equivocating_candidates_cannot_both_super_ratify_same_witness_set() {
        let validators = StaticValidators::weighted_abcd();
        let mut blocklace = MemoryBlocklace::default();

        blocklace.insert(block("x", "A", 0, vec![]));
        blocklace.insert(block("x_prime", "A", 0, vec![]));
        blocklace.insert(block("approver_b", "B", 1, vec!["x"]));
        blocklace.insert(block("ratifier_a", "A", 2, vec!["approver_b"]));
        blocklace.insert(block("ratifier_b", "B", 2, vec!["approver_b"]));

        let witnesses = ["ratifier_a", "ratifier_b"];
        let x_super_ratifies = super_ratify(
            &blocklace,
            &witnesses,
            blocklace.block_ref("x"),
            &validators,
            threshold(),
        );
        let x_prime_super_ratifies = super_ratify(
            &blocklace,
            &witnesses,
            blocklace.block_ref("x_prime"),
            &validators,
            threshold(),
        );

        assert!(x_super_ratifies);
        assert!(!x_prime_super_ratifies);
        assert!(!(x_super_ratifies && x_prime_super_ratifies));
    }

    #[test]
    fn memoization_reuses_approval_and_ratification_pairs() {
        let validators = StaticValidators::weighted_abcd();
        let mut blocklace = MemoryBlocklace::default();

        blocklace.insert(block("x", "A", 0, vec![]));
        blocklace.insert(block("approver_b", "B", 1, vec!["x"]));
        blocklace.insert(block("ratifier_a", "A", 2, vec!["approver_b"]));
        blocklace.insert(block("ratifier_b", "B", 2, vec!["approver_b"]));

        let mut memo = FinalityMemo::new();
        let witnesses = ["ratifier_a", "ratifier_b"];

        let first = super_ratify_with_memo(
            &blocklace,
            &witnesses,
            blocklace.block_ref("x"),
            &validators,
            threshold(),
            &mut memo,
        );
        let approve_cache_len = memo.approve_cache_len();
        let ratify_cache_len = memo.ratify_cache_len();

        let second = super_ratify_with_memo(
            &blocklace,
            &witnesses,
            blocklace.block_ref("x"),
            &validators,
            threshold(),
            &mut memo,
        );

        assert_eq!(first, second);
        assert_eq!(memo.approve_cache_len(), approve_cache_len);
        assert_eq!(memo.ratify_cache_len(), ratify_cache_len);
    }

    #[test]
    fn zero_total_weight_never_reaches_threshold() {
        let validators = StaticValidators {
            weights: HashMap::from([("A", 10)]),
            total: 0,
        };
        let mut blocklace = MemoryBlocklace::default();

        blocklace.insert(block("x", "A", 0, vec![]));

        assert!(!ratify(
            &blocklace,
            blocklace.block_ref("x"),
            blocklace.block_ref("x"),
            &validators,
            threshold(),
        ));
    }

    #[test]
    fn threshold_uses_strict_integer_comparison() {
        let strict_two_thirds = ApprovalThreshold::strict_two_thirds();

        assert!(strict_two_thirds.is_met(7, 10));
        assert!(!strict_two_thirds.is_met(6, 10));
        assert!(!ApprovalThreshold::new(2, 0).is_met(10, 10));
    }
}
