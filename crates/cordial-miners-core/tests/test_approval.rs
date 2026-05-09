use cordial_miners_core::blocklace::Blocklace;
use cordial_miners_core::consensus::{approves, approving_blocks};
use cordial_miners_core::crypto::CryptoVerifier;
use cordial_miners_core::{Block, BlockContent, BlockIdentity, NodeId};
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

fn node(id: u8) -> NodeId {
    NodeId(vec![id])
}

fn create_mock_block(creator_id: u8, hash_byte: u8, predecessors: HashSet<BlockIdentity>) -> Block {
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

/// Test that a block approves a target it observes when no equivocating sibling exists.
///
/// Setup:
/// - Approver creates a block that references the target block as a predecessor
/// - Target has no equivocating siblings (no other blocks by the same creator at the same round)
///
/// Expected: approves returns true
#[test]
fn approves_observed_target_without_equivocation() {
    let mut blocklace = Blocklace::new();

    // Create a target block (initial block with no predecessors, at round 0)
    let target = create_mock_block(1, 1, HashSet::new());
    insert(&mut blocklace, &target);

    // Create an approver block that observes the target
    let approver = create_mock_block(2, 2, HashSet::from([target.identity.clone()]));
    insert(&mut blocklace, &approver);

    // The approver should approve the target since:
    // 1. Approver observes the target
    // 2. There are no equivocating siblings of the target
    assert!(approves(&blocklace, &approver.identity, &target.identity));
}

#[test]
fn block_approves_itself_when_no_conflicting_branch_is_observed() {
    let mut blocklace = Blocklace::new();

    let target = create_mock_block(1, 1, HashSet::new());
    insert(&mut blocklace, &target);

    assert!(approves(&blocklace, &target.identity, &target.identity));
}

/// Test that a block does NOT approve a target when it also observes an equivocating sibling.
///
/// Setup:
/// - Target block: created by node 1 at round 0
/// - Equivocating sibling: another block by node 1 at round 0
/// - Approver: observes both the target and its equivocating sibling
///
/// Expected: approves returns false (because approver observes an equivocating sibling)
#[test]
fn rejects_target_with_observed_equivocating_sibling() {
    let mut blocklace = Blocklace::new();

    // Create two blocks by the same creator at the same round (equivocation)
    let target = create_mock_block(1, 1, HashSet::new());
    let equivocating_sibling = create_mock_block(1, 2, HashSet::new());

    insert(&mut blocklace, &target);
    insert(&mut blocklace, &equivocating_sibling);

    // Create an approver that observes both the target and the equivocating sibling
    let approver = create_mock_block(
        2,
        3,
        HashSet::from([
            target.identity.clone(),
            equivocating_sibling.identity.clone(),
        ]),
    );
    insert(&mut blocklace, &approver);

    // The approver should NOT approve the target because it observes an equivocating sibling
    assert!(!approves(&blocklace, &approver.identity, &target.identity));
}

/// Test that a block approves a target when no equivocating sibling is observed,
/// even if equivocating siblings exist in the blocklace.
///
/// Setup:
/// - Two equivocating blocks by node 1 at round 0
/// - Approver observes only one of them (the target)
///
/// Expected: approves returns true (because approver does not observe the equivocating sibling)
#[test]
fn approves_target_when_equivocating_sibling_not_observed() {
    let mut blocklace = Blocklace::new();

    // Create two equivocating blocks
    let target = create_mock_block(1, 1, HashSet::new());
    let unobserved_sibling = create_mock_block(1, 2, HashSet::new());

    insert(&mut blocklace, &target);
    insert(&mut blocklace, &unobserved_sibling);

    // Create an approver that observes only the target, not the unobserved_sibling
    let approver = create_mock_block(2, 3, HashSet::from([target.identity.clone()]));
    insert(&mut blocklace, &approver);

    // The approver should approve the target because it does not observe the equivocating sibling
    assert!(approves(&blocklace, &approver.identity, &target.identity));
}

#[test]
fn rejects_target_when_observed_conflict_is_at_different_round() {
    let mut blocklace = Blocklace::new();

    let genesis = create_mock_block(1, 1, HashSet::new());
    insert(&mut blocklace, &genesis);

    let target = create_mock_block(1, 2, HashSet::from([genesis.identity.clone()]));
    insert(&mut blocklace, &target);

    // Same creator, different depth, and incomparable with `target`.
    let conflicting_branch = create_mock_block(1, 3, HashSet::new());
    insert(&mut blocklace, &conflicting_branch);

    let approver = create_mock_block(
        2,
        4,
        HashSet::from([target.identity.clone(), conflicting_branch.identity.clone()]),
    );
    insert(&mut blocklace, &approver);

    assert!(!approves(&blocklace, &approver.identity, &target.identity));
}

/// Test that approves returns false when the approver does not observe the target.
///
/// Setup:
/// - Target block exists but is not in the approver's predecessor closure
/// - Approver has no reference to the target
///
/// Expected: approves returns false
#[test]
fn rejects_target_not_observed_by_approver() {
    let mut blocklace = Blocklace::new();

    let target = create_mock_block(1, 1, HashSet::new());
    insert(&mut blocklace, &target);

    // Create an approver with no predecessors (does not observe the target)
    let approver = create_mock_block(2, 2, HashSet::new());
    insert(&mut blocklace, &approver);

    // The approver should not approve the target because it doesn't observe it
    assert!(!approves(&blocklace, &approver.identity, &target.identity));
}

/// Test that approves returns false when the approver block does not exist in the blocklace.
///
/// Expected: approves returns false
#[test]
fn returns_false_when_approver_not_in_blocklace() {
    let blocklace = Blocklace::new();

    let target = create_mock_block(1, 1, HashSet::new());
    let nonexistent_approver = create_mock_block(2, 2, HashSet::from([target.identity.clone()]));

    // The nonexistent_approver is not inserted into the blocklace
    // approves should return false
    assert!(!approves(
        &blocklace,
        &nonexistent_approver.identity,
        &target.identity
    ));
}

/// Test that approves returns false when the target block does not exist in the blocklace.
///
/// Expected: approves returns false
#[test]
fn returns_false_when_target_not_in_blocklace() {
    let mut blocklace = Blocklace::new();

    let target = create_mock_block(1, 1, HashSet::new());
    let approver = create_mock_block(2, 2, HashSet::from([target.identity.clone()]));
    insert(&mut blocklace, &target);
    insert(&mut blocklace, &approver);

    // Create a fake target identity that doesn't exist in the blocklace
    let nonexistent_target = BlockIdentity {
        content_hash: [3u8; 32],
        creator: node(3),
        signature: vec![],
    };

    // approves should return false for a nonexistent target
    assert!(!approves(
        &blocklace,
        &approver.identity,
        &nonexistent_target
    ));
}

/// Test approval through indirect observation (transitive closure of predecessors).
///
/// Setup:
/// - Target block exists
/// - Witness block observes the target
/// - Approver observes the witness (transitively observes the target through the witness)
///
/// Expected: approves returns true (because the target is in the transitive closure)
#[test]
fn approves_through_transitive_predecessor_closure() {
    let mut blocklace = Blocklace::new();

    let target = create_mock_block(1, 1, HashSet::new());
    insert(&mut blocklace, &target);

    let witness = create_mock_block(2, 2, HashSet::from([target.identity.clone()]));
    insert(&mut blocklace, &witness);

    let approver = create_mock_block(3, 3, HashSet::from([witness.identity.clone()]));
    insert(&mut blocklace, &approver);

    // The approver transitively observes the target through the witness
    assert!(approves(&blocklace, &approver.identity, &target.identity));
}

/// Test that approving_blocks returns the correct set of blocks that approve a target.
///
/// Setup:
/// - Target block at round 0
/// - Target observes itself → approves
/// - Block A observes target → approves
/// - Block B observes target → approves
/// - Block C does not observe target → does not approve
/// - Block D observes an equivocating sibling of target → does not approve
///
/// Expected: approving_blocks returns {target, A, B}
#[test]
fn approving_blocks_returns_correct_set() {
    let mut blocklace = Blocklace::new();

    // Create target and two equivocating siblings
    let target = create_mock_block(1, 1, HashSet::new());
    let equivocating_sibling = create_mock_block(1, 2, HashSet::new());
    insert(&mut blocklace, &target);
    insert(&mut blocklace, &equivocating_sibling);

    // Block A observes only the target
    let block_a = create_mock_block(2, 10, HashSet::from([target.identity.clone()]));
    insert(&mut blocklace, &block_a);

    // Block B observes only the target
    let block_b = create_mock_block(3, 11, HashSet::from([target.identity.clone()]));
    insert(&mut blocklace, &block_b);

    // Block C doesn't observe anything
    let block_c = create_mock_block(4, 12, HashSet::new());
    insert(&mut blocklace, &block_c);

    // Block D observes both target and its equivocating sibling
    let block_d = create_mock_block(
        5,
        13,
        HashSet::from([
            target.identity.clone(),
            equivocating_sibling.identity.clone(),
        ]),
    );
    insert(&mut blocklace, &block_d);

    let approvers = approving_blocks(&blocklace, &target.identity);

    // Expected: target, block_a and block_b approve the target
    // block_c doesn't observe the target
    // block_d observes an equivocating sibling so doesn't approve
    assert_eq!(approvers.len(), 3);
    assert!(approvers.contains(&target));
    assert!(approvers.contains(&block_a));
    assert!(approvers.contains(&block_b));
    assert!(!approvers.contains(&block_c));
    assert!(!approvers.contains(&block_d));
}

/// Test that approving_blocks returns an empty set when no block observes the target.
///
/// Setup:
/// - Target block exists in the blocklace
/// - The target exists and approves itself
/// - Multiple other blocks exist but none observe the target
///
/// Expected: approving_blocks returns only the target
#[test]
fn approving_blocks_returns_empty_when_no_block_observes_target() {
    let mut blocklace = Blocklace::new();

    let target = create_mock_block(1, 1, HashSet::new());
    insert(&mut blocklace, &target);

    // Create several blocks that don't observe the target
    let block_a = create_mock_block(2, 10, HashSet::new());
    insert(&mut blocklace, &block_a);

    let block_b = create_mock_block(3, 11, HashSet::new());
    insert(&mut blocklace, &block_b);

    let block_c = create_mock_block(4, 12, HashSet::new());
    insert(&mut blocklace, &block_c);

    let approvers = approving_blocks(&blocklace, &target.identity);

    // No other blocks observe the target, but the target approves itself.
    assert_eq!(approvers.len(), 1);
    assert!(approvers.contains(&target));
}
