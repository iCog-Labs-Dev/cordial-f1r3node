use cordial_miners_core::blocklace::Blocklace;
use cordial_miners_core::consensus::cordiality::{
    acknowledges_equivocation, all_equivocations, equivocation_blocks_at_round,
    hidden_equivocations, is_cordial_block, missing_known_tips, observed_block_ids,
};
use cordial_miners_core::crypto::CryptoVerifier;
use cordial_miners_core::{Block, BlockContent, BlockIdentity, NodeId};
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

fn insert(blocklace: &mut Blocklace, block: &Block) {
    let verifier = MockVerifier;
    blocklace
        .insert(block.clone(), &verifier)
        .expect("insert failed");
}


// Test that we can detect same-round equivocation by a creator. We create two blocks by the same creator at the same round and check that they are both detected as equivocations.
#[test]
fn detects_same_round_equivocation() {
    let mut blocklace = Blocklace::new();
    let e1 = create_mock_block(1, 1, HashSet::new());
    let e2 = create_mock_block(1, 2, HashSet::new());

    insert(&mut blocklace, &e1);
    insert(&mut blocklace, &e2);

    // this cause equivocation because both blocks are created by the same creator (node 1) and they are at the same round (round 0, since they have no predecessors). We check that both blocks are detected as equivocations by calling the equivocation_blocks_at_round function and verifying that it returns both blocks.
    let equivocation = equivocation_blocks_at_round(&blocklace, &node(1), 0); 
    assert_eq!(equivocation.len(), 2);
    assert!(equivocation.contains(&e1));
    assert!(equivocation.contains(&e2));
}

// Test that different rounds by the same creator do not count as equivocation. We create two blocks by the same creator at different rounds and check that they are not detected as equivocations.
#[test]
fn different_rounds_are_not_same_round_equivocations() {
    let mut blocklace = Blocklace::new();

    let g = create_mock_block(1, 1, HashSet::new());
    insert(&mut blocklace, &g);
    let child = create_mock_block(1, 2, HashSet::from([g.identity]));
    insert(&mut blocklace, &child);

    assert!(equivocation_blocks_at_round(&blocklace, &node(1), 0).is_empty());
    assert!(equivocation_blocks_at_round(&blocklace, &node(1), 1).is_empty());

}