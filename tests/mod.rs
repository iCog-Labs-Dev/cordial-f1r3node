use std::collections::HashSet;
use blocklace::types::{BlockContent, BlockIdentity, NodeId};
use blocklace::block::Block;

/// Build a NodeId from a single byte — keeps tests readable: node(1), node(2) …
pub fn node(byte: u8) -> NodeId {
    NodeId(vec![byte])
}

/// Build a matching private key stub for a node byte.
pub fn private_key(byte: u8) -> Vec<u8> {
    vec![byte]
}

/// Build a BlockIdentity without real crypto.
/// `tag` makes each identity unique — different tags = different blocks.
pub fn make_identity(creator: NodeId, tag: u8) -> BlockIdentity {
    let mut hash = [0u8; 32];
    hash[0] = creator.0[0]; // encode creator in byte 0
    hash[1] = tag;           // encode uniqueness in byte 1
    BlockIdentity {
        content_hash: hash,
        creator,
        signature: vec![tag],
    }
}

/// Build a genesis block (P = ∅) for a given creator and tag.
pub fn genesis(creator: NodeId, tag: u8) -> Block {
    Block {
        identity: make_identity(creator.clone(), tag),
        content: BlockContent {
            payload: vec![tag],
            predecessors: HashSet::new(),
        },
    }
}

/// Build a block that points to one or more predecessor blocks.
pub fn block_on(creator: NodeId, tag: u8, predecessors: Vec<&Block>) -> Block {
    let pred_ids = predecessors
        .iter()
        .map(|b| b.identity.clone())
        .collect::<HashSet<_>>();

    Block {
        identity: make_identity(creator, tag),
        content: BlockContent {
            payload: vec![tag],
            predecessors: pred_ids,
        },
    }
}