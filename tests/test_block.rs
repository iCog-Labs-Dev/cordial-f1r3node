use blocklace::{Block, BlockContent, BlockIdentity, NodeId};


// Tests for Block struct and related functions.

// Tests for Block::is_initial()
#[test]
fn genesis_block_is_initial() {
    let genesis_block = Block {
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
    assert!(genesis_block.is_initial());
}

// Tests for Block::is_initial() with a block that has predecessors.
#[test]
fn block_with_predecessors_is_not_initial() {
    let predecessor_identity = BlockIdentity {
        content_hash: [0x00; 32],
        creator: NodeId(vec![0x00]),
        signature: vec![],
    };


    let block_content = BlockContent {
        payload: vec![],
        predecessors: std::collections::HashSet::from([predecessor_identity]),
    };
    

    let block = Block {
        identity: BlockIdentity {
            content_hash: [0xab, 0xcd, 0xef, 0x01, 0x23, 0x45, 0x67, 0x89,
                0xab, 0xcd, 0xef, 0x01, 0x23, 0x45, 0x67, 0x89,
                0xab, 0xcd, 0xef, 0x01, 0x23, 0x45, 0x67, 0x89,
                0xab, 0xcd, 0xef, 0x01, 0x23, 0x45, 0x67, 0x89],
            creator: NodeId(vec![0xab, 0xcd, 0xef, 0x12]),
            signature: vec![],
        },
        content: block_content,
    };
    assert!(!block.is_initial());
}


// Test for Block::node() function to return the creator of the block.
#[test]
fn node_returns_creator() {
    let block = Block {
        identity: BlockIdentity {
            content_hash: [0xab, 0xcd, 0xef, 0x01, 0x23, 0x45, 0x67, 0x89,
                0xab, 0xcd, 0xef, 0x01, 0x23, 0x45, 0x67, 0x89,
                0xab, 0xcd, 0xef, 0x01, 0x23, 0x45, 0x67, 0x89,
                0xab, 0xcd, 0xef, 0x01, 0x23, 0x45, 0x67, 0x89],
            creator: NodeId(vec![1]),
            signature: vec![],
        },
        content: BlockContent { payload: vec![], predecessors: std::collections::HashSet::new() },
    };
    assert_eq!(block.node(), &NodeId(vec![1]));
}

// Test for the Block::id() function to return the block's identity.
#[test]
fn id_returns_identity() {
    let block_identity = BlockIdentity {
        content_hash: [0xab, 0xcd, 0xef, 0x01, 0x23, 0x45, 0x67, 0x89,
            0xab, 0xcd, 0xef, 0x01, 0x23, 0x45, 0x67, 0x89,
            0xab, 0xcd, 0xef, 0x01, 0x23, 0x45, 0x67, 0x89,
            0xab, 0xcd, 0xef, 0x01, 0x23, 0x45, 0x67, 0x89],
        creator: NodeId(vec![1]),
        signature: vec![],
    };
    let block = Block {
        identity: block_identity.clone(),
        content: BlockContent { payload: vec![], predecessors: std::collections::HashSet::new() },
    };
    assert_eq!(block.id(), &block_identity);
}

#[test]
fn pointed_from_directs_direct_reference() {

    let block_a = Block {
        identity: BlockIdentity {
            content_hash: [0x01; 32],
            creator: NodeId(vec![1]),
            signature: vec![],
        },
        content: BlockContent { payload: vec![], predecessors: std::collections::HashSet::new() },
    };
    let block_b = Block {
        identity: BlockIdentity {
            content_hash: [0x02; 32],
            creator: NodeId(vec![2]),
            signature: vec![],
        },
        content: BlockContent { payload: vec![], predecessors: std::collections::HashSet::from([block_a.id().clone()]) },
    };
    assert!(block_a.is_pointed_from(&block_b));
}

#[test]
fn pointed_from_is_false_when_no_reference() {
    let block_a = Block {
        identity: BlockIdentity {
            content_hash: [0x01; 32],
            creator: NodeId(vec![1]),
            signature: vec![],
        },
        content: BlockContent { payload: vec![], predecessors: std::collections::HashSet::new() },
    };
    let block_b = Block {
        identity: BlockIdentity {
            content_hash: [0x02; 32],
            creator: NodeId(vec![2]),
            signature: vec![],
        },
        content: BlockContent { payload: vec![], predecessors: std::collections::HashSet::new() },
    };
    assert!(!block_a.is_pointed_from(&block_b));
}

// Equality and hashing tests
#[test]
fn same_identity_means_equal_blocks(){
    let blocka = Block {
        identity: BlockIdentity {
            content_hash: [0x01; 32],
            creator: NodeId(vec![1]),
            signature: vec![],
        },
        content: BlockContent { payload: vec![], predecessors: std::collections::HashSet::new() },
    };
    let blockb = blocka.clone();
    assert_eq!(blocka, blockb);
}

#[test]
fn different_tag_means_different_blocks(){
    let blocka = Block {
        identity: BlockIdentity {
            content_hash: [0x01; 32],
            creator: NodeId(vec![1]),
            signature: vec![],
        },
        content: BlockContent { payload: vec![], predecessors: std::collections::HashSet::new() },
    };
    let blockb = Block {
        identity: BlockIdentity {
            content_hash: [0x02; 32],
            creator: NodeId(vec![1]),
            signature: vec![],
        },
        content: BlockContent { payload: vec![], predecessors: std::collections::HashSet::new() },
    };
    assert_ne!(blocka, blockb);
}