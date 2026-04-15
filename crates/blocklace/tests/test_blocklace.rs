use blocklace::{Block, BlockContent, BlockIdentity, NodeId};
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


#[test]
fn block_with_known_predecessor_can_be_inserted() {
    let mut b1 = Blocklace::new();

    let g = blocklace::Block {
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
    insert(&mut b1, g.clone());

    let b2 = blocklace::Block {
        identity: BlockIdentity {
            content_hash: [0x12, 0x34, 0x56, 0x78, 0x9a, 0xbc, 0xde, 0xf0,
                0x12, 0x34, 0x56, 0x78, 0x9a, 0xbc, 0xde, 0xf0,
                0x12, 0x34, 0x56, 0x78, 0x9a, 0xbc, 0xde, 0xf0,
                0x12, 0x34, 0x56, 0x78, 0x9a, 0xbc, 0xde, 0xf0],
            creator: NodeId(vec![0xab, 0xcd, 0xef, 0x34]),
            signature: vec![],
        },
        content: BlockContent { payload: vec![], predecessors: [g.identity.clone()].iter().cloned().collect() },
    };
    insert(&mut b1, b2);
    assert!(b1.is_closed());
}

#[test]
fn inserting_block_with_unknown_predecessor_fails() {
    let mut b1 = Blocklace::new();
    let unknown_pred = BlockIdentity {
        content_hash: [0x12, 0x34, 0x56, 0x78, 0x9a, 0xbc, 0xde, 0xf0,
            0x12, 0x34, 0x56, 0x78, 0x9a, 0xbc, 0xde, 0xf0, 0x12, 0x34, 0x56, 0x78, 0x9a, 0xbc, 0xde, 0xf0,
            0x12, 0x34, 0x56, 0x78, 0x9a, 0xbc, 0xde, 0xf0],
        creator: NodeId(vec![0xab, 0xcd, 0xef, 0x34]),
        signature: vec![],
    };
    let block_with_unknown_pred = Block {
        identity: BlockIdentity {   
            content_hash: [0x12, 0x34, 0x56, 0x78, 0x9a, 0xbc, 0xde, 0xf0,
                0x12, 0x34, 0x56, 0x78, 0x9a, 0xbc, 0xde, 0xf0,
                0x12, 0x34, 0x56, 0x78, 0x9a, 0xbc, 0xde, 0xf0,
                0x12, 0x34, 0x56, 0x78, 0x9a, 0xbc, 0xde, 0xf1],
            creator: NodeId(vec![0xab, 0xcd, 0xef, 0x34]),
            signature: vec![],
        },
        content: BlockContent { payload: vec![], predecessors: [unknown_pred].iter().cloned().collect() },
    };
    let result = b1.insert(block_with_unknown_pred);
    assert!(result.is_err())
}

// Map-view accessors
#[test]
fn content_returns_none_for_unknown_id() {
    let b1 = Blocklace::new();
    let block = Block {
        identity: BlockIdentity { 
            content_hash: [0x12, 0x34, 0x56, 0x78, 0x9a, 0xbc, 0xde, 0xf0,
                0x12, 0x34, 0x56, 0x78, 0x9a, 0xbc, 0xde, 0xf0,
                0x12, 0x34, 0x56, 0x78, 0x9a, 0xbc, 0xde, 0xf0,
                0x12, 0x34, 0x56, 0x78, 0x9a, 0xbc, 0xde, 0xf1],
            creator: NodeId(vec![0xab, 0xcd, 0xef, 0x34]),
            signature: vec![],
    },
    content: BlockContent { payload: vec![],predecessors: std::collections::HashSet::new() }, 
};
    assert!(b1.content(&block.identity).is_none())
}

#[test]
fn get_returns_full_block_after_insert() {
    let mut b1 = Blocklace::new();
    let block = Block {
        identity: BlockIdentity {
            content_hash: [0xab, 0xcd, 0xef, 0x01, 0x23, 0x45, 0x67, 0x89,
                0xab, 0xcd, 0xef, 0x01, 0x23, 0x45, 0x67, 0x89,
                0xab, 0xcd, 0xef, 0x01, 0x23, 0x45, 0x67, 0x89,
                0xab, 0xcd, 0xef, 0x01, 0x23, 0x45, 0x67, 0x89],
            creator: NodeId(vec![0xab, 0xcd, 0xef, 0x12]),
            signature: vec![],
        },
        content: BlockContent { payload: vec![1, 2, 3], predecessors: std::collections::HashSet::new() },
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
            content_hash: [0xab, 0xcd, 0xef, 0x01, 0x23, 0x45, 0x67, 0x89,
                0xab, 0xcd, 0xef, 0x01, 0x23, 0x45, 0x67, 0x89,
                0xab, 0xcd, 0xef, 0x01, 0x23, 0x45, 0x67, 0x89,
                0xab, 0xcd, 0xef, 0x01, 0x23, 0x45, 0x67, 0x89],
            creator: NodeId(vec![0xab, 0xcd, 0xef, 0x12]),
            signature: vec![],
        },
        content: BlockContent { payload: vec![], predecessors: std::collections::HashSet::new() },
    };
    insert(&mut b1, g.clone());

    let b2 = Block {
        identity: BlockIdentity {
            content_hash: [0x12, 0x34, 0x56, 0x78, 0x9a, 0xbc, 0xde, 0xf0,
                0x12, 0x34, 0x56, 0x78, 0x9a, 0xbc, 0xde, 0xf0,
                0x12, 0x34, 0x56, 0x78, 0x9a, 0xbc, 0xde, 0xf0,
                0x12, 0x34, 0x56, 0x78, 0x9a, 0xbc, 0xde, 0xf0],
            creator: NodeId(vec![0xab, 0xcd, 0xef, 0x34]),
            signature: vec![],
        },
        content: BlockContent { payload: vec![4, 5], predecessors: [g.identity.clone()].iter().cloned().collect() },
    };
    insert(&mut b1, b2.clone());

    let ids: std::collections::HashSet<BlockIdentity> =
        [g.identity.clone(), b2.identity.clone()].iter().cloned().collect();
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
            content_hash: [0xab, 0xcd, 0xef, 0x01, 0x23, 0x45, 0x67, 0x89,
                0xab, 0xcd, 0xef, 0x01, 0x23, 0x45, 0x67, 0x89,
                0xab, 0xcd, 0xef, 0x01, 0x23, 0x45, 0x67, 0x89,
                0xab, 0xcd, 0xef, 0x01, 0x23, 0x45, 0x67, 0x89],
            creator: NodeId(vec![0xab, 0xcd, 0xef, 0x12]),
            signature: vec![],
        },
        content: BlockContent { payload: vec![], predecessors: std::collections::HashSet::new() },
    };
    insert(&mut b1, g.clone());

    let b2 = Block {
        identity: BlockIdentity {
            content_hash: [0x12, 0x34, 0x56, 0x78, 0x9a, 0xbc, 0xde, 0xf0,
                0x12, 0x34, 0x56, 0x78, 0x9a, 0xbc, 0xde, 0xf0,
                0x12, 0x34, 0x56, 0x78, 0x9a, 0xbc, 0xde, 0xf0,
                0x12, 0x34, 0x56, 0x78, 0x9a, 0xbc, 0xde, 0xf0],
            creator: NodeId(vec![0xab, 0xcd, 0xef, 0x34]),
            signature: vec![],
        },
        content: BlockContent { payload: vec![], predecessors: [g.identity.clone()].iter().cloned().collect() },
    };
    insert(&mut b1, b2.clone());

    let dom = b1.dom();
    assert_eq!(dom.len(), 2);
    assert!(dom.contains(&g.identity));
    assert!(dom.contains(&b2.identity));
}