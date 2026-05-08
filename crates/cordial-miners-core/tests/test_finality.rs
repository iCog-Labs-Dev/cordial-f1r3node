use cordial_miners_core::blocklace::Blocklace;
use cordial_miners_core::consensus::{
    FinalityStatus, can_be_finalized, check_finality, find_last_finalized,
};
use cordial_miners_core::cordiality::ValidatorSet;
use cordial_miners_core::crypto::CryptoVerifier;
use cordial_miners_core::finality::{ApprovalThreshold, approve, approves};
use cordial_miners_core::{Block, BlockContent, BlockIdentity, NodeId};
use std::collections::HashMap;
use std::collections::HashSet;

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

// ── Mock ValidatorSet ──

struct MockValidatorSet {
    weights: HashMap<NodeId, u128>,
    total: u128,
}

impl MockValidatorSet {
    fn new() -> Self {
        Self {
            weights: HashMap::new(),
            total: 0,
        }
    }

    fn add_validator(mut self, node_id: NodeId, weight: u128) -> Self {
        self.weights.insert(node_id, weight);
        self.total += weight;
        self
    }
}

impl ValidatorSet<&NodeId> for MockValidatorSet {
    type Weight = u128;

    fn weight_of(&self, validator: &&NodeId) -> Option<Self::Weight> {
        self.weights.get(*validator).copied()
    }

    fn total_weight(&self) -> Self::Weight {
        self.total
    }
}

// ── Helpers ──

fn node(id: u8) -> NodeId {
    NodeId(vec![id])
}

fn make_id(creator: &NodeId, tag: u8) -> BlockIdentity {
    let mut hash = [0u8; 32];
    hash[0] = creator.0[0];
    hash[1] = tag;
    BlockIdentity {
        content_hash: hash,
        creator: creator.clone(),
        signature: vec![tag],
    }
}

fn genesis(creator: &NodeId, tag: u8) -> Block {
    Block {
        identity: make_id(creator, tag),
        content: BlockContent {
            payload: vec![tag],
            predecessors: HashSet::new(),
        },
    }
}

fn child(creator: &NodeId, tag: u8, parents: &[&Block]) -> Block {
    let preds = parents.iter().map(|b| b.identity.clone()).collect();
    Block {
        identity: make_id(creator, tag),
        content: BlockContent {
            payload: vec![tag],
            predecessors: preds,
        },
    }
}

/// Helper to create a block with unique hash and specified predecessors
/// Used by the approval tests which need more control over hash bytes
fn create_block(creator_id: u8, hash_byte: u8, predecessors: HashSet<BlockIdentity>) -> Block {
    let mut content_hash = [0u8; 32];
    content_hash[0] = hash_byte;

    Block {
        identity: BlockIdentity {
            content_hash,
            creator: NodeId(vec![creator_id]),
            signature: vec![],
        },
        content: BlockContent {
            payload: vec![],
            predecessors,
        },
    }
}

fn insert(bl: &mut Blocklace, block: &Block) {
    let verifier = MockVerifier;
    bl.insert(block.clone(), &verifier).expect("insert failed");
}

fn bonds(entries: &[(u8, u64)]) -> HashMap<NodeId, u64> {
    entries
        .iter()
        .map(|(id, stake)| (node(*id), *stake))
        .collect()
}

// ── check_finality tests ──

#[test]
fn unknown_block_returns_unknown() {
    let bl = Blocklace::new();
    let fake_id = make_id(&node(1), 99);
    let b = bonds(&[(1, 100)]);
    assert_eq!(check_finality(&bl, &fake_id, &b), FinalityStatus::Unknown);
}

#[test]
fn single_validator_finalizes_own_genesis() {
    let mut bl = Blocklace::new();
    let v1 = node(1);
    let g = genesis(&v1, 1);
    insert(&mut bl, &g);

    let b = bonds(&[(1, 100)]);
    let status = check_finality(&bl, &g.identity, &b);
    assert!(status.is_finalized());
}

#[test]
fn block_not_in_supermajority_ancestry_is_pending() {
    let mut bl = Blocklace::new();
    let v1 = node(1);
    let v2 = node(2);
    let v3 = node(3);

    let g1 = genesis(&v1, 1);
    let g2 = genesis(&v2, 2);
    let g3 = genesis(&v3, 3);
    insert(&mut bl, &g1);
    insert(&mut bl, &g2);
    insert(&mut bl, &g3);

    let b = bonds(&[(1, 100), (2, 100), (3, 100)]);
    let status = check_finality(&bl, &g1.identity, &b);
    assert!(status.is_pending());
}

#[test]
fn supermajority_support_finalizes_block() {
    let mut bl = Blocklace::new();
    let v1 = node(1);
    let v2 = node(2);
    let v3 = node(3);

    let g = genesis(&v1, 1);
    insert(&mut bl, &g);

    let b2 = child(&v2, 2, &[&g]);
    let b3 = child(&v3, 3, &[&g]);
    insert(&mut bl, &b2);
    insert(&mut bl, &b3);

    let b = bonds(&[(1, 100), (2, 100), (3, 100)]);
    let status = check_finality(&bl, &g.identity, &b);

    assert!(status.is_finalized());
    match status {
        FinalityStatus::Finalized {
            supporting_stake,
            total_honest_stake,
        } => {
            assert_eq!(supporting_stake, 300);
            assert_eq!(total_honest_stake, 300);
        }
        _ => panic!("expected Finalized"),
    }
}

#[test]
fn two_thirds_plus_one_is_enough() {
    let mut bl = Blocklace::new();
    let v1 = node(1);
    let v2 = node(2);
    let v3 = node(3);

    let g = genesis(&v1, 1);
    insert(&mut bl, &g);

    let b2 = child(&v2, 2, &[&g]);
    let g3 = genesis(&v3, 3);
    insert(&mut bl, &b2);
    insert(&mut bl, &g3);

    let b = bonds(&[(1, 100), (2, 100), (3, 100)]);
    let status = check_finality(&bl, &g.identity, &b);
    assert!(status.is_pending());

    let b2 = bonds(&[(1, 100), (2, 100), (3, 99)]);
    let status2 = check_finality(&bl, &g.identity, &b2);
    assert!(status2.is_finalized());
}

#[test]
fn equivocator_stake_excluded_from_total() {
    let mut bl = Blocklace::new();
    let v1 = node(1);
    let v2 = node(2);
    let v3 = node(3);

    let g = genesis(&v1, 1);
    insert(&mut bl, &g);

    let b2 = child(&v2, 2, &[&g]);
    insert(&mut bl, &b2);

    let g3a = genesis(&v3, 3);
    let g3b = genesis(&v3, 4);
    insert(&mut bl, &g3a);
    insert(&mut bl, &g3b);

    let b = bonds(&[(1, 100), (2, 100), (3, 1000)]);
    let status = check_finality(&bl, &g.identity, &b);
    assert!(status.is_finalized());
}

// ── find_last_finalized tests ──

#[test]
fn no_finalized_block_in_empty_blocklace() {
    let bl = Blocklace::new();
    let b = bonds(&[(1, 100)]);
    assert!(find_last_finalized(&bl, &b).is_none());
}

#[test]
fn find_last_finalized_returns_highest_finalized() {
    let mut bl = Blocklace::new();
    let v1 = node(1);
    let v2 = node(2);
    let v3 = node(3);

    let g = genesis(&v1, 1);
    insert(&mut bl, &g);

    let b2 = child(&v2, 2, &[&g]);
    insert(&mut bl, &b2);

    let b3 = child(&v3, 3, &[&b2]);
    insert(&mut bl, &b3);

    let b4 = child(&v1, 4, &[&b3]);
    insert(&mut bl, &b4);

    let b = bonds(&[(1, 100), (2, 100), (3, 100)]);
    let lfb = find_last_finalized(&bl, &b);

    assert!(lfb.is_some());
    let lfb = lfb.unwrap();
    assert!(check_finality(&bl, &lfb, &b).is_finalized());
}

#[test]
fn single_validator_last_finalized_is_tip() {
    let mut bl = Blocklace::new();
    let v1 = node(1);
    let g = genesis(&v1, 1);
    let b2 = child(&v1, 2, &[&g]);
    insert(&mut bl, &g);
    insert(&mut bl, &b2);

    let b = bonds(&[(1, 100)]);
    let lfb = find_last_finalized(&bl, &b).unwrap();

    assert_eq!(lfb, b2.identity);
}

// ── can_be_finalized tests ──

#[test]
fn unknown_block_cannot_be_finalized() {
    let bl = Blocklace::new();
    let fake_id = make_id(&node(1), 99);
    let b = bonds(&[(1, 100)]);
    assert!(!can_be_finalized(&bl, &fake_id, &b));
}

#[test]
fn block_with_full_support_can_be_finalized() {
    let mut bl = Blocklace::new();
    let v1 = node(1);
    let g = genesis(&v1, 1);
    insert(&mut bl, &g);

    let b = bonds(&[(1, 100), (2, 100)]);
    assert!(can_be_finalized(&bl, &g.identity, &b));
}

#[test]
fn orphaned_block_cannot_be_finalized() {
    let mut bl = Blocklace::new();
    let v1 = node(1);
    let v2 = node(2);
    let v3 = node(3);

    let g1 = genesis(&v1, 1);
    let g2 = genesis(&v2, 2);
    let g3 = genesis(&v3, 3);
    insert(&mut bl, &g1);
    insert(&mut bl, &g2);
    insert(&mut bl, &g3);

    let b = bonds(&[(1, 100), (2, 100), (3, 100)]);
    assert!(!can_be_finalized(&bl, &g1.identity, &b));
}

#[test]
fn finality_status_helpers() {
    let finalized = FinalityStatus::Finalized {
        supporting_stake: 200,
        total_honest_stake: 300,
    };
    let pending = FinalityStatus::Pending {
        supporting_stake: 100,
        total_honest_stake: 300,
    };
    let unknown = FinalityStatus::Unknown;

    assert!(finalized.is_finalized());
    assert!(!finalized.is_pending());

    assert!(!pending.is_finalized());
    assert!(pending.is_pending());

    assert!(!unknown.is_finalized());
    assert!(!unknown.is_pending());
}

// ── approve / weighted_approve tests ──

#[test]
fn test_approve_returns_false_if_candidate_not_observed() {
    let mut bl = Blocklace::new();

    let genesis_b = create_block(1, 1, HashSet::new());
    insert(&mut bl, &genesis_b);

    // block_v2 points to genesis but NOT to unobserved_candidate
    let block_v2 = create_block(2, 2, [genesis_b.identity.clone()].iter().cloned().collect());
    insert(&mut bl, &block_v2);

    // unobserved_candidate is a standalone block block_v2 cannot see
    let unobserved_candidate = create_block(3, 3, HashSet::new());
    insert(&mut bl, &unobserved_candidate);

    assert!(!approves(
        &bl,
        &block_v2.identity,
        &unobserved_candidate.identity
    ));
}

#[test]
fn test_weighted_approve_returns_false_below_threshold() {
    let mut bl = Blocklace::new();

    let genesis_b = create_block(1, 1, HashSet::new());
    insert(&mut bl, &genesis_b);

    let candidate = create_block(2, 2, [genesis_b.identity.clone()].iter().cloned().collect());
    insert(&mut bl, &candidate);

    // Only validator 1 (weight 1) observes the candidate
    let approve_block = create_block(1, 3, [candidate.identity.clone()].iter().cloned().collect());
    insert(&mut bl, &approve_block);

    // Total weight = 3, supporting = 1 — well below 2/3
    let validators = MockValidatorSet::new()
        .add_validator(NodeId(vec![1]), 1)
        .add_validator(NodeId(vec![2]), 1)
        .add_validator(NodeId(vec![3]), 1);

    let threshold = ApprovalThreshold::new(2, 3);

    assert!(!approve(
        &bl,
        &approve_block.identity,
        &candidate.identity,
        threshold,
        &validators
    ));
}

#[test]
fn test_weighted_approve_returns_true_above_threshold() {
    let mut bl = Blocklace::new();

    let genesis_b = create_block(1, 1, HashSet::new());
    insert(&mut bl, &genesis_b);

    let candidate = create_block(2, 2, [genesis_b.identity.clone()].iter().cloned().collect());
    insert(&mut bl, &candidate);

    // Validator 1 (weight 2) observes the candidate
    let approve_block_a =
        create_block(1, 3, [candidate.identity.clone()].iter().cloned().collect());
    insert(&mut bl, &approve_block_a);

    // Validator 3 (weight 2) observes approve_block_a (and thus candidate transitively)
    let approve_block_c = create_block(
        3,
        4,
        [approve_block_a.identity.clone()].iter().cloned().collect(),
    );
    insert(&mut bl, &approve_block_c);

    // Total = 5, support = 4 (v1+v3), 4 * 3 > 5 * 2 → true
    let validators = MockValidatorSet::new()
        .add_validator(NodeId(vec![1]), 2)
        .add_validator(NodeId(vec![2]), 1)
        .add_validator(NodeId(vec![3]), 2);

    let threshold = ApprovalThreshold::new(2, 3);

    assert!(approve(
        &bl,
        &approve_block_c.identity,
        &candidate.identity,
        threshold,
        &validators
    ));
}

#[test]
fn test_approve_returns_false_with_equivocation() {
    let mut bl = Blocklace::new();

    let genesis_b = create_block(1, 1, HashSet::new());
    insert(&mut bl, &genesis_b);

    // Validator 2 equivocates — two conflicting blocks both pointing to genesis
    let equiv1 = create_block(2, 2, [genesis_b.identity.clone()].iter().cloned().collect());
    let equiv2 = create_block(2, 3, [genesis_b.identity.clone()].iter().cloned().collect());
    insert(&mut bl, &equiv1);
    insert(&mut bl, &equiv2);

    // approve_block observes BOTH equivocating blocks
    let approve_block = create_block(
        1,
        4,
        [equiv1.identity.clone(), equiv2.identity.clone()]
            .iter()
            .cloned()
            .collect(),
    );
    insert(&mut bl, &approve_block);

    // Must return false — approver sees the equivocation
    assert!(!approves(&bl, &approve_block.identity, &equiv1.identity));
}

#[test]
fn test_weighted_approve_returns_false_with_equivocation_despite_weight() {
    let mut bl = Blocklace::new();

    let genesis_b = create_block(1, 1, HashSet::new());
    insert(&mut bl, &genesis_b);

    // Validator 2 equivocates
    let candidate_equiv1 =
        create_block(2, 2, [genesis_b.identity.clone()].iter().cloned().collect());
    let candidate_equiv2 =
        create_block(2, 3, [genesis_b.identity.clone()].iter().cloned().collect());
    insert(&mut bl, &candidate_equiv1);
    insert(&mut bl, &candidate_equiv2);

    // Both high-weight validators observe BOTH equivocating blocks
    let approve_block_a = create_block(
        1,
        4,
        [
            candidate_equiv1.identity.clone(),
            candidate_equiv2.identity.clone(),
        ]
        .iter()
        .cloned()
        .collect(),
    );
    insert(&mut bl, &approve_block_a);

    let approve_block_c = create_block(
        3,
        5,
        [
            candidate_equiv1.identity.clone(),
            candidate_equiv2.identity.clone(),
        ]
        .iter()
        .cloned()
        .collect(),
    );
    insert(&mut bl, &approve_block_c);

    // Total = 5, support would be 4 — but equivocation disqualifies
    let validators = MockValidatorSet::new()
        .add_validator(NodeId(vec![1]), 2)
        .add_validator(NodeId(vec![2]), 1)
        .add_validator(NodeId(vec![3]), 2);

    let threshold = ApprovalThreshold::new(2, 3);

    assert!(!approve(
        &bl,
        &approve_block_c.identity,
        &candidate_equiv1.identity,
        threshold,
        &validators
    ));
}

#[test]
fn test_approval_threshold_is_exceeded() {
    let threshold = ApprovalThreshold::new(2, 3);

    assert!(threshold.is_exceeded(3, 4)); // 3*3=9 > 4*2=8 ✓
    assert!(!threshold.is_exceeded(2, 4)); // 2*3=6 NOT > 4*2=8 ✓
    assert!(!threshold.is_exceeded(100, 150)); // 300 NOT > 300 ✓
}
