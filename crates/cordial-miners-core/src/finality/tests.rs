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

    blocklace.insert(block("x", "L", 0, vec![]));
    blocklace.insert(block("approver_a", "A", 1, vec!["x"]));
    blocklace.insert(block("approver_b", "B", 1, vec!["x"]));
    blocklace.insert(block("witness_z", "Z", 2, vec!["approver_a", "approver_b"]));

    assert!(ratify(
        &blocklace,
        blocklace.block_ref("witness_z"),
        blocklace.block_ref("x"),
        &validators,
        threshold(),
    ));
}

#[test]
fn ratify_fails_below_weighted_supermajority() {
    let validators = StaticValidators::weighted_abcd();
    let mut blocklace = MemoryBlocklace::default();

    blocklace.insert(block("x", "L", 0, vec![]));
    blocklace.insert(block("approver_a", "A", 1, vec!["x"]));
    blocklace.insert(block("approver_c", "C", 1, vec!["x"]));
    blocklace.insert(block("witness_z", "Z", 2, vec!["approver_a", "approver_c"]));

    assert!(!ratify(
        &blocklace,
        blocklace.block_ref("witness_z"),
        blocklace.block_ref("x"),
        &validators,
        threshold(),
    ));
}

#[test]
fn super_ratify_succeeds_with_weighted_supermajority_ratifiers() {
    let validators = StaticValidators::weighted_abcd();
    let mut blocklace = MemoryBlocklace::default();

    blocklace.insert(block("x", "L", 0, vec![]));
    blocklace.insert(block("approver_a", "A", 1, vec!["x"]));
    blocklace.insert(block("approver_b", "B", 1, vec!["x"]));
    blocklace.insert(block(
        "ratifier_a",
        "A",
        2,
        vec!["approver_a", "approver_b"],
    ));
    blocklace.insert(block(
        "ratifier_b",
        "B",
        2,
        vec!["approver_a", "approver_b"],
    ));

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

    blocklace.insert(block("x", "L", 0, vec![]));
    blocklace.insert(block("approver_a", "A", 1, vec!["x"]));
    blocklace.insert(block("approver_b", "B", 1, vec!["x"]));
    blocklace.insert(block(
        "ratifier_b",
        "B",
        2,
        vec!["approver_a", "approver_b"],
    ));
    blocklace.insert(block(
        "ratifier_c",
        "C",
        2,
        vec!["approver_a", "approver_b"],
    ));
    blocklace.insert(block(
        "ratifier_d",
        "D",
        2,
        vec!["approver_a", "approver_b"],
    ));

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

    blocklace.insert(block("x", "L", 0, vec![]));
    blocklace.insert(block("a1", "A", 1, vec!["x"]));
    blocklace.insert(block("a2", "A", 2, vec!["a1"]));
    blocklace.insert(block("witness_z", "Z", 3, vec!["a2"]));
    blocklace.insert(block("approver_a", "A", 1, vec!["x"]));
    blocklace.insert(block("approver_b", "B", 1, vec!["x"]));
    blocklace.insert(block(
        "ratifier_a1",
        "A",
        2,
        vec!["approver_a", "approver_b"],
    ));
    blocklace.insert(block(
        "ratifier_a2",
        "A",
        2,
        vec!["approver_a", "approver_b"],
    ));

    assert!(!ratify(
        &blocklace,
        blocklace.block_ref("witness_z"),
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

    blocklace.insert(block("x", "L", 0, vec![]));
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

    blocklace.insert(block("x", "L", 0, vec![]));
    blocklace.insert(block("x_prime", "L", 0, vec![]));
    blocklace.insert(block("approver_a", "A", 1, vec!["x"]));
    blocklace.insert(block("approver_b", "B", 1, vec!["x"]));
    blocklace.insert(block(
        "ratifier_a",
        "A",
        2,
        vec!["approver_a", "approver_b"],
    ));
    blocklace.insert(block(
        "ratifier_b",
        "B",
        2,
        vec!["approver_a", "approver_b"],
    ));

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
fn witnesses_that_observe_equivocation_approve_neither_candidate() {
    let validators = StaticValidators::weighted_abcd();
    let mut blocklace = MemoryBlocklace::default();

    blocklace.insert(block("x", "L", 0, vec![]));
    blocklace.insert(block("x_prime", "L", 0, vec![]));
    blocklace.insert(block("w_a", "A", 1, vec!["x", "x_prime"]));
    blocklace.insert(block("w_b", "B", 1, vec!["x", "x_prime"]));
    blocklace.insert(block("w_c", "C", 1, vec!["x", "x_prime"]));
    blocklace.insert(block("w_d", "D", 1, vec!["x", "x_prime"]));

    assert!(!approve(
        &blocklace,
        blocklace.block_ref("w_a"),
        blocklace.block_ref("x")
    ));
    assert!(!approve(
        &blocklace,
        blocklace.block_ref("w_a"),
        blocklace.block_ref("x_prime")
    ));

    let witnesses = ["w_a", "w_b", "w_c", "w_d"];
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

    assert!(!x_super_ratifies);
    assert!(!x_prime_super_ratifies);
    assert!(!(x_super_ratifies && x_prime_super_ratifies));
}

#[test]
fn memoization_reuses_approval_and_ratification_pairs() {
    let validators = StaticValidators::weighted_abcd();
    let mut blocklace = MemoryBlocklace::default();

    blocklace.insert(block("x", "L", 0, vec![]));
    blocklace.insert(block("approver_a", "A", 1, vec!["x"]));
    blocklace.insert(block("approver_b", "B", 1, vec!["x"]));
    blocklace.insert(block(
        "ratifier_a",
        "A",
        2,
        vec!["approver_a", "approver_b"],
    ));
    blocklace.insert(block(
        "ratifier_b",
        "B",
        2,
        vec!["approver_a", "approver_b"],
    ));

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
}

#[test]
fn invalid_thresholds_are_rejected() {
    assert_eq!(
        ApprovalThreshold::try_new(2, 0),
        Err(ThresholdError::ZeroDenominator)
    );
    assert_eq!(
        ApprovalThreshold::try_new(3, 3),
        Err(ThresholdError::NonStrictProperFraction)
    );
    assert_eq!(
        ApprovalThreshold::try_new(4, 3),
        Err(ThresholdError::NonStrictProperFraction)
    );
}

#[test]
fn approve_returns_false_when_candidate_missing() {
    let mut blocklace = MemoryBlocklace::default();
    blocklace.insert(block("approver_a", "A", 1, vec!["missing"]));
    let missing_candidate = block("missing", "L", 0, vec![]);

    assert!(!approve(
        &blocklace,
        blocklace.block_ref("approver_a"),
        &missing_candidate
    ));
}

#[test]
fn approve_returns_false_when_approver_missing() {
    let mut blocklace = MemoryBlocklace::default();
    blocklace.insert(block("x", "L", 0, vec![]));
    let missing_approver = block("missing_approver", "A", 1, vec!["x"]);

    assert!(!approve(
        &blocklace,
        &missing_approver,
        blocklace.block_ref("x")
    ));
}

#[test]
fn ratify_returns_false_when_witness_missing() {
    let validators = StaticValidators::weighted_abcd();
    let mut blocklace = MemoryBlocklace::default();
    blocklace.insert(block("x", "L", 0, vec![]));
    let missing_witness = block("missing_witness", "A", 1, vec!["x"]);

    assert!(!ratify(
        &blocklace,
        &missing_witness,
        blocklace.block_ref("x"),
        &validators,
        threshold(),
    ));
}

#[test]
fn super_ratify_ignores_missing_witness_ids() {
    let validators = StaticValidators::weighted_abcd();
    let mut blocklace = MemoryBlocklace::default();

    blocklace.insert(block("x", "L", 0, vec![]));
    blocklace.insert(block("approver_a", "A", 1, vec!["x"]));
    blocklace.insert(block("approver_b", "B", 1, vec!["x"]));
    blocklace.insert(block(
        "ratifier_a",
        "A",
        2,
        vec!["approver_a", "approver_b"],
    ));
    blocklace.insert(block(
        "ratifier_b",
        "B",
        2,
        vec!["approver_a", "approver_b"],
    ));

    assert!(super_ratify(
        &blocklace,
        &["missing_witness", "ratifier_a", "ratifier_b"],
        blocklace.block_ref("x"),
        &validators,
        threshold(),
    ));
}

#[test]
fn super_ratify_returns_false_for_empty_witness_set() {
    let validators = StaticValidators::weighted_abcd();
    let mut blocklace = MemoryBlocklace::default();
    blocklace.insert(block("x", "L", 0, vec![]));

    assert!(!super_ratify(
        &blocklace,
        &[],
        blocklace.block_ref("x"),
        &validators,
        threshold(),
    ));
}

#[test]
fn ratify_handles_missing_parent_without_finality() {
    let validators = StaticValidators::weighted_abcd();
    let mut blocklace = MemoryBlocklace::default();

    blocklace.insert(block("x", "L", 0, vec![]));
    blocklace.insert(block("approver_a", "A", 1, vec!["x", "missing_parent"]));
    blocklace.insert(block("witness_z", "Z", 2, vec!["approver_a"]));

    assert!(!ratify(
        &blocklace,
        blocklace.block_ref("witness_z"),
        blocklace.block_ref("x"),
        &validators,
        threshold(),
    ));
}

#[test]
fn super_ratify_ignores_unrelated_non_ratifying_witnesses() {
    let validators = StaticValidators::weighted_abcd();
    let mut blocklace = MemoryBlocklace::default();

    blocklace.insert(block("x", "L", 0, vec![]));
    blocklace.insert(block("approver_a", "A", 1, vec!["x"]));
    blocklace.insert(block("approver_b", "B", 1, vec!["x"]));
    blocklace.insert(block(
        "ratifier_a",
        "A",
        2,
        vec!["approver_a", "approver_b"],
    ));
    blocklace.insert(block(
        "ratifier_b",
        "B",
        2,
        vec!["approver_a", "approver_b"],
    ));
    blocklace.insert(block("unrelated_c", "C", 2, vec![]));

    assert!(super_ratify(
        &blocklace,
        &["unrelated_c", "ratifier_a", "ratifier_b"],
        blocklace.block_ref("x"),
        &validators,
        threshold(),
    ));
}

#[test]
fn memo_context_is_scoped_to_threshold() {
    let validators = StaticValidators::weighted_abcd();
    let mut blocklace = MemoryBlocklace::default();

    blocklace.insert(block("x", "L", 0, vec![]));
    blocklace.insert(block("approver_a", "A", 1, vec!["x"]));
    blocklace.insert(block("approver_c", "C", 1, vec!["x"]));
    blocklace.insert(block("witness_z", "Z", 2, vec!["approver_a", "approver_c"]));

    let mut strict_memo = FinalityMemo::new();
    let mut half_memo = FinalityMemo::new();
    let half = ApprovalThreshold::try_new(1, 2).expect("valid threshold");

    assert!(!ratify_with_memo(
        &blocklace,
        blocklace.block_ref("witness_z"),
        blocklace.block_ref("x"),
        &validators,
        threshold(),
        &mut strict_memo,
    ));
    assert!(ratify_with_memo(
        &blocklace,
        blocklace.block_ref("witness_z"),
        blocklace.block_ref("x"),
        &validators,
        half,
        &mut half_memo,
    ));
}

#[test]
fn support_weight_overflow_fails_closed() {
    let validators = StaticValidators {
        weights: HashMap::from([("A", u128::MAX - 2), ("B", 10)]),
        total: u128::MAX,
    };
    let high_threshold =
        ApprovalThreshold::try_new(u128::MAX - 1, u128::MAX).expect("valid threshold");
    let mut blocklace = MemoryBlocklace::default();

    blocklace.insert(block("x", "L", 0, vec![]));
    blocklace.insert(block("approver_a", "A", 1, vec!["x"]));
    blocklace.insert(block("approver_b", "B", 1, vec!["x"]));
    blocklace.insert(block("witness_z", "Z", 2, vec!["approver_a", "approver_b"]));

    assert!(!ratify(
        &blocklace,
        blocklace.block_ref("witness_z"),
        blocklace.block_ref("x"),
        &validators,
        high_threshold,
    ));
}

#[test]
fn wide_mul_matches_boundary_cases() {
    assert_eq!(wide_mul(0, u128::MAX), (0, 0));
    assert_eq!(wide_mul(1, u128::MAX), (0, u128::MAX));

    let u64_max = u64::MAX as u128;
    assert_eq!(wide_mul(u64_max, u64_max), (0, u64_max * u64_max));
    assert_eq!(wide_mul(u64_max + 1, u64_max + 1), (1, 0));
    assert_eq!(wide_mul(u128::MAX, 2), (1, u128::MAX - 1));
    assert_eq!(wide_mul(u128::MAX, u128::MAX), (u128::MAX - 1, 1));
}

#[test]
fn threshold_comparison_handles_u128_cross_multiplication_overflow() {
    let almost_one = ApprovalThreshold::try_new(u128::MAX - 1, u128::MAX).expect("valid threshold");
    let half = ApprovalThreshold::try_new(1, 2).expect("valid threshold");

    assert!(almost_one.is_met(u128::MAX, u128::MAX));
    assert!(!almost_one.is_met(u128::MAX - 1, u128::MAX));
    assert!(half.is_met(u128::MAX, u128::MAX));
}
