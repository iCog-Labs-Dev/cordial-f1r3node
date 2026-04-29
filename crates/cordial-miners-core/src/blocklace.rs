use crate::block::Block;
use crate::crypto::CryptoVerifier;
use crate::types::{BlockContent, BlockIdentity, NodeId};
use std::collections::{BTreeSet, HashMap, HashSet, VecDeque};

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
        Self {
            blocks: HashMap::new(),
        }
    }
}

impl Default for Blocklace {
    fn default() -> Self {
        Self::new()
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
    pub fn insert<V: CryptoVerifier>(&mut self, block: Block, verifier: &V) -> Result<(), String> {
        // 1. Signature Verification (Issue 2)
        verifier
            .verify_block(
                &block.content,
                &block.identity.signature,
                &block.identity.creator,
            )
            .map_err(|e| format!("Invalid signature: {:?}", e))?;

        // 2. Closure Axiom Enforcement (Issue 1)
        for pred_id in &block.content.predecessors {
            if !self.blocks.contains_key(pred_id) {
                return Err(format!(
                    "Closure violation: predecessor {pred_id:?} not in blocklace"
                ));
            }
        }

        // 3. Commit to state
        self.blocks.insert(block.identity.clone(), block.content);
        Ok(())
    }
    // pub fn insert(&mut self, block: Block) -> Result<(), String> {
    //     for pred_id in &block.content.predecessors {
    //         if !self.blocks.contains_key(pred_id) {
    //             return Err(format!(
    //                 "Closure violation: predecessor {pred_id:?} not in blocklace"
    //             ));
    //         }
    //     }
    //     self.blocks.insert(block.identity.clone(), block.content);
    //     Ok(())
    // }

    pub fn is_closed(&self) -> bool {
        self.blocks.values().all(|content| {
            content
                .predecessors
                .iter()
                .all(|pred_id| self.blocks.contains_key(pred_id))
        })
    }
}

// Pointed relation based on definition 2.2
impl Blocklace {
    /// <-b - direcet predecessors of block b
    /// {a | a <- b} the blocks directly pointed to by b
    pub fn predecessors(&self, id: &BlockIdentity) -> HashSet<Block> {
        match self.content(id) {
            None => HashSet::new(),
            Some(content) => self.get_set(&content.predecessors),
        }
    }
}

impl Blocklace {
    /// <b - all ancestors of b (transitive closure of <-) Not including b itself
    /// Computed by iterative DFS over the prodecessor graph
    /// Termination is guaranteed by the closure axiom, which ensures no cycles
    pub fn ancestors(&self, id: BlockIdentity) -> HashSet<Block> {
        let mut visited = HashSet::new();
        let mut queue: Vec<BlockIdentity> = vec![id.clone()];

        while let Some(current_id) = queue.pop() {
            if let Some(content) = self.content(&current_id) {
                for pred_id in &content.predecessors {
                    if visited.insert(pred_id.clone()) {
                        queue.push(pred_id.clone());
                    }
                }
            }
        }
        visited.iter().filter_map(|id| self.get(id)).collect()
    }

    pub fn observe(&self, from: &BlockIdentity) -> BTreeSet<BlockIdentity> {
        let mut visited = BTreeSet::new();
        let mut queue = VecDeque::new();

        // Start from the block itself (inclusive closure)
        queue.push_back(from.clone());
        visited.insert(from.clone());

        while let Some(current_id) = queue.pop_front() {
            if let Some(content) = self.content(&current_id) {
                for pred_id in &content.predecessors {
                    // BTreeSet.insert returns false if the item was already present
                    if visited.insert(pred_id.clone()) {
                        queue.push_back(pred_id.clone());
                    }
                }
            }
        }
        visited
    }

    /// ⪯b — ancestors of b including b itself
    /// This is downward closure used heavly throughout the paper, so we provide a direct method for it.
    pub fn ancestors_inclusive(&self, id: &BlockIdentity) -> HashSet<Block> {
        let mut result = self.ancestors(id.clone());
        if let Some(block) = self.get(id) {
            result.insert(block);
        }
        result
    }

    /// <S - all blocks that are ancestors of any block in S
    pub fn ancestors_of_set(&self, ids: &HashSet<BlockIdentity>) -> HashSet<Block> {
        ids.iter()
            .flat_map(|id| self.ancestors(id.clone()))
            .collect()
    }

    /// Check if a < b - a is somewhere in b's ancestry
    pub fn precedes(&self, a: &BlockIdentity, b: &BlockIdentity) -> bool {
        self.ancestors(b.clone())
            .iter()
            .any(|block| &block.identity == a)
    }

    /// Check if a ⪯ b - a  preceeds b or is equal to b
    pub fn preceedes_or_equals(&self, a: &BlockIdentity, b: &BlockIdentity) -> bool {
        a == b || self.precedes(a, b)
    }
}

impl Blocklace {
    /// Returns all blocks created by a give node p
    pub fn blocks_by(&self, node: &NodeId) -> Vec<Block> {
        self.blocks
            .iter()
            .filter(|(id, _)| &id.creator == node)
            .map(|(id, content)| Block {
                identity: id.clone(),
                content: content.clone(),
            })
            .collect()
    }
    /// Check the virtual chain axiom (CHAIN) for a specific node p.
    /// Any two p-blocks must be comparable under ≺:
    /// node(a) = node = p =>  a ≺ b ∨ b ≺ a
    ///
    /// A node that violates this is a Byzantine equivocator.
    pub fn satisfies_chain_axiom(&self, node: &NodeId) -> bool {
        let p_blocks = self.blocks_by(node);
        for i in 0..p_blocks.len() {
            for j in (i + 1)..p_blocks.len() {
                let a = &p_blocks[i].identity;
                let b = &p_blocks[j].identity;
                if !self.precedes(a, b) && !self.precedes(b, a) {
                    return false;
                }
            }
        }
        true
    }

    /// Check the chain axiom for every node in the blocklace
    pub fn satisfies_chain_axiom_all(&self) -> bool {
        self.all_nodes()
            .iter()
            .all(|node| self.satisfies_chain_axiom(node))
    }

    /// Returns the set of all byzantine equivocators - nodes violating (CHAIN).
    pub fn find_equivacators(&self) -> HashSet<NodeId> {
        self.all_nodes()
            .into_iter()
            .filter(|node| !self.satisfies_chain_axiom(node))
            .collect()
    }

    /// Get The tip of node p's virtual chain - the p-block that no other
    /// p-block precedes()i.e. p's most recent block in the blocklace
    pub fn tip_of(&self, node: &NodeId) -> Option<Block> {
        let p_blocks = self.blocks_by(node);
        p_blocks
            .iter()
            .find(|candidate| {
                !p_blocks.iter().any(|other| {
                    other.identity != candidate.identity
                        && self.precedes(&candidate.identity, &other.identity)
                })
            })
            .cloned()
    }

    /// Helper - collect the set of all node ids present the blocklace
    fn all_nodes(&self) -> HashSet<NodeId> {
        self.blocks.keys().map(|id| id.creator.clone()).collect()
    }
}
