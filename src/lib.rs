use std::collections::HashSet;

// The struct definition of block content
struct BlockContent {
    payload: String,           // The 'v' (Value)
    predecessors: HashSet<String>, // The 'P' (Set of IDs)
}

// The "b" (Block) from the paper
struct Block {
    identity: String,          // The 'i' (Signed Hash)
    creator: String,           // The 'p' (Node identity)
    content: BlockContent,     // The 'C'
}

impl Block {
    // A helper to check if it's the genesis block
    fn is_initial(&self) -> bool {
        self.content.predecessors.is_empty()
    }
}