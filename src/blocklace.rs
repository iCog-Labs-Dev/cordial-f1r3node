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

// Construction
impl Blocklace {
    pub fn new() -> Self {
        Self {blocks: HashMap::new()}
    }
}

// Map-view accessors based on definition 2.3
impl Blocklace {
    /// B(b) - get the content of a block by its identity.
    pub fn content(&self, id: &BlockIdentity) -> Option<&BlockContent> {
        self.blocks.get(id)
    }

    /// B[b] - get the full block (identity + content) by identity.
    pub fn get(&self, id: &BlockIdentity) -> Option<Block> {
        self.blocks.get(id).map(|content| Block {
            identity: id.clone(),
            content: content.clone(),
        })
    }
    /// B[P] - get all blocks whose ids are in the set P>
    pub fn get_set(&self, ids: &HashSet<BlockIdentity>) -> HashSet<Block> {
        ids.iter().filter_map(|id| self.get(id)).collect()
    }

    /// dom(B) - the set of all known block identities
    pub fn dom(&self) -> HashSet<&BlockIdentity> {
        self.blocks.keys().collect()
    }
}


// Insertion and Closure axiom
impl Blocklace {
    /// Insert a block into the blocklace, enforcing the closure axiom.
   pub fn insert(&mut self, block: Block) -> Result<(), String> {
    for pred_id in &block.content.predecessors {
        if !self.blocks.contains_key(pred_id) {
            return Err(format!("Closure violation: predecessor {:?} not in blocklace", pred_id));
        }
    }
    self.blocks.insert(block.identity.clone(), block.content);
    Ok(())
   }

    pub fn is_closed(&self) -> bool {
        self.blocks.values().all(|content| {
            content.predecessors.iter()
                .all(|pred_id| self.blocks.contains_key(pred_id))
        })
    }

}