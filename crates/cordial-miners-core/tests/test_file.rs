use cordial_miners_core::blocklace::Blocklace;
use cordial_miners_core::{Block, BlockContent, BlockIdentity, NodeId};
use ed25519_dalek::Verifier;
use std::collections::HashSet;
use cordial_miners_core::crypto::{CryptoVerifier};

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

// Helpers test

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
    blocklace.insert(block.clone(), &MockVerifier).expect("Failed to insert genesis block");
    // insert(&mut blocklace, block);
}
