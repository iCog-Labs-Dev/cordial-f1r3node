use std::collections::HashSet;
use super::identity_id::BlockIdentity;

/// C = (v, P) — the block content that gets hashed to form the block identity.
///
/// From the paper (§2.2):
///   The block content is a pair C = (v, P) of an arbitrary value v
///   (the block payload) and a set P of the block identities of the
///   predecessors of b.
#[derive(Debug, Clone)]
pub struct BlockContent {
    /// The arbitrary payload 'v'.
    /// Can encode an operation, a list of operations, transactions, etc.
    pub payload: Vec<u8>,

    /// 'P' — the set of predecessor block identities.
    /// If P = ∅ then the block is initial (genesis).
    pub predecessors: HashSet<BlockIdentity>,
}