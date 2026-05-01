use cordial_miners_core::blocklace::Blocklace;
/// Test file to check the depth of the tree is correct after inserting nodes.
use cordial_miners_core::consensus::round::{
    blocks_at_depth, compute_all_depths, depth, depth_prefix, depth_suffix, is_round_cordial,
    latest_cordial_round, max_depth,
};
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
        Ok(()) // Always allow in tests
    }
}
/// Helper to create a block without the boilerplate
fn create_mock_block(creator_id: u8, hash_byte: u8, predecessors: HashSet<BlockIdentity>) -> Block {
    let mut content_hash = [0u8; 32];
    content_hash[0] = hash_byte; // Unique enough for local testing

    Block {
        identity: BlockIdentity {
            content_hash,
            creator: NodeId(vec![creator_id]),
            signature: vec![], // Not checking sigs in logic tests
        },
        content: BlockContent {
            payload: vec![],
            predecessors,
        },
    }
}

fn insert(b1: &mut Blocklace, block: cordial_miners_core::Block) {
    let verifier = MockVerifier;
    b1.insert(block, &verifier).expect("insert failed");
}

#[test]
fn test_depth() {
    let mut b1 = Blocklace::new();
    let b0 = create_mock_block(1, 1, HashSet::new());
    insert(&mut b1, b0.clone());
    let b2 = create_mock_block(2, 2, HashSet::from([b0.identity.clone()]));
    insert(&mut b1, b2.clone());
    let b3 = create_mock_block(3, 3, HashSet::from([b0.identity.clone()]));
    insert(&mut b1, b3.clone());
    let b4 = create_mock_block(
        4,
        4,
        HashSet::from([b2.identity.clone(), b3.identity.clone()]),
    );
    insert(&mut b1, b4.clone());
    let b5 = create_mock_block(5, 5, HashSet::from([b4.identity.clone()]));
    insert(&mut b1, b5.clone());

    let depths = compute_all_depths(&b1);
    assert_eq!(depths.get(&b0.identity), Some(&0));
    assert_eq!(depths.get(&b2.identity), Some(&1));
    assert_eq!(depths.get(&b3.identity), Some(&1));
    assert_eq!(depths.get(&b4.identity), Some(&2));
    assert_eq!(depths.get(&b5.identity), Some(&3));
}

#[test]
fn test_depth_single_block() {
    let mut b1 = Blocklace::new();
    let b0 = create_mock_block(1, 1, HashSet::new());
    insert(&mut b1, b0.clone());

    // Test individual depth function
    assert_eq!(depth(&b1, &b0.identity), Some(0));

    // Test compute_all_depths
    let depths = compute_all_depths(&b1);
    assert_eq!(depths.len(), 1);
    assert_eq!(depths.get(&b0.identity), Some(&0));
}

#[test]
fn test_depth_linear_chain() {
    let mut b1 = Blocklace::new();
    let b0 = create_mock_block(1, 1, HashSet::new());
    insert(&mut b1, b0.clone());
    let b1_block = create_mock_block(2, 2, HashSet::from([b0.identity.clone()]));
    insert(&mut b1, b1_block.clone());
    let b2 = create_mock_block(3, 3, HashSet::from([b1_block.identity.clone()]));
    insert(&mut b1, b2.clone());
    let b3 = create_mock_block(4, 4, HashSet::from([b2.identity.clone()]));
    insert(&mut b1, b3.clone());

    let depths = compute_all_depths(&b1);
    assert_eq!(depths.get(&b0.identity), Some(&0));
    assert_eq!(depths.get(&b1_block.identity), Some(&1));
    assert_eq!(depths.get(&b2.identity), Some(&2));
    assert_eq!(depths.get(&b3.identity), Some(&3));
}

#[test]
fn test_depth_complex_dag() {
    let mut b1 = Blocklace::new();

    // Genesis blocks
    let g1 = create_mock_block(1, 1, HashSet::new());
    insert(&mut b1, g1.clone());
    let g2 = create_mock_block(2, 2, HashSet::new());
    insert(&mut b1, g2.clone());

    // Level 1 blocks
    let l1a = create_mock_block(3, 3, HashSet::from([g1.identity.clone()]));
    insert(&mut b1, l1a.clone());
    let l1b = create_mock_block(
        4,
        4,
        HashSet::from([g1.identity.clone(), g2.identity.clone()]),
    );
    insert(&mut b1, l1b.clone());

    // Level 2 blocks
    let l2a = create_mock_block(
        5,
        5,
        HashSet::from([l1a.identity.clone(), l1b.identity.clone()]),
    );
    insert(&mut b1, l2a.clone());
    let l2b = create_mock_block(6, 6, HashSet::from([l1b.identity.clone()]));
    insert(&mut b1, l2b.clone());

    let depths = compute_all_depths(&b1);
    assert_eq!(depths.get(&g1.identity), Some(&0));
    assert_eq!(depths.get(&g2.identity), Some(&0));
    assert_eq!(depths.get(&l1a.identity), Some(&1));
    assert_eq!(depths.get(&l1b.identity), Some(&1));
    assert_eq!(depths.get(&l2a.identity), Some(&2));
    assert_eq!(depths.get(&l2b.identity), Some(&2));
}

#[test]
fn test_blocks_at_depth() {
    let mut b1 = Blocklace::new();

    // Genesis blocks (depth 0)
    let g1 = create_mock_block(1, 1, HashSet::new());
    insert(&mut b1, g1.clone());
    let g2 = create_mock_block(2, 2, HashSet::new());
    insert(&mut b1, g2.clone());

    // Level 1 blocks (depth 1)
    let l1a = create_mock_block(3, 3, HashSet::from([g1.identity.clone()]));
    insert(&mut b1, l1a.clone());
    let l1b = create_mock_block(
        4,
        4,
        HashSet::from([g1.identity.clone(), g2.identity.clone()]),
    );
    insert(&mut b1, l1b.clone());

    // Level 2 blocks (depth 2)
    let l2a = create_mock_block(
        5,
        5,
        HashSet::from([l1a.identity.clone(), l1b.identity.clone()]),
    );
    insert(&mut b1, l2a.clone());

    // Test blocks_at_depth for each depth
    let depth_0_blocks = blocks_at_depth(&b1, 0);
    assert_eq!(depth_0_blocks.len(), 2);
    assert!(depth_0_blocks.contains(&g1));
    assert!(depth_0_blocks.contains(&g2));

    let depth_1_blocks = blocks_at_depth(&b1, 1);
    assert_eq!(depth_1_blocks.len(), 2);
    assert!(depth_1_blocks.contains(&l1a));
    assert!(depth_1_blocks.contains(&l1b));

    let depth_2_blocks = blocks_at_depth(&b1, 2);
    assert_eq!(depth_2_blocks.len(), 1);
    assert!(depth_2_blocks.contains(&l2a));

    // Test non-existent depth
    let depth_3_blocks = blocks_at_depth(&b1, 3);
    assert_eq!(depth_3_blocks.len(), 0);
}

#[test]
fn test_depth_prefix_and_suffix() {
    let mut b1 = Blocklace::new();

    // Create blocks at depths 0, 1, 2
    let g1 = create_mock_block(1, 1, HashSet::new());
    insert(&mut b1, g1.clone());
    let l1a = create_mock_block(3, 3, HashSet::from([g1.identity.clone()]));
    insert(&mut b1, l1a.clone());
    let l2a = create_mock_block(5, 5, HashSet::from([l1a.identity.clone()]));
    insert(&mut b1, l2a.clone());

    // Test depth_prefix B(d)
    let prefix_0 = depth_prefix(&b1, 0);
    assert_eq!(prefix_0.len(), 1);
    assert!(prefix_0.contains(&g1));

    let prefix_1 = depth_prefix(&b1, 1);
    assert_eq!(prefix_1.len(), 2);
    assert!(prefix_1.contains(&g1));
    assert!(prefix_1.contains(&l1a));

    let prefix_2 = depth_prefix(&b1, 2);
    assert_eq!(prefix_2.len(), 3);
    assert!(prefix_2.contains(&g1));
    assert!(prefix_2.contains(&l1a));
    assert!(prefix_2.contains(&l2a));

    // Test depth_suffix B̄(d)
    let suffix_0 = depth_suffix(&b1, 0);
    assert_eq!(suffix_0.len(), 2);
    assert!(suffix_0.contains(&l1a));
    assert!(suffix_0.contains(&l2a));

    let suffix_1 = depth_suffix(&b1, 1);
    assert_eq!(suffix_1.len(), 1);
    assert!(suffix_1.contains(&l2a));

    let suffix_2 = depth_suffix(&b1, 2);
    assert_eq!(suffix_2.len(), 0);

    // Verify mathematical properties: B(d) ∪ B̄(d) = all_blocks
    let all_blocks = depth_prefix(&b1, 2);
    let prefix_1 = depth_prefix(&b1, 1);
    let suffix_1 = depth_suffix(&b1, 1);
    let union: HashSet<_> = prefix_1.union(&suffix_1).cloned().collect();
    assert_eq!(all_blocks, union);

    // Verify B(d) ∩ B̄(d) = ∅
    let intersection: HashSet<_> = prefix_1.intersection(&suffix_1).cloned().collect();
    assert!(intersection.is_empty());
}

#[test]
fn test_max_depth() {
    let mut b1 = Blocklace::new();

    // Empty blocklace
    assert_eq!(max_depth(&b1), None);

    // Single block
    let g1 = create_mock_block(1, 1, HashSet::new());
    insert(&mut b1, g1.clone());
    assert_eq!(max_depth(&b1), Some(0));

    // Linear chain
    let l1a = create_mock_block(3, 3, HashSet::from([g1.identity.clone()]));
    insert(&mut b1, l1a.clone());
    let l2a = create_mock_block(5, 5, HashSet::from([l1a.identity.clone()]));
    insert(&mut b1, l2a.clone());
    assert_eq!(max_depth(&b1), Some(2));
}

#[test]
fn test_is_round_cordial() {
    let mut b1 = Blocklace::new();

    // Create blocks with different creators
    let g1 = create_mock_block(1, 1, HashSet::new());
    insert(&mut b1, g1.clone());
    let g2 = create_mock_block(2, 2, HashSet::new());
    insert(&mut b1, g2.clone());
    let g3 = create_mock_block(3, 3, HashSet::new());
    insert(&mut b1, g3.clone());

    // Test with n=3, f=1
    assert!(is_round_cordial(&b1, 0, 3, 1));

    // Test with n=4, f=1
    assert!(is_round_cordial(&b1, 0, 4, 1));

    // Test with n=5, f=1
    assert!(!is_round_cordial(&b1, 0, 5, 1));

    // Test with n=7, f=2
    assert!(!is_round_cordial(&b1, 0, 7, 2));

    // Add more blocks to reach supermajority
    let g4 = create_mock_block(4, 4, HashSet::new());
    insert(&mut b1, g4.clone());
    let g5 = create_mock_block(5, 5, HashSet::new());
    insert(&mut b1, g5.clone());

    assert!(is_round_cordial(&b1, 0, 7, 2));
}

#[test]
fn test_latest_cordial_round() {
    let mut b1 = Blocklace::new();

    // Empty blocklace
    assert_eq!(latest_cordial_round(&b1, 3, 1), None);

    // Create blocks across multiple rounds
    let g1 = create_mock_block(1, 1, HashSet::new());
    insert(&mut b1, g1.clone());
    let g2 = create_mock_block(2, 2, HashSet::new());
    insert(&mut b1, g2.clone());

    // Round 1 blocks
    let l1a = create_mock_block(3, 3, HashSet::from([g1.identity.clone()]));
    insert(&mut b1, l1a.clone());
    let l1b = create_mock_block(
        4,
        4,
        HashSet::from([g1.identity.clone(), g2.identity.clone()]),
    );
    insert(&mut b1, l1b.clone());

    // Round 2 blocks (only one creator - not cordial)
    let l2a = create_mock_block(3, 5, HashSet::from([l1a.identity.clone()]));
    insert(&mut b1, l2a.clone());

    // Test with n=3, f=1
    assert_eq!(latest_cordial_round(&b1, 3, 1), None);

    // Add more blocks to make round 1 cordial
    let l1c = create_mock_block(5, 6, HashSet::from([g2.identity.clone()]));
    insert(&mut b1, l1c.clone());

    assert_eq!(latest_cordial_round(&b1, 3, 1), Some(1));

    let l2b = create_mock_block(1, 7, HashSet::from([l1b.identity.clone()]));
    insert(&mut b1, l2b.clone());
    let l2c = create_mock_block(2, 8, HashSet::from([l1c.identity.clone()]));
    insert(&mut b1, l2c.clone());
    assert_eq!(latest_cordial_round(&b1, 3, 1), Some(2));
}

#[test]
fn test_depth_nonexistent_block() {
    let b1 = Blocklace::new();
    let nonexistent_id = BlockIdentity {
        content_hash: [99; 32],
        creator: NodeId(vec![99]),
        signature: vec![],
    };

    assert_eq!(depth(&b1, &nonexistent_id), None);
}

#[test]
fn test_supermajority_edge_cases() {
    let mut b1 = Blocklace::new();

    let g1 = create_mock_block(1, 1, HashSet::new());
    insert(&mut b1, g1.clone());

    assert!(is_round_cordial(&b1, 0, 1, 0));

    let g2 = create_mock_block(2, 2, HashSet::new());
    insert(&mut b1, g2.clone());

    assert!(is_round_cordial(&b1, 0, 2, 0));
    assert!(is_round_cordial(&b1, 0, 2, 1));
}
