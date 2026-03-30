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


    // direct predecessors of b — implements ←b
    /// { a | a ← b } i.e. the blocks directly pointed to by b
    fn predecessors(&self, id: &BlockIdentity) -> HashSet<Block> {
        match self.content(id) {
            None => HashSet::new(),
            Some(content) => self.get_set(&content.predecessors),
        }
    }


    // All ancestors of b — implements ≺b
    /// The transitive closure of ← starting at b, NOT including b itself
    fn ancestors(&self, id: &BlockIdentity) -> HashSet<Block> {
        let mut visited = HashSet::new();
        let mut queue: Vec<BlockIdentity> = vec![id.clone()];

        while let Some(current_id) = queue.pop() {
            if let Some(content) = self.content(&current_id) {
                for pred_id in &content.predecessors {
                    // only visit if not yet seen
                    if visited.insert(pred_id.clone()) {
                        queue.push(pred_id.clone());
                    }
                }
            }
        }

        // return the full blocks for all visited ancestors
        visited.iter()
            .filter_map(|id| self.get(id))
            .collect()
    }

    /// ⪯b — ancestors including b itself
    /// This is the "downward closure" the paper uses heavily
    fn ancestors_inclusive(&self, id: &BlockIdentity) -> HashSet<Block> {
        let mut result = self.ancestors(id);
        if let Some(block) = self.get(id) {
            result.insert(block);
        }
        result
    }


    // ≺S — all ancestors of any block in a set
    fn ancestors_of_set(&self, ids: &HashSet<BlockIdentity>) -> HashSet<Block> {
        ids.iter()
            .flat_map(|id| self.ancestors(id))
            .collect()
    }


    /// Check if a precedes b — i.e. a ≺ b
    /// a is somewhere in b's ancestry
    fn precedes(&self, a: &BlockIdentity, b: &BlockIdentity) -> bool {
        self.ancestors(b).iter().any(|block| &block.identity == a)
    }

    /// Check if a ⪯ b — precedes or equals
    fn precedes_or_equals(&self, a: &BlockIdentity, b: &BlockIdentity) -> bool {
        a == b || self.precedes(a, b)
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


        /// Returns all blocks created by a given node
    fn blocks_by(&self, node: &NodeId) -> Vec<Block> {
        self.blocks
            .iter()
            .filter(|(id, _)| &id.creator == node)
            .map(|(id, content)| Block {
                identity: id.clone(),
                content: content.clone(),
            })
            .collect()
    }

    /// Check the virtual chain axiom for a specific node p — (CHAIN)
    /// Any two p-blocks must be comparable under ≺
    fn satisfies_chain_axiom(&self, node: &NodeId) -> bool {
        let p_blocks = self.blocks_by(node);

        // check every pair (a, b) of distinct p-blocks
        for i in 0..p_blocks.len() {
            for j in (i + 1)..p_blocks.len() {
                let a = &p_blocks[i].identity;
                let b = &p_blocks[j].identity;

                // at least one of a ≺ b or b ≺ a must hold
                let comparable = self.precedes(a, b) || self.precedes(b, a);
                if !comparable {
                    return false; // Byzantine equivocator detected
                }
            }
        }
        true
    }

    /// Check chain axiom for ALL nodes in the blocklace
    fn satisfies_chain_axiom_all(&self) -> bool {
        let all_nodes: HashSet<NodeId> = self.blocks
            .keys()
            .map(|id| id.creator.clone())
            .collect();

        all_nodes.iter().all(|node| self.satisfies_chain_axiom(node))
    }

    /// Identify Byzantine equivocators — nodes that violate (CHAIN)
    fn find_equivocators(&self) -> HashSet<NodeId> {
        let all_nodes: HashSet<NodeId> = self.blocks
            .keys()
            .map(|id| id.creator.clone())
            .collect();

        all_nodes
            .into_iter()
            .filter(|node| !self.satisfies_chain_axiom(node))
            .collect()
    }

    /// Get the latest block of node p — the one that precedes no other p-block
    /// (the "tip" of p's virtual chain)
    fn tip_of(&self, node: &NodeId) -> Option<Block> {
        let p_blocks = self.blocks_by(node);

        // the tip is the p-block that is NOT preceded by any other p-block
        p_blocks.iter().find(|candidate| {
            !p_blocks.iter().any(|other| {
                other.identity != candidate.identity
                    && self.precedes(&candidate.identity, &other.identity)
            })
        }).cloned()
    }
}


/// Represents a node's full local state
struct Node {
    /// The node's identity (public key)
    id: NodeId,
    /// The node's private key — used for signing
    private_key: Vec<u8>,
    /// B — the validated, closed blocklace
    blocklace: Blocklace,
    /// D — buffered blocks waiting for their predecessors
    buffer: Vec<Block>,
}

impl Node {
    /// max≺(B) — maximal blocks in B under ≺
    /// These are blocks that no other block in B points to.
    /// i.e., they are not in any predecessor set of any block in B
    fn maximal_blocks(&self) -> HashSet<Block> {
        // collect all ids that ARE pointed to by someone
        let pointed_to: HashSet<&BlockIdentity> = self.blocklace
            .blocks
            .values()
            .flat_map(|content| content.predecessors.iter())
            .collect();

        // maximal = blocks whose id is NOT in pointed_to
        self.blocklace
            .blocks
            .iter()
            .filter(|(id, _)| !pointed_to.contains(id))
            .map(|(id, content)| Block {
                identity: id.clone(),
                content: content.clone(),
            })
            .collect()
    }

    /// The core add(v) operation from the paper:
    /// B' = B ∪ { new_p(B, v) }
    fn add(&mut self, payload: Vec<u8>) -> Block {
        // Step 1: P = ids(max≺(B)) — predecessor set is the current tips
        let max_blocks = self.maximal_blocks();
        let predecessors: HashSet<BlockIdentity> = max_blocks
            .into_iter()
            .map(|b| b.identity)
            .collect();

        // Step 2: C = (v, P)
        let content = BlockContent {
            payload,
            predecessors,
        };

        // Step 3: i = signedhash((v, P), k_p)
        let identity = self.sign_content(&content);

        // Step 4: new block b = (i, C)
        let block = Block { identity, content };

        // Step 5: B' = B ∪ {b} — closure axiom holds trivially
        // since P = ids(max(B)) and all of B is already closed
        self.blocklace.insert(block.clone())
            .expect("New block must satisfy closure by construction");

        block
    }

    /// Simulate signedhash((v, P), k_p)
    /// In production: serialize content, SHA-256, sign with private key
    fn sign_content(&self, content: &BlockContent) -> BlockIdentity {
        // placeholder — real impl would use ed25519 or similar
        let content_hash = hash_content(content);
        let signature = sign(&content_hash, &self.private_key);

        BlockIdentity {
            content_hash,
            creator: self.id.clone(),
            signature,
        }
    }

    /// Receive a block from the network — either accept into B or buffer in D
    fn receive(&mut self, block: Block) {
        if self.can_accept(&block) {
            self.accept(block);
        } else {
            // predecessors not yet in B — buffer it
            self.buffer.push(block);
        }
    }

    /// A block can be accepted into B iff all its predecessors are already in B
    /// (this is exactly the closure axiom check)
    fn can_accept(&self, block: &Block) -> bool {
        block.content.predecessors.iter()
            .all(|pred_id| self.blocklace.blocks.contains_key(pred_id))
    }

    /// Accept a block into B, then drain any buffered blocks that are now unblocked
    fn accept(&mut self, block: Block) {
        self.blocklace.insert(block)
            .expect("Block passed can_accept so closure is guaranteed");

        // try to drain the buffer — newly accepted block may unblock others
        self.drain_buffer();
    }

    /// Repeatedly scan buffer until no more blocks can be promoted to B
    fn drain_buffer(&mut self) {
        loop {
            // find the first buffered block that is now acceptable
            let pos = self.buffer.iter()
                .position(|b| self.can_accept(b));

            match pos {
                None => break, // nothing left to unblock
                Some(i) => {
                    let block = self.buffer.remove(i);
                    self.blocklace.insert(block)
                        .expect("Buffer block passed can_accept");
                    // loop again — this might unblock more
                }
            }
        }
    }
}