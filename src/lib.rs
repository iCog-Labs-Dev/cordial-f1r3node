use std::collections::{HashMap, HashSet};

// blocklace itself
struct Blocklace {
    ///  The map view: identity → content (this is B as a function)
    /// Closure axiom: every id in any predecessor set must be a key here
    blocks: HashMap<BlockIdentity, BlockContent>,
}

/// The cryptographic identity of a block: hash(C) signed by its creator.
/// From the paper: knowing `i` lets you recover `node(i) = p`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct BlockIdentity {
    /// SHA-256 (or similar) of the serialized BlockContent
    content_hash: [u8; 32],
    /// The node that signed this hash (recoverable from the signature,
    /// stored explicitly here for convenience)
    creator: NodeId,
    /// Signature bytes: sign(content_hash, creator_private_key)
    signature: Vec<u8>,
}

/// A node identity — in practice, a public key
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct NodeId(Vec<u8>); // public key bytes

/// C = (v, P) — the block content that gets hashed
#[derive(Debug, Clone)]
struct BlockContent {
    /// The arbitrary payload 'v' (operations, transactions, etc.)
    payload: Vec<u8>,
    /// 'P' — pointers to predecessor blocks via their identities
    predecessors: HashSet<BlockIdentity>,
}

/// b = (i, C) — the full block
#[derive(Debug, Clone)]
struct Block {
    /// 'i' = hash(C) signed by creator — the unique, author-stamped identity
    identity: BlockIdentity,
    /// 'C' — the content that was hashed to produce the identity
    content: BlockContent,
}

impl Block {
    /// b is initial (genesis) iff P = ∅
    fn is_initial(&self) -> bool {
        self.content.predecessors.is_empty()
    }

    /// node(b) = p — the creator of this block
    fn node(&self) -> &NodeId {
        &self.identity.creator
    }

    /// id(b) = i — the block's identity
    fn id(&self) -> &BlockIdentity {
        &self.identity
    }

    /// Returns true if `self` is pointed from `other`
    /// i.e., self ← other, i.e., id(self) ∈ P(other)
    fn is_pointed_from(&self, other: &Block) -> bool {
        other.content.predecessors.contains(&self.identity)
    }
}

/// nodes(S) = { node(b) | b ∈ S } — all creators in a set of blocks
fn nodes(blocks: &[Block]) -> HashSet<&NodeId> {
    blocks.iter().map(|b| b.node()).collect()
}

/// ids(S) = { id(b) | b ∈ S } — all identities in a set of blocks
fn ids(blocks: &[Block]) -> HashSet<&BlockIdentity> {
    blocks.iter().map(|b| b.id()).collect()
}


impl Blocklace {
    fn new() -> Self {
        Self { blocks: HashMap::new() }
    }

    /// B(b) — get the content of a block by its identity
    fn content(&self, id: &BlockIdentity) -> Option<&BlockContent> {
        self.blocks.get(id)
    }

    /// B[b] — get the full block (identity + content) by identity
    fn get(&self, id: &BlockIdentity) -> Option<Block> {
        self.blocks.get(id).map(|content| Block {
            identity: id.clone(),
            content: content.clone(),
        })
    }

    /// B[P] — get all blocks whose ids are in the set P
    fn get_set(&self, ids: &HashSet<BlockIdentity>) -> HashSet<Block> {
        ids.iter()
            .filter_map(|id| self.get(id))
            .collect()
    }

    /// dom(B) — the set of all known block identities
    fn dom(&self) -> HashSet<&BlockIdentity> {
        self.blocks.keys().collect()
    }

    /// Check the closure axiom: ∀(i, (v, P)) ∈ B · P ⊂ dom(B)
    fn is_closed(&self) -> bool {
        self.blocks.values().all(|content| {
            content.predecessors.iter()
                .all(|pred_id| self.blocks.contains_key(pred_id))
        })
    }

    /// Add a block — enforcing the closure axiom at insert time
    /// A block can only be added if all its predecessors are already present
    fn insert(&mut self, block: Block) -> Result<(), String> {
        // Check closure axiom before inserting
        for pred_id in &block.content.predecessors {
            if !self.blocks.contains_key(pred_id) {
                return Err(format!(
                    "Closure violation: predecessor {:?} not in blocklace",
                    pred_id
                ));
            }
        }
        self.blocks.insert(block.identity, block.content);
        Ok(())
    }
}