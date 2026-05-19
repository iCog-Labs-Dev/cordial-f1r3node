use std::collections::HashSet;

use cordial_miners_core::blocklace::Blocklace;
use cordial_miners_core::consensus::approved_blocks_for_leader;
use cordial_miners_core::crypto::CryptoVerifier;
use cordial_miners_core::{Block, BlockContent, BlockIdentity, NodeId};

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

#[test]
fn approved_blocks_for_leader_returns_empty_for_unknown_leader() {
    let blocklace = Blocklace::new();
    let missing_leader = block(1, 1, HashSet::new());

    let result = approved_blocks_for_leader(&blocklace, &missing_leader.identity);

    assert!(result.is_empty());
}

#[test]
fn approved_blocks_for_leader_includes_blocks_approved_by_leader() {
    let mut blocklace = Blocklace::new();

    let leader = block(1, 1, HashSet::new());
    insert(&mut blocklace, &leader);

    let approved_a = block(2, 2, HashSet::from([leader.identity.clone()]));
    let approved_b = block(3, 3, HashSet::from([approved_a.identity.clone()]));
    let unrelated = block(4, 4, HashSet::new());
    insert(&mut blocklace, &approved_a);
    insert(&mut blocklace, &approved_b);
    insert(&mut blocklace, &unrelated);

    let result = approved_blocks_for_leader(&blocklace, &approved_b.identity);

    assert!(result.contains(&leader));
    assert!(result.contains(&approved_a));
    assert!(result.contains(&approved_b));
    assert!(!result.contains(&unrelated));
}

#[test]
fn approved_blocks_for_leader_excludes_blocks_not_approved_due_to_equivocation() {
    let mut blocklace = Blocklace::new();

    let target = block(1, 1, HashSet::new());
    let conflicting = block(1, 2, HashSet::new());
    insert(&mut blocklace, &target);
    insert(&mut blocklace, &conflicting);

    let leader = block(
        2,
        3,
        HashSet::from([target.identity.clone(), conflicting.identity.clone()]),
    );
    insert(&mut blocklace, &leader);

    let result = approved_blocks_for_leader(&blocklace, &leader.identity);

    assert!(!result.contains(&target));
    assert!(!result.contains(&conflicting));
    assert!(result.contains(&leader));
}
