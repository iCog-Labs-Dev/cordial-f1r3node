use std::collections::{HashMap, HashSet};
use crate::block::Block;
use crate::types::{BlockContent, BlockIdentity, NodeId};


// The blocklace B - a set of blocks satisfying the closure and axioms
// From definition 2.3, A blocklace B is a set of blocks subject to some invariants.
// since each block has a unique identity, we can represent the blocklace as a HashMap from BlockIdentity to Block content.

/// Invariants enforced at all times:
///  - CLOSED: ∀(i, (v, P)) ∈ B · P ⊂ dom(B)  — no dangling pointers
///  - CHAIN: all blocks from a correct node are totally ordered under  ≺
pub struct Blocklace {
    pub(crate) blocks: HashMap<BlockIdentity, BlockContent>,
}
