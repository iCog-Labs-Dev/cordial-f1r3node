use cordial_miners_core::blocklace::Blocklace;
use cordial_miners_core::{Block, BlockContent, BlockIdentity, NodeId};
use std::collections::HashSet;
use cordial_miners_core::crypto::{CryptoVerifier, Secp256k1Scheme};
// Helpers test
// Mock Verifier 
struct MockVerifier;

impl CryptoVerifier for MockVerifier {
    type Error = String;
    fn verify_block(
        &self, 
        _content: &BlockContent, 
        _sig: &[u8], 
        _creator: &NodeId
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

// closure axiom test: inserting a block with unknown predecessor should fail
#[test]
fn genesis_can_be_inserted_into_empty_blocklace() {
    let block = cordial_miners_core::Block {
        identity: BlockIdentity {
            content_hash: [
                0xab, 0xcd, 0xef, 0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef, 0x01, 0x23, 0x45,
                0x67, 0x89, 0xab, 0xcd, 0xef, 0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef, 0x01,
                0x23, 0x45, 0x67, 0x89,
            ],
            creator: NodeId(vec![0xab, 0xcd, 0xef, 0x12]),
            signature: vec![],
        },
        content: BlockContent {
            payload: vec![],
            predecessors: std::collections::HashSet::new(),
        },
    };
    let mut blocklace = Blocklace::new();
    insert(&mut blocklace, block);
}

#[test]
fn block_with_known_predecessor_can_be_inserted() {
    let mut b1 = Blocklace::new();

    let g = cordial_miners_core::Block {
        identity: BlockIdentity {
            content_hash: [
                0xab, 0xcd, 0xef, 0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef, 0x01, 0x23, 0x45,
                0x67, 0x89, 0xab, 0xcd, 0xef, 0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef, 0x01,
                0x23, 0x45, 0x67, 0x89,
            ],
            creator: NodeId(vec![0xab, 0xcd, 0xef, 0x12]),
            signature: vec![],
        },
        content: BlockContent {
            payload: vec![],
            predecessors: std::collections::HashSet::new(),
        },
    };
    insert(&mut b1, g.clone());

    let b2 = cordial_miners_core::Block {
        identity: BlockIdentity {
            content_hash: [
                0x12, 0x34, 0x56, 0x78, 0x9a, 0xbc, 0xde, 0xf0, 0x12, 0x34, 0x56, 0x78, 0x9a, 0xbc,
                0xde, 0xf0, 0x12, 0x34, 0x56, 0x78, 0x9a, 0xbc, 0xde, 0xf0, 0x12, 0x34, 0x56, 0x78,
                0x9a, 0xbc, 0xde, 0xf0,
            ],
            creator: NodeId(vec![0xab, 0xcd, 0xef, 0x34]),
            signature: vec![],
        },
        content: BlockContent {
            payload: vec![],
            predecessors: [g.identity.clone()].iter().cloned().collect(),
        },
    };
    insert(&mut b1, b2);
    assert!(b1.is_closed());
}

#[test]
fn inserting_block_with_unknown_predecessor_fails() {
    let mut b1 = Blocklace::new();
    let unknown_pred = BlockIdentity {
        content_hash: [
            0x12, 0x34, 0x56, 0x78, 0x9a, 0xbc, 0xde, 0xf0, 0x12, 0x34, 0x56, 0x78, 0x9a, 0xbc,
            0xde, 0xf0, 0x12, 0x34, 0x56, 0x78, 0x9a, 0xbc, 0xde, 0xf0, 0x12, 0x34, 0x56, 0x78,
            0x9a, 0xbc, 0xde, 0xf0,
        ],
        creator: NodeId(vec![0xab, 0xcd, 0xef, 0x34]),
        signature: vec![],
    };
    let block_with_unknown_pred = Block {
        identity: BlockIdentity {
            content_hash: [
                0x12, 0x34, 0x56, 0x78, 0x9a, 0xbc, 0xde, 0xf0, 0x12, 0x34, 0x56, 0x78, 0x9a, 0xbc,
                0xde, 0xf0, 0x12, 0x34, 0x56, 0x78, 0x9a, 0xbc, 0xde, 0xf0, 0x12, 0x34, 0x56, 0x78,
                0x9a, 0xbc, 0xde, 0xf1,
            ],
            creator: NodeId(vec![0xab, 0xcd, 0xef, 0x34]),
            signature: vec![],
        },
        content: BlockContent {
            payload: vec![],
            predecessors: [unknown_pred].iter().cloned().collect(),
        },
    };
    let verifier = Secp256k1Scheme;
    let result = b1.insert(block_with_unknown_pred, &verifier);
    assert!(result.is_err())
}

// Map-view accessors
#[test]
fn content_returns_none_for_unknown_id() {
    let b1 = Blocklace::new();
    let block = Block {
        identity: BlockIdentity {
            content_hash: [
                0x12, 0x34, 0x56, 0x78, 0x9a, 0xbc, 0xde, 0xf0, 0x12, 0x34, 0x56, 0x78, 0x9a, 0xbc,
                0xde, 0xf0, 0x12, 0x34, 0x56, 0x78, 0x9a, 0xbc, 0xde, 0xf0, 0x12, 0x34, 0x56, 0x78,
                0x9a, 0xbc, 0xde, 0xf1,
            ],
            creator: NodeId(vec![0xab, 0xcd, 0xef, 0x34]),
            signature: vec![],
        },
        content: BlockContent {
            payload: vec![],
            predecessors: std::collections::HashSet::new(),
        },
    };
    assert!(b1.content(&block.identity).is_none())
}

#[test]
fn get_returns_full_block_after_insert() {
    let mut b1 = Blocklace::new();
    let block = Block {
        identity: BlockIdentity {
            content_hash: [
                0xab, 0xcd, 0xef, 0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef, 0x01, 0x23, 0x45,
                0x67, 0x89, 0xab, 0xcd, 0xef, 0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef, 0x01,
                0x23, 0x45, 0x67, 0x89,
            ],
            creator: NodeId(vec![0xab, 0xcd, 0xef, 0x12]),
            signature: vec![],
        },
        content: BlockContent {
            payload: vec![1, 2, 3],
            predecessors: std::collections::HashSet::new(),
        },
    };
    insert(&mut b1, block.clone());

    let retrieved = b1.get(&block.identity);
    assert!(retrieved.is_some());
    let retrieved = retrieved.unwrap();
    assert_eq!(retrieved.identity, block.identity);
    assert_eq!(retrieved.content.payload, block.content.payload);
    assert_eq!(retrieved.content.predecessors, block.content.predecessors);
}

#[test]
fn get_set_returns_all_requested_blocks() {
    let mut b1 = Blocklace::new();

    let g = Block {
        identity: BlockIdentity {
            content_hash: [
                0xab, 0xcd, 0xef, 0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef, 0x01, 0x23, 0x45,
                0x67, 0x89, 0xab, 0xcd, 0xef, 0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef, 0x01,
                0x23, 0x45, 0x67, 0x89,
            ],
            creator: NodeId(vec![0xab, 0xcd, 0xef, 0x12]),
            signature: vec![],
        },
        content: BlockContent {
            payload: vec![],
            predecessors: std::collections::HashSet::new(),
        },
    };
    insert(&mut b1, g.clone());

    let b2 = Block {
        identity: BlockIdentity {
            content_hash: [
                0x12, 0x34, 0x56, 0x78, 0x9a, 0xbc, 0xde, 0xf0, 0x12, 0x34, 0x56, 0x78, 0x9a, 0xbc,
                0xde, 0xf0, 0x12, 0x34, 0x56, 0x78, 0x9a, 0xbc, 0xde, 0xf0, 0x12, 0x34, 0x56, 0x78,
                0x9a, 0xbc, 0xde, 0xf0,
            ],
            creator: NodeId(vec![0xab, 0xcd, 0xef, 0x34]),
            signature: vec![],
        },
        content: BlockContent {
            payload: vec![4, 5],
            predecessors: [g.identity.clone()].iter().cloned().collect(),
        },
    };
    insert(&mut b1, b2.clone());

    let ids: std::collections::HashSet<BlockIdentity> = [g.identity.clone(), b2.identity.clone()]
        .iter()
        .cloned()
        .collect();
    let result = b1.get_set(&ids);

    assert_eq!(result.len(), 2);
    assert!(result.iter().any(|b| b.identity == g.identity));
    assert!(result.iter().any(|b| b.identity == b2.identity));
}

#[test]
fn dom_contains_all_inserted_identities() {
    let mut b1 = Blocklace::new();

    let g = Block {
        identity: BlockIdentity {
            content_hash: [
                0xab, 0xcd, 0xef, 0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef, 0x01, 0x23, 0x45,
                0x67, 0x89, 0xab, 0xcd, 0xef, 0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef, 0x01,
                0x23, 0x45, 0x67, 0x89,
            ],
            creator: NodeId(vec![0xab, 0xcd, 0xef, 0x12]),
            signature: vec![],
        },
        content: BlockContent {
            payload: vec![],
            predecessors: std::collections::HashSet::new(),
        },
    };
    insert(&mut b1, g.clone());

    let b2 = Block {
        identity: BlockIdentity {
            content_hash: [
                0x12, 0x34, 0x56, 0x78, 0x9a, 0xbc, 0xde, 0xf0, 0x12, 0x34, 0x56, 0x78, 0x9a, 0xbc,
                0xde, 0xf0, 0x12, 0x34, 0x56, 0x78, 0x9a, 0xbc, 0xde, 0xf0, 0x12, 0x34, 0x56, 0x78,
                0x9a, 0xbc, 0xde, 0xf0,
            ],
            creator: NodeId(vec![0xab, 0xcd, 0xef, 0x34]),
            signature: vec![],
        },
        content: BlockContent {
            payload: vec![],
            predecessors: [g.identity.clone()].iter().cloned().collect(),
        },
    };
    insert(&mut b1, b2.clone());

    let dom = b1.dom();
    assert_eq!(dom.len(), 2);
    assert!(dom.contains(&g.identity));
    assert!(dom.contains(&b2.identity));
}

/// Logic tests in Observe and Closure Closure
#[test]
fn test_closure_axiom_enforcement() {
    let mut bl = Blocklace::new();
    let g = create_mock_block(1, 0xAA, HashSet::new());
    insert(&mut bl, g.clone());

    // valid insertion
    let mut preds = HashSet::new();
    preds.insert(g.identity.clone());
    let b1 = create_mock_block(2, 0x88, preds);
    insert(&mut bl, b1);

    // Invalid insertion
    let unknown_id = create_mock_block(9, 0xFF, HashSet::new()).identity;
    let mut bad_preds = HashSet::new();
    bad_preds.insert(unknown_id);
    let rouge_block = create_mock_block(3, 0xCC, bad_preds);
    let verifier = Secp256k1Scheme;
    assert!(
        bl.insert(rouge_block, &verifier).is_err(),
        "Should fail due to missing predecessor"
    );
}

/// New observe and Determinism Tests
#[test]
fn test_observe_includes_self_and_ancestors() {
    let mut bl = Blocklace::new();

    let g = create_mock_block(1, 0x01, HashSet::new());
    let mut p1 = HashSet::new();
    p1.insert(g.identity.clone());

    let a = create_mock_block(1, 0x002, p1);
    let mut p2 = HashSet::new();
    p2.insert(a.identity.clone());

    let b = create_mock_block(2, 0x003, p2);

    insert(&mut bl, g.clone());
    insert(&mut bl, a.clone());
    insert(&mut bl, b.clone());

    let observation = bl.observe(&b.identity);

    assert_eq!(observation.len(), 3);
    assert!(
        observation.contains(&b.identity),
        "observation must be inclusive"
    );
    assert!(
        observation.contains(&g.identity),
        "observation must find transitive ancestor"
    );
}

#[test]
fn test_observe_isolates_parallet_forks() {
    let mut bl = Blocklace::new();

    let g = create_mock_block(1, 0x00, HashSet::new());
    insert(&mut bl, g.clone());

    // Fork A
    let mut p_a = HashSet::new();
    p_a.insert(g.identity.clone());
    let a1 = create_mock_block(1, 0xA1, p_a);
    insert(&mut bl, a1.clone());

    // Fork B
    let mut p_b = HashSet::new();
    p_b.insert(g.identity.clone());
    let b1 = create_mock_block(2, 0x81, p_b);
    insert(&mut bl, b1.clone());

    let obs_a = bl.observe(&a1.identity.clone());

    assert!(obs_a.contains(&a1.identity)); // INCLUSIVE: MEANING CONTAINS ITSELF TOO!
    assert!(obs_a.contains(&g.identity));
    assert!(
        !obs_a.contains(&b1.identity),
        "Should not see blocks on parallel paths"
    )
}

#[test]
fn test_observe_determinism_across_reconstruction() {
    let mut bl1 = Blocklace::new();
    let mut bl2 = Blocklace::new();

    let g = create_mock_block(1, 0x01, HashSet::new());
    let a = create_mock_block(1, 0x02, HashSet::new());

    let mut p_c = HashSet::new();
    p_c.insert(g.identity.clone());
    p_c.insert(a.identity.clone());
    let c = create_mock_block(2, 0x03, p_c);

    // Construction Order 1: G then A then C
    insert(&mut bl1, g.clone());
    insert(&mut bl1, a.clone());
    insert(&mut bl1, c.clone());

    // Construction Order 2: A then G then C
    insert(&mut bl2, a.clone());
    insert(&mut bl2, g.clone());
    insert(&mut bl2, c.clone());

    let list1: Vec<_> = bl1.observe(&c.identity).into_iter().collect();
    let list2: Vec<_> = bl2.observe(&c.identity).into_iter().collect();

    assert_eq!(
        list1, list2,
        "BTreeSet iteration must be identical regardless of insertion order"
    )
}
