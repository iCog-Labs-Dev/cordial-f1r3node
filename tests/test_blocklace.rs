use blocklace::{BlockContent, BlockIdentity, NodeId};
use blocklace::blocklace::Blocklace;
// Helpers test

fn insert(b1: &mut Blocklace, block: blocklace::Block) {
    b1.insert(block).expect("insert failed");
}

// closure axiom test: inserting a block with unknown predecessor should fail
#[test]
fn genesis_can_be_inserted_into_empty_blocklace() {
    let block = blocklace::Block {
        identity: BlockIdentity {
            content_hash: [0xab, 0xcd, 0xef, 0x01, 0x23, 0x45, 0x67, 0x89,
                0xab, 0xcd, 0xef, 0x01, 0x23, 0x45, 0x67, 0x89,
                0xab, 0xcd, 0xef, 0x01, 0x23, 0x45, 0x67, 0x89,
                0xab, 0xcd, 0xef, 0x01, 0x23, 0x45, 0x67, 0x89],
            creator: NodeId(vec![0xab, 0xcd, 0xef, 0x12]),
            signature: vec![],
        },
        content: BlockContent { payload: vec![], predecessors: std::collections::HashSet::new() },
    };
    let mut blocklace = Blocklace::new();
    insert(&mut blocklace, block);
}