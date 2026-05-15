use cordial_miners_core::{
    Block, BlockContent, BlockIdentity, Blocklace, NodeId,
    consensus::{
        approves, is_supermajority, is_weighted_supermajority, ratifies, super_ratifies,
        weighted_approving_creators, weighted_ratifies, weighted_super_ratifies,
    },
    crypto::CryptoVerifier,
};
use std::collections::{HashMap, HashSet};

struct MockVerifier;

impl CryptoVerifier for MockVerifier {
    type Error = String;

    fn verify_block(
        &self,
        _content: &BlockContent,
        _sig: &[u8],
        _creator: &NodeId,
    ) -> Result<(), Self::Error> {
        Ok(())
    }
}

fn node(id: u8) -> NodeId {
    NodeId(vec![id])
}

fn block(creator_id: u8, hash_byte: u8, predecessors: HashSet<BlockIdentity>) -> Block {
    let mut content_hash = [0u8; 32];
    content_hash[0] = creator_id;
    content_hash[1] = hash_byte;

    Block {
        identity: BlockIdentity {
            content_hash,
            creator: node(creator_id),
            signature: vec![],
        },
        content: BlockContent {
            payload: vec![],
            predecessors,
        },
    }
}

fn insert(blocklace: &mut Blocklace, block: &Block) {
    let verifier = MockVerifier;
    blocklace
        .insert(block.clone(), &verifier)
        .expect("insert failed");
}

fn bonds(entries: &[(u8, u64)]) -> HashMap<NodeId, u64> {
    entries
        .iter()
        .map(|(creator, weight)| (node(*creator), *weight))
        .collect()
}

struct WeightedFixture {
    blocklace: Blocklace,
    target: Block,
    approver2: Block,
    approver3: Block,
    approver4: Block,
    weights: HashMap<NodeId, u64>,
}

fn target_with_weighted_approvers() -> WeightedFixture {
    let mut blocklace = Blocklace::new();

    let target = block(1, 1, HashSet::new());
    insert(&mut blocklace, &target);

    let approver2 = block(2, 2, HashSet::from([target.identity.clone()]));
    let approver3 = block(3, 3, HashSet::from([target.identity.clone()]));
    let approver4 = block(4, 4, HashSet::from([target.identity.clone()]));
    insert(&mut blocklace, &approver2);
    insert(&mut blocklace, &approver3);
    insert(&mut blocklace, &approver4);

    WeightedFixture {
        blocklace,
        target,
        approver2,
        approver3,
        approver4,
        weights: bonds(&[(2, 4), (3, 3), (4, 3)]),
    }
}

#[test]
fn weighted_approving_creators_returns_positive_weight_supporters() {
    let mut blocklace = Blocklace::new();

    let target = block(1, 1, HashSet::new());
    insert(&mut blocklace, &target);

    let bonded = block(2, 2, HashSet::from([target.identity.clone()]));
    let unknown = block(3, 3, HashSet::from([target.identity.clone()]));
    let zero_weight = block(4, 4, HashSet::from([target.identity.clone()]));
    insert(&mut blocklace, &bonded);
    insert(&mut blocklace, &unknown);
    insert(&mut blocklace, &zero_weight);

    let blocks = HashSet::from([bonded, unknown, zero_weight]);
    let result = weighted_approving_creators(
        &blocklace,
        &blocks,
        &target.identity,
        &bonds(&[(2, 5), (4, 0)]),
    );

    assert_eq!(result, HashSet::from([node(2)]));
}

#[test]
fn weighted_ratifies_succeeds_with_strict_two_thirds_stake() {
    let WeightedFixture {
        mut blocklace,
        target,
        approver2,
        approver3,
        approver4,
        weights,
    } = target_with_weighted_approvers();

    let ratifier = block(
        2,
        5,
        HashSet::from([
            approver2.identity.clone(),
            approver3.identity.clone(),
            approver4.identity.clone(),
        ]),
    );
    insert(&mut blocklace, &ratifier);

    assert!(weighted_ratifies(&blocklace, &ratifier, &target, &weights));
}

#[test]
fn weighted_ratifies_fails_below_strict_two_thirds_stake() {
    let mut blocklace = Blocklace::new();

    let target = block(1, 1, HashSet::new());
    insert(&mut blocklace, &target);

    let approver = block(2, 2, HashSet::from([target.identity.clone()]));
    insert(&mut blocklace, &approver);

    let ratifier = block(4, 3, HashSet::from([approver.identity.clone()]));
    insert(&mut blocklace, &ratifier);

    let weights = bonds(&[(2, 4), (3, 4), (4, 2)]);

    assert!(!weighted_ratifies(&blocklace, &ratifier, &target, &weights));
}

#[test]
fn weighted_ratifies_uses_inclusive_ratifier_closure() {
    let mut blocklace = Blocklace::new();

    let target = block(1, 1, HashSet::new());
    insert(&mut blocklace, &target);

    let ratifier = block(2, 2, HashSet::from([target.identity.clone()]));
    insert(&mut blocklace, &ratifier);

    let weights = bonds(&[(2, 7), (3, 3)]);

    assert!(weighted_ratifies(&blocklace, &ratifier, &target, &weights));
}

#[test]
fn weighted_ratifies_counts_each_approving_creator_once() {
    let mut blocklace = Blocklace::new();

    let target = block(1, 1, HashSet::new());
    insert(&mut blocklace, &target);

    let first_approval = block(2, 2, HashSet::from([target.identity.clone()]));
    insert(&mut blocklace, &first_approval);

    let second_approval = block(2, 3, HashSet::from([first_approval.identity.clone()]));
    insert(&mut blocklace, &second_approval);

    let ratifier = block(5, 4, HashSet::from([second_approval.identity.clone()]));
    insert(&mut blocklace, &ratifier);

    let weights = bonds(&[(2, 4), (3, 3), (4, 3)]);

    assert!(!weighted_ratifies(&blocklace, &ratifier, &target, &weights));
}

#[test]
fn weighted_super_ratifies_succeeds_with_weighted_ratifier_majority() {
    let WeightedFixture {
        mut blocklace,
        target,
        approver2,
        approver3,
        approver4,
        weights,
    } = target_with_weighted_approvers();

    let predecessors = HashSet::from([
        approver2.identity.clone(),
        approver3.identity.clone(),
        approver4.identity.clone(),
    ]);
    let ratifier2 = block(2, 5, predecessors.clone());
    let ratifier3 = block(3, 6, predecessors.clone());
    let ratifier4 = block(4, 7, predecessors);
    insert(&mut blocklace, &ratifier2);
    insert(&mut blocklace, &ratifier3);
    insert(&mut blocklace, &ratifier4);

    let witnesses = HashSet::from([ratifier2, ratifier3, ratifier4]);

    assert!(weighted_super_ratifies(
        &blocklace, &witnesses, &target, &weights
    ));
}

#[test]
fn weighted_super_ratifies_fails_below_weighted_ratifier_majority() {
    let WeightedFixture {
        mut blocklace,
        target,
        approver2,
        approver3,
        approver4,
        weights,
    } = target_with_weighted_approvers();

    let ratifier = block(
        2,
        5,
        HashSet::from([
            approver2.identity.clone(),
            approver3.identity.clone(),
            approver4.identity.clone(),
        ]),
    );
    insert(&mut blocklace, &ratifier);

    let witnesses = HashSet::from([ratifier]);

    assert!(!weighted_super_ratifies(
        &blocklace, &witnesses, &target, &weights
    ));
}

#[test]
fn weighted_super_ratifies_counts_each_ratifying_creator_once() {
    let WeightedFixture {
        mut blocklace,
        target,
        approver2,
        approver3,
        approver4,
        weights,
    } = target_with_weighted_approvers();

    let first_ratifier = block(
        2,
        5,
        HashSet::from([
            approver2.identity.clone(),
            approver3.identity.clone(),
            approver4.identity.clone(),
        ]),
    );
    insert(&mut blocklace, &first_ratifier);

    let second_ratifier = block(2, 6, HashSet::from([first_ratifier.identity.clone()]));
    insert(&mut blocklace, &second_ratifier);

    let witnesses = HashSet::from([first_ratifier, second_ratifier]);

    assert!(!weighted_super_ratifies(
        &blocklace, &witnesses, &target, &weights
    ));
}

#[test]
fn weighted_super_ratifies_ignores_unknown_and_zero_weight_ratifiers() {
    let mut blocklace = Blocklace::new();

    let target = block(1, 1, HashSet::new());
    insert(&mut blocklace, &target);

    let approver2 = block(2, 2, HashSet::from([target.identity.clone()]));
    let approver3 = block(3, 3, HashSet::from([target.identity.clone()]));
    let approver5 = block(5, 4, HashSet::from([target.identity.clone()]));
    insert(&mut blocklace, &approver2);
    insert(&mut blocklace, &approver3);
    insert(&mut blocklace, &approver5);

    let predecessors = HashSet::from([
        approver2.identity.clone(),
        approver3.identity.clone(),
        approver5.identity.clone(),
    ]);
    let unknown_ratifier = block(9, 5, predecessors.clone());
    let zero_weight_ratifier = block(4, 6, predecessors);
    insert(&mut blocklace, &unknown_ratifier);
    insert(&mut blocklace, &zero_weight_ratifier);

    let weights = bonds(&[(2, 4), (3, 3), (4, 0), (5, 3)]);
    let witnesses = HashSet::from([unknown_ratifier, zero_weight_ratifier]);

    assert!(!weighted_super_ratifies(
        &blocklace, &witnesses, &target, &weights
    ));
}

#[test]
fn weighted_ratifies_returns_false_when_target_missing() {
    let mut blocklace = Blocklace::new();

    let ratifier = block(2, 1, HashSet::new());
    insert(&mut blocklace, &ratifier);

    let missing_target = block(1, 2, HashSet::new());

    assert!(!weighted_ratifies(
        &blocklace,
        &ratifier,
        &missing_target,
        &bonds(&[(2, 10)])
    ));
}

#[test]
fn weighted_ratifies_returns_false_when_ratifier_missing() {
    let mut blocklace = Blocklace::new();

    let target = block(1, 1, HashSet::new());
    insert(&mut blocklace, &target);

    let missing_ratifier = block(2, 2, HashSet::from([target.identity.clone()]));

    assert!(!weighted_ratifies(
        &blocklace,
        &missing_ratifier,
        &target,
        &bonds(&[(2, 10)])
    ));
}

#[test]
fn weighted_super_ratifies_returns_false_for_empty_block_set() {
    let mut blocklace = Blocklace::new();

    let target = block(1, 1, HashSet::new());
    insert(&mut blocklace, &target);

    assert!(!weighted_super_ratifies(
        &blocklace,
        &HashSet::new(),
        &target,
        &bonds(&[(1, 10)])
    ));
}

#[test]
fn weighted_supermajority_is_strictly_more_than_two_thirds() {
    let weights = bonds(&[(1, 3), (2, 3), (3, 3)]);

    assert!(!is_weighted_supermajority(
        &HashSet::from([node(1), node(2)]),
        &weights,
    ));

    assert!(is_weighted_supermajority(
        &HashSet::from([node(1), node(2), node(3)]),
        &weights,
    ));
}

#[test]
fn weighted_supermajority_rejects_six_of_ten_and_accepts_seven_of_ten() {
    let weights = bonds(&[(1, 4), (2, 2), (3, 1), (4, 3)]);

    assert!(!is_weighted_supermajority(
        &HashSet::from([node(1), node(2)]),
        &weights,
    ));

    assert!(is_weighted_supermajority(
        &HashSet::from([node(1), node(2), node(3)]),
        &weights,
    ));
}

#[test]
fn weighted_supermajority_returns_false_for_zero_total_weight() {
    assert!(!is_weighted_supermajority(
        &HashSet::from([node(1)]),
        &bonds(&[(1, 0)]),
    ));
}

#[test]
fn weighted_super_ratifies_same_creator_conflicts_cannot_both_pass() {
    let mut blocklace = Blocklace::new();

    let x = block(1, 1, HashSet::new());
    let x_prime = block(1, 2, HashSet::new());
    insert(&mut blocklace, &x);
    insert(&mut blocklace, &x_prime);

    let predecessors = HashSet::from([x.identity.clone(), x_prime.identity.clone()]);
    let witness2 = block(2, 3, predecessors.clone());
    let witness3 = block(3, 4, predecessors.clone());
    let witness4 = block(4, 5, predecessors);
    insert(&mut blocklace, &witness2);
    insert(&mut blocklace, &witness3);
    insert(&mut blocklace, &witness4);

    let witnesses = HashSet::from([witness2, witness3, witness4]);
    let weights = bonds(&[(2, 4), (3, 3), (4, 3)]);

    let x_super = weighted_super_ratifies(&blocklace, &witnesses, &x, &weights);
    let x_prime_super = weighted_super_ratifies(&blocklace, &witnesses, &x_prime, &weights);

    assert!(!x_super);
    assert!(!x_prime_super);
    assert!(!(x_super && x_prime_super));
}

#[test]
fn paper_native_ratification_predicates_are_preserved() {
    let mut blocklace = Blocklace::new();

    let target = block(1, 1, HashSet::new());
    insert(&mut blocklace, &target);

    let approver2 = block(2, 2, HashSet::from([target.identity.clone()]));
    let approver3 = block(3, 3, HashSet::from([target.identity.clone()]));
    let approver4 = block(4, 4, HashSet::from([target.identity.clone()]));
    insert(&mut blocklace, &approver2);
    insert(&mut blocklace, &approver3);
    insert(&mut blocklace, &approver4);

    let approvers = HashSet::from([approver2.clone(), approver3.clone(), approver4.clone()]);
    assert!(approves(&blocklace, &approver2.identity, &target.identity));
    assert!(is_supermajority(&approvers, 4, 1));

    let predecessors = HashSet::from([
        approver2.identity.clone(),
        approver3.identity.clone(),
        approver4.identity.clone(),
    ]);
    let ratifier2 = block(2, 5, predecessors.clone());
    let ratifier3 = block(3, 6, predecessors.clone());
    let ratifier4 = block(4, 7, predecessors);
    insert(&mut blocklace, &ratifier2);
    insert(&mut blocklace, &ratifier3);
    insert(&mut blocklace, &ratifier4);

    assert!(ratifies(&blocklace, &ratifier2, &target, 4, 1));

    let ratifiers = HashSet::from([ratifier2, ratifier3, ratifier4]);
    assert!(super_ratifies(&blocklace, &ratifiers, &target, 4, 1));
}
