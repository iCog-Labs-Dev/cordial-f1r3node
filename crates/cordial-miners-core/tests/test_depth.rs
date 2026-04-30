use cordial_miners_core::blocklace::Blocklace;
/// Test file to check the depth of the tree is correct after inserting nodes.
use cordial_miners_core::consensus::round::compute_all_depths;
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
