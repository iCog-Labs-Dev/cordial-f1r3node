use cordial_miners_core::Block;
use cordial_miners_core::blocklace::Blocklace;
use cordial_miners_core::consensus::{
    PendingBlockBuffer, ValidationConfig, required_acknowledgements, select_predecessors,
    select_predecessors_sorted, validator_visible_tips, weighted_required_acknowledgements,
};
use cordial_miners_core::crypto::CryptoVerifier;
use cordial_miners_core::types::{BlockContent, BlockIdentity, NodeId};
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

fn create_same_hash_block(
    creator_id: u8,
    shared_hash: [u8; 32],
    signature_tag: u8,
    predecessors: HashSet<BlockIdentity>,
) -> Block {
    Block {
        identity: BlockIdentity {
            content_hash: shared_hash,
            creator: node(creator_id),
            signature: vec![signature_tag],
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

fn dissemination_test_config() -> ValidationConfig {
    ValidationConfig {
        check_content_hash: false,
        ..ValidationConfig::default()
    }
}

// ============================================================================
// ACCEPTANCE TESTS (from specification)
// ============================================================================

/// **AC1**: Empty blocklace returns empty predecessor set
#[test]
fn empty_blocklace_returns_empty_predecessors() {
    let blocklace = Blocklace::new();
    let bonds = HashMap::new();

    let preds = select_predecessors(&blocklace, &bonds);

    assert!(preds.is_empty());
}

/// **AC2**: Single-chain growth: tip advances correctly as the chain grows.
///
/// Tests a 3-block chain to confirm that as each new block is appended, the
/// predecessor returned is always the latest tip, not an earlier ancestor.
#[test]
fn single_chain_growth_selects_latest_as_predecessor() {
    let mut blocklace = Blocklace::new();
    let mut bonds = HashMap::new();
    bonds.insert(node(1), 100);

    // Block 1 (genesis)
    let block1 = create_mock_block(1, 1, HashSet::new());
    insert(&mut blocklace, &block1);

    let preds = select_predecessors(&blocklace, &bonds);
    assert_eq!(preds.len(), 1);
    assert!(
        preds.contains(&block1.identity),
        "genesis should be the tip initially"
    );

    // Block 2 extends the chain
    let block2 = create_mock_block(1, 2, HashSet::from([block1.identity.clone()]));
    insert(&mut blocklace, &block2);

    let preds = select_predecessors(&blocklace, &bonds);
    assert_eq!(preds.len(), 1);
    assert!(
        preds.contains(&block2.identity),
        "block2 should be the tip after block1"
    );
    assert!(
        !preds.contains(&block1.identity),
        "block1 should no longer be the tip"
    );

    // Block 3 extends further
    let block3 = create_mock_block(1, 3, HashSet::from([block2.identity.clone()]));
    insert(&mut blocklace, &block3);

    let preds = select_predecessors(&blocklace, &bonds);
    assert_eq!(preds.len(), 1);
    assert!(
        preds.contains(&block3.identity),
        "block3 should be the tip after block2"
    );
    assert!(
        !preds.contains(&block2.identity),
        "block2 should no longer be the tip"
    );
}

/// **AC3**: Multiple validator tips are all selected as predecessors
#[test]
fn multiple_validator_tips_all_selected() {
    let mut blocklace = Blocklace::new();
    let mut bonds = HashMap::new();

    let tip_v1 = create_mock_block(1, 1, HashSet::new());
    let tip_v2 = create_mock_block(2, 2, HashSet::new());
    let tip_v3 = create_mock_block(3, 3, HashSet::new());

    insert(&mut blocklace, &tip_v1);
    insert(&mut blocklace, &tip_v2);
    insert(&mut blocklace, &tip_v3);

    bonds.insert(node(1), 100);
    bonds.insert(node(2), 100);
    bonds.insert(node(3), 100);

    let preds = select_predecessors(&blocklace, &bonds);

    assert_eq!(preds.len(), 3);
    assert!(preds.contains(&tip_v1.identity));
    assert!(preds.contains(&tip_v2.identity));
    assert!(preds.contains(&tip_v3.identity));
}

/// **AC4**: Equivocating validators are excluded from the tips map and from predecessors.
///
/// The test explicitly checks both layers:
/// 1. `validator_visible_tips` must not contain the equivocating validator at all.
/// 2. `select_predecessors` must include the honest tip and any missing
///    equivocation branches needed to avoid hiding the known equivocation.
#[test]
fn equivocating_validators_excluded_from_predecessors() {
    let mut blocklace = Blocklace::new();
    let mut bonds = HashMap::new();

    // Equivocation: two genesis-level blocks by validator 1 (no predecessor relationship)
    let e1 = create_mock_block(1, 1, HashSet::new());
    let e2 = create_mock_block(1, 2, HashSet::new());

    insert(&mut blocklace, &e1);
    insert(&mut blocklace, &e2);

    // Honest validator 2 references one branch of the equivocation transitively
    let tip_v2 = create_mock_block(2, 3, HashSet::from([e1.identity.clone()]));
    insert(&mut blocklace, &tip_v2);

    bonds.insert(node(1), 100); // equivocator
    bonds.insert(node(2), 100); // honest

    // Layer 1: tips map must not contain the equivocating validator
    let tips = validator_visible_tips(&blocklace, &bonds);
    assert!(
        !tips.contains_key(&node(1)),
        "equivocating validator 1 must be absent from the tips map"
    );
    assert!(
        tips.contains_key(&node(2)),
        "honest validator 2 must appear in the tips map"
    );

    // Layer 2: predecessor set keeps the honest tip and adds the missing
    // equivocation branch that is not already transitively observed.
    let preds = select_predecessors(&blocklace, &bonds);
    assert_eq!(preds.len(), 2);
    assert!(preds.contains(&tip_v2.identity));
    assert!(
        !preds.contains(&e1.identity),
        "already observed branch need not be a direct predecessor"
    );
    assert!(
        preds.contains(&e2.identity),
        "missing branch should be added explicitly"
    );
}

/// When a locally known equivocation is not yet fully acknowledged through the
/// honest tip set, predecessor selection must add the missing branches so a new
/// block does not hide that known equivocation.
#[test]
fn known_equivocation_branches_are_added_when_not_transitively_observed() {
    let mut blocklace = Blocklace::new();
    let mut bonds = HashMap::new();

    let e1 = create_mock_block(1, 1, HashSet::new());
    let e2 = create_mock_block(1, 2, HashSet::new());
    insert(&mut blocklace, &e1);
    insert(&mut blocklace, &e2);

    // Honest validator 2 only sees one branch of the equivocation.
    let tip_v2 = create_mock_block(2, 3, HashSet::from([e1.identity.clone()]));
    insert(&mut blocklace, &tip_v2);

    bonds.insert(node(1), 100);
    bonds.insert(node(2), 100);

    let preds = select_predecessors(&blocklace, &bonds);

    assert!(preds.contains(&tip_v2.identity));
    assert!(
        !preds.contains(&e1.identity),
        "already observed branch should remain transitive only"
    );
    assert!(
        preds.contains(&e2.identity),
        "the missing equivocation branch should be added explicitly"
    );
}

/// **AC5**: Predecessor selection is deterministic across repeated calls on the same view.
#[test]
fn deterministic_predecessor_selection() {
    let mut blocklace = Blocklace::new();
    let mut bonds = HashMap::new();

    let genesis = create_mock_block(1, 1, HashSet::new());
    insert(&mut blocklace, &genesis);
    bonds.insert(node(1), 100);

    let block2 = create_mock_block(2, 2, HashSet::from([genesis.identity.clone()]));
    insert(&mut blocklace, &block2);
    bonds.insert(node(2), 100);

    let block3 = create_mock_block(1, 3, HashSet::from([block2.identity.clone()]));
    insert(&mut blocklace, &block3);

    let block4 = create_mock_block(3, 4, HashSet::from([block3.identity.clone()]));
    insert(&mut blocklace, &block4);
    bonds.insert(node(3), 100);

    let preds1 = select_predecessors(&blocklace, &bonds);
    let preds2 = select_predecessors(&blocklace, &bonds);
    let preds3 = select_predecessors(&blocklace, &bonds);

    assert_eq!(preds1, preds2);
    assert_eq!(preds2, preds3);
}

// ============================================================================
// ADDITIONAL TESTS (robustness and edge cases)
// ============================================================================

/// Bonded validators with no blocks are absent from tips; unbonded validators'
/// blocks are ignored even when present in the blocklace.
#[test]
fn bonded_without_blocks_and_unbonded_with_blocks_are_both_excluded() {
    let mut blocklace = Blocklace::new();
    let mut bonds = HashMap::new();

    let tip_bonded = create_mock_block(1, 1, HashSet::new());
    // validator 2 is bonded but never produces a block
    // validator 3 produces a block but is not bonded
    let tip_unbonded = create_mock_block(3, 3, HashSet::new());

    insert(&mut blocklace, &tip_bonded);
    insert(&mut blocklace, &tip_unbonded);

    bonds.insert(node(1), 100);
    bonds.insert(node(2), 100); // bonded, no blocks

    let tips = validator_visible_tips(&blocklace, &bonds);
    assert_eq!(
        tips.len(),
        1,
        "only the bonded validator with a block should appear"
    );
    assert!(tips.contains_key(&node(1)));
    assert!(!tips.contains_key(&node(2)), "bonded but no blocks: absent");
    assert!(
        !tips.contains_key(&node(3)),
        "has blocks but not bonded: absent"
    );

    let preds = select_predecessors(&blocklace, &bonds);
    assert_eq!(preds.len(), 1);
    assert!(preds.contains(&tip_bonded.identity));
    assert!(!preds.contains(&tip_unbonded.identity));
}

/// All returned predecessors must exist in the blocklace (closure axiom).
#[test]
fn predecessors_are_in_blocklace() {
    let mut blocklace = Blocklace::new();
    let mut bonds = HashMap::new();

    let genesis = create_mock_block(1, 1, HashSet::new());
    insert(&mut blocklace, &genesis);
    bonds.insert(node(1), 100);

    let block2 = create_mock_block(2, 2, HashSet::from([genesis.identity.clone()]));
    insert(&mut blocklace, &block2);
    bonds.insert(node(2), 100);

    let block3 = create_mock_block(1, 3, HashSet::from([block2.identity.clone()]));
    insert(&mut blocklace, &block3);

    let preds = select_predecessors(&blocklace, &bonds);

    for pred_id in &preds {
        assert!(
            blocklace.get(pred_id).is_some(),
            "predecessor {:?} must exist in the blocklace",
            pred_id
        );
    }
}

/// Multi-layer DAG: tip identity for each validator is independently correct.
///
/// Explicitly checks `validator_visible_tips` to anchor which block is v2's tip,
/// rather than inferring it from `select_predecessors` alone.
#[test]
fn complex_dag_multilayer() {
    let mut blocklace = Blocklace::new();
    let mut bonds = HashMap::new();

    // Layer 0: independent genesis blocks
    let g1 = create_mock_block(1, 1, HashSet::new());
    let g2 = create_mock_block(2, 2, HashSet::new());
    insert(&mut blocklace, &g1);
    insert(&mut blocklace, &g2);

    // Layer 1: each validator advances their own chain
    let l1_v1 = create_mock_block(1, 3, HashSet::from([g1.identity.clone()]));
    let l1_v2 = create_mock_block(2, 4, HashSet::from([g2.identity.clone()]));
    insert(&mut blocklace, &l1_v1);
    insert(&mut blocklace, &l1_v2);

    // Layer 2: validator 1 advances again, referencing both layer-1 tips
    let l2_v1 = create_mock_block(
        1,
        5,
        HashSet::from([l1_v1.identity.clone(), l1_v2.identity.clone()]),
    );
    insert(&mut blocklace, &l2_v1);

    bonds.insert(node(1), 100);
    bonds.insert(node(2), 100);

    // Confirm individual tip identities before checking predecessor set
    let tips = validator_visible_tips(&blocklace, &bonds);
    assert_eq!(
        tips[&node(1)],
        l2_v1.identity,
        "v1's tip must be l2_v1 (the most recent block)"
    );
    assert_eq!(
        tips[&node(2)],
        l1_v2.identity,
        "v2's tip must be l1_v2 (v2 has not advanced beyond layer 1)"
    );

    let preds = select_predecessors(&blocklace, &bonds);
    assert_eq!(preds.len(), 2);
    assert!(preds.contains(&l2_v1.identity));
    assert!(preds.contains(&l1_v2.identity));
}

/// A validator with only equivocating blocks and no honest peers yields an empty
/// predecessor set.
#[test]
fn equivocating_only_validator_yields_empty_predecessors() {
    let mut blocklace = Blocklace::new();
    let mut bonds = HashMap::new();

    // 3-way equivocation at genesis level
    let e1 = create_mock_block(1, 1, HashSet::new());
    let e2 = create_mock_block(1, 2, HashSet::new());
    let e3 = create_mock_block(1, 3, HashSet::new());

    insert(&mut blocklace, &e1);
    insert(&mut blocklace, &e2);
    insert(&mut blocklace, &e3);

    bonds.insert(node(1), 100);

    // Tips map must not contain the equivocator
    let tips = validator_visible_tips(&blocklace, &bonds);
    assert!(
        !tips.contains_key(&node(1)),
        "equivocating validator must be absent from the tips map"
    );

    let preds = select_predecessors(&blocklace, &bonds);
    assert_eq!(preds.len(), 0);
}

/// Multiple equivocating validators are all excluded; honest validators remain.
#[test]
fn multiple_equivocations_different_validators() {
    let mut blocklace = Blocklace::new();
    let mut bonds = HashMap::new();

    let v1_e1 = create_mock_block(1, 1, HashSet::new());
    let v1_e2 = create_mock_block(1, 2, HashSet::new());
    let v2_e1 = create_mock_block(2, 3, HashSet::new());
    let v2_e2 = create_mock_block(2, 4, HashSet::new());

    insert(&mut blocklace, &v1_e1);
    insert(&mut blocklace, &v1_e2);
    insert(&mut blocklace, &v2_e1);
    insert(&mut blocklace, &v2_e2);

    bonds.insert(node(1), 100);
    bonds.insert(node(2), 100);

    let tips = validator_visible_tips(&blocklace, &bonds);
    assert!(
        !tips.contains_key(&node(1)),
        "equivocator v1 absent from tips"
    );
    assert!(
        !tips.contains_key(&node(2)),
        "equivocator v2 absent from tips"
    );

    let preds = select_predecessors(&blocklace, &bonds);
    assert_eq!(preds.len(), 0);
}

/// `select_predecessors_sorted` is deterministic and the output follows the
/// natural ordering of `BlockIdentity`.
#[test]
fn select_predecessors_sorted_is_deterministic_and_ordered() {
    let mut blocklace = Blocklace::new();
    let mut bonds = HashMap::new();

    // hash_byte values are set so the natural insertion order differs from sorted order:
    // creator=10 → hash[0]=10, creator=5 → hash[0]=5, creator=15 → hash[0]=15
    // Natural insertion: 10, 5, 15. Sorted order: 5, 10, 15.
    let tip1 = create_mock_block(10, 0, HashSet::new()); // hash[0]=10
    let tip2 = create_mock_block(5, 0, HashSet::new()); // hash[0]=5  (smallest)
    let tip3 = create_mock_block(15, 0, HashSet::new()); // hash[0]=15 (largest)

    insert(&mut blocklace, &tip1);
    insert(&mut blocklace, &tip2);
    insert(&mut blocklace, &tip3);

    bonds.insert(node(10), 100);
    bonds.insert(node(5), 100);
    bonds.insert(node(15), 100);

    let sorted1 = select_predecessors_sorted(&blocklace, &bonds);
    let sorted2 = select_predecessors_sorted(&blocklace, &bonds);

    // Deterministic across calls
    assert_eq!(sorted1, sorted2);

    // Correct ascending order
    assert!(
        sorted1.windows(2).all(|w| w[0] <= w[1]),
        "output must be sorted ascending by BlockIdentity"
    );

    // Verify the specific expected order: tip2 (hash[0]=5) < tip1 (hash[0]=10) < tip3 (hash[0]=15)
    assert_eq!(sorted1[0].content_hash[0], 5);
    assert_eq!(sorted1[1].content_hash[0], 10);
    assert_eq!(sorted1[2].content_hash[0], 15);
}

#[test]
fn select_predecessors_sorted_is_deterministic_when_hashes_collide() {
    let mut blocklace = Blocklace::new();
    let mut bonds = HashMap::new();

    let shared_hash = [7u8; 32];
    let tip1 = create_same_hash_block(2, shared_hash, 9, HashSet::new());
    let tip2 = create_same_hash_block(1, shared_hash, 3, HashSet::new());

    insert(&mut blocklace, &tip1);
    insert(&mut blocklace, &tip2);

    bonds.insert(node(1), 100);
    bonds.insert(node(2), 100);

    let sorted1 = select_predecessors_sorted(&blocklace, &bonds);
    let sorted2 = select_predecessors_sorted(&blocklace, &bonds);

    assert_eq!(sorted1, sorted2);
    assert_eq!(sorted1, vec![tip2.identity.clone(), tip1.identity.clone()]);
}

/// `validator_visible_tips` returns the correct map structure with the right identities.
#[test]
fn validator_visible_tips_structure() {
    let mut blocklace = Blocklace::new();
    let mut bonds = HashMap::new();

    let tip1 = create_mock_block(1, 1, HashSet::new());
    let tip2 = create_mock_block(2, 2, HashSet::new());

    insert(&mut blocklace, &tip1);
    insert(&mut blocklace, &tip2);

    bonds.insert(node(1), 100);
    bonds.insert(node(2), 100);

    let tips = validator_visible_tips(&blocklace, &bonds);

    assert_eq!(tips.len(), 2);
    assert_eq!(tips[&node(1)], tip1.identity);
    assert_eq!(tips[&node(2)], tip2.identity);
}

// ============================================================================
// required_acknowledgements TESTS
// ============================================================================

/// Empty validator set requires 0 acknowledgements.
#[test]
fn required_acknowledgements_empty_set() {
    let bonds: HashMap<NodeId, u64> = HashMap::new();
    assert_eq!(required_acknowledgements(&bonds), 0);
}

/// n=1: single validator, threshold is 1 (the only validator must acknowledge itself).
#[test]
fn required_acknowledgements_single_validator() {
    let mut bonds = HashMap::new();
    bonds.insert(node(1), 100);
    // (2*1)/3 + 1 = 0 + 1 = 1
    assert_eq!(required_acknowledgements(&bonds), 1);
}

/// n=4 (standard 3f+1 with f=1): threshold must be 3 (2f+1 = 3).
#[test]
fn required_acknowledgements_four_validators() {
    let mut bonds = HashMap::new();
    for i in 1..=4 {
        bonds.insert(node(i), 100);
    }
    // (2*4)/3 + 1 = 2 + 1 = 3
    assert_eq!(required_acknowledgements(&bonds), 3);
}

/// n=7 (3f+1 with f=2): threshold must be 5 (2f+1 = 5).
#[test]
fn required_acknowledgements_seven_validators() {
    let mut bonds = HashMap::new();
    for i in 1..=7 {
        bonds.insert(node(i), 100);
    }
    // (2*7)/3 + 1 = 4 + 1 = 5
    assert_eq!(required_acknowledgements(&bonds), 5);
}

/// n=10 (3f+1 with f=3): threshold must be 7 (2f+1 = 7).
#[test]
fn required_acknowledgements_ten_validators() {
    let mut bonds = HashMap::new();
    for i in 1..=10 {
        bonds.insert(node(i), 100);
    }
    // (2*10)/3 + 1 = 6 + 1 = 7
    assert_eq!(required_acknowledgements(&bonds), 7);
}

/// Threshold is always strictly greater than two-thirds for all n up to 100.
#[test]
fn required_acknowledgements_always_supermajority() {
    for n in 1usize..=100 {
        let mut bonds = HashMap::new();
        for i in 0..n {
            bonds.insert(NodeId(vec![i as u8]), 100);
        }
        let threshold = required_acknowledgements(&bonds);
        // Must be strictly greater than 2n/3
        assert!(
            threshold * 3 > 2 * n,
            "n={}: threshold {} is not > 2n/3",
            n,
            threshold
        );
        // Must not exceed n (can't require more acknowledgements than validators exist)
        assert!(
            threshold <= n,
            "n={}: threshold {} exceeds validator count",
            n,
            threshold
        );
    }
}

/// Integration: select_predecessors result meets the required_acknowledgements threshold
/// when a full honest validator set is present.
#[test]
fn select_predecessors_meets_cordiality_threshold_with_full_honest_set() {
    let mut blocklace = Blocklace::new();
    let mut bonds = HashMap::new();

    // 4 validators (f=1, threshold=3)
    for i in 1..=4u8 {
        let tip = create_mock_block(i, i, HashSet::new());
        insert(&mut blocklace, &tip);
        bonds.insert(node(i), 100);
    }

    let preds = select_predecessors(&blocklace, &bonds);
    let threshold = required_acknowledgements(&bonds);

    assert!(
        preds.len() >= threshold,
        "predecessor count {} must be >= cordiality threshold {}",
        preds.len(),
        threshold
    );
}

#[test]
fn weighted_required_acknowledgements_empty_set() {
    let bonds: HashMap<NodeId, u64> = HashMap::new();
    assert_eq!(weighted_required_acknowledgements(&bonds), 0);
}

/// Equal weights: 3 validators with 100 stake each (Total 300).
/// Threshold must be strictly greater than 2/3 of 300 (which is 200), so 201.
#[test]
fn weighted_required_acknowledgements_equal_weights() {
    let mut bonds = HashMap::new();
    bonds.insert(node(1), 100);
    bonds.insert(node(2), 100);
    bonds.insert(node(3), 100);

    // (2 * 300) / 3 + 1 = 200 + 1 = 201
    assert_eq!(weighted_required_acknowledgements(&bonds), 201);
}

/// Skewed weights: A Proof-of-Stake scenario where one node has massive weight.
/// Node 1 has 90 stake. Nodes 2 and 3 have 5 stake each. (Total 100).
/// Threshold must be strictly greater than 66.66 (so 67).
#[test]
fn weighted_required_acknowledgements_skewed_weights() {
    let mut bonds = HashMap::new();
    bonds.insert(node(1), 90);
    bonds.insert(node(2), 5);
    bonds.insert(node(3), 5);

    // (2 * 100) / 3 + 1 = 66 (floor) + 1 = 67
    let threshold = weighted_required_acknowledgements(&bonds);
    assert_eq!(threshold, 67);

    // Protocol check: In this network, Node 1 CANNOT reach a supermajority
    // by combining with Node 2 and 3. Node 1 has 90, so Node 1 alone is a supermajority!
    // If Node 1 equivocates, the network halts because the remaining 10 stake
    // cannot reach the 67 threshold. This proves the math is correct.
}

/// Single massive validator: 1 validator with 1,000,000 stake.
/// Threshold must be (2,000,000 / 3) + 1 = 666,667.
#[test]
fn weighted_required_acknowledgements_large_numbers() {
    let mut bonds = HashMap::new();
    bonds.insert(node(1), 1_000_000);

    assert_eq!(weighted_required_acknowledgements(&bonds), 666_667);
}

// PENDING BLOCK BUFFER TESTS
#[test]
fn pending_buffer_resolves_single_missing_predecessor() {
    let mut blocklace = Blocklace::new();
    let mut buffer = PendingBlockBuffer::new();
    let config = dissemination_test_config();
    let mut bonds = HashMap::new();
    bonds.insert(node(1), 100);

    let genesis = create_mock_block(1, 1, HashSet::new());
    let block2 = create_mock_block(1, 2, HashSet::from([genesis.identity.clone()]));

    // block2 arrives before genesis
    buffer.buffer_block_with_missing_predecessors(block2.clone());

    // retry shouldn't insert block2 yet
    buffer.retry_buffered_blocks(&mut blocklace, &bonds, &config);
    assert_eq!(buffer.buffered_blocks.len(), 1);
    assert!(blocklace.content(&block2.identity).is_none());

    // genesis arrives
    insert(&mut blocklace, &genesis);

    // retry should now insert block2
    buffer.retry_buffered_blocks(&mut blocklace, &bonds, &config);
    assert!(buffer.buffered_blocks.is_empty());
    assert!(blocklace.content(&block2.identity).is_some());
}

#[test]
fn pending_buffer_resolves_chained_missing_predecessors() {
    let mut blocklace = Blocklace::new();
    let mut buffer = PendingBlockBuffer::new();
    let config = dissemination_test_config();
    let mut bonds = HashMap::new();
    bonds.insert(node(1), 100);

    let genesis = create_mock_block(1, 1, HashSet::new());
    let block2 = create_mock_block(1, 2, HashSet::from([genesis.identity.clone()]));
    let block3 = create_mock_block(1, 3, HashSet::from([block2.identity.clone()]));

    // block3 and block2 arrive before genesis out of order
    buffer.buffer_block_with_missing_predecessors(block3.clone());
    buffer.buffer_block_with_missing_predecessors(block2.clone());

    buffer.retry_buffered_blocks(&mut blocklace, &bonds, &config);
    assert_eq!(buffer.buffered_blocks.len(), 2);

    // genesis arrives
    insert(&mut blocklace, &genesis);

    // retry should insert both recursively
    buffer.retry_buffered_blocks(&mut blocklace, &bonds, &config);
    assert!(buffer.buffered_blocks.is_empty());
    assert!(blocklace.content(&block2.identity).is_some());
    assert!(blocklace.content(&block3.identity).is_some());
}

#[test]
fn pending_buffer_handles_out_of_order_arrival() {
    let mut blocklace = Blocklace::new();
    let mut buffer = PendingBlockBuffer::new();
    let config = dissemination_test_config();
    let mut bonds = HashMap::new();
    bonds.insert(node(1), 100);
    bonds.insert(node(2), 100);
    bonds.insert(node(3), 100);

    let genesis = create_mock_block(1, 1, HashSet::new());
    let block2a = create_mock_block(1, 2, HashSet::from([genesis.identity.clone()]));
    let block2b = create_mock_block(2, 2, HashSet::from([genesis.identity.clone()]));
    let block3 = create_mock_block(
        3,
        3,
        HashSet::from([block2a.identity.clone(), block2b.identity.clone()]),
    );

    // block 3 arrives early
    buffer.buffer_block_with_missing_predecessors(block3.clone());

    // genesis and one predecessor arrive
    insert(&mut blocklace, &genesis);
    insert(&mut blocklace, &block2a);
    buffer.retry_buffered_blocks(&mut blocklace, &bonds, &config);

    // block 3 should still be unresolved due to missing block2b
    assert_eq!(buffer.buffered_blocks.len(), 1);

    // block2b arrives
    insert(&mut blocklace, &block2b);
    buffer.retry_buffered_blocks(&mut blocklace, &bonds, &config);

    assert!(buffer.buffered_blocks.is_empty());
    assert!(blocklace.content(&block3.identity).is_some());
}

#[test]
fn pending_buffer_no_duplicate_insertion_on_repeated_retry() {
    let mut blocklace = Blocklace::new();
    let mut buffer = PendingBlockBuffer::new();
    let config = dissemination_test_config();
    let mut bonds = HashMap::new();
    bonds.insert(node(1), 100);

    let genesis = create_mock_block(1, 1, HashSet::new());
    let block2 = create_mock_block(1, 2, HashSet::from([genesis.identity.clone()]));

    buffer.buffer_block_with_missing_predecessors(block2.clone());

    insert(&mut blocklace, &genesis);

    buffer.retry_buffered_blocks(&mut blocklace, &bonds, &config);
    assert!(buffer.buffered_blocks.is_empty());

    // retry again, should be a no-op
    buffer.retry_buffered_blocks(&mut blocklace, &bonds, &config);
    assert!(blocklace.content(&block2.identity).is_some());
}

#[test]
fn pending_buffer_removes_block_already_in_blocklace() {
    let mut blocklace = Blocklace::new();
    let mut buffer = PendingBlockBuffer::new();
    let config = dissemination_test_config();
    let mut bonds = HashMap::new();
    bonds.insert(node(1), 100);

    let genesis = create_mock_block(1, 1, HashSet::new());
    let block2 = create_mock_block(1, 2, HashSet::from([genesis.identity.clone()]));

    // block2 buffered while genesis is missing
    buffer.buffer_block_with_missing_predecessors(block2.clone());

    // Both arrive directly — buffer not yet retried
    insert(&mut blocklace, &genesis);
    insert(&mut blocklace, &block2);

    // retry should still clear the buffer cleanly
    buffer.retry_buffered_blocks(&mut blocklace, &bonds, &config);
    assert!(buffer.buffered_blocks.is_empty());
}

#[test]
fn pending_buffer_drops_blocks_that_fail_consensus_validation() {
    let mut blocklace = Blocklace::new();
    let mut buffer = PendingBlockBuffer::new();
    let config = dissemination_test_config();
    let mut bonds = HashMap::new();
    bonds.insert(node(1), 100);

    let genesis = create_mock_block(1, 1, HashSet::new());
    let unbonded_child = create_mock_block(2, 2, HashSet::from([genesis.identity.clone()]));

    buffer.buffer_block_with_missing_predecessors(unbonded_child.clone());

    insert(&mut blocklace, &genesis);
    buffer.retry_buffered_blocks(&mut blocklace, &bonds, &config);

    assert!(
        buffer.buffered_blocks.is_empty(),
        "definitively invalid buffered blocks should be dropped"
    );
    assert!(
        blocklace.content(&unbonded_child.identity).is_none(),
        "replay must not bypass validation for unbonded senders"
    );
}
