use cordial_miners_core::crypto::{Sha256Hasher, block_hash, block_hash_ext};
use cordial_miners_core::{BlockContent, BlockIdentity, NodeId};
use std::collections::HashSet;

// Blake2b-256 Tests (New Default)

#[test]
fn blake_hash_of_empty_genesis_is_deterministic() {
    let creator = NodeId(vec![1]);
    let content = BlockContent {
        payload: vec![],
        predecessors: HashSet::new(),
    };
    let h1 = block_hash(&creator, &content);
    let h2 = block_hash(&creator, &content);
    assert_eq!(h1, h2);
}

#[test]
fn blake_different_payloads_produce_different_hashes() {
    let creator = NodeId(vec![1]);
    let c1 = BlockContent {
        payload: vec![1, 2, 3],
        predecessors: HashSet::new(),
    };
    let c2 = BlockContent {
        payload: vec![4, 5, 6],
        predecessors: HashSet::new(),
    };
    assert_ne!(block_hash(&creator, &c1), block_hash(&creator, &c2));
}

#[test]
fn blake_different_predecessors_produce_different_hashes() {
    let creator = NodeId(vec![1]);
    let pred_a = BlockIdentity {
        content_hash: [0x01; 32],
        creator: NodeId(vec![1]),
        signature: vec![],
    };
    let pred_b = BlockIdentity {
        content_hash: [0x02; 32],
        creator: NodeId(vec![2]),
        signature: vec![],
    };
    let c1 = BlockContent {
        payload: vec![],
        predecessors: HashSet::from([pred_a]),
    };
    let c2 = BlockContent {
        payload: vec![],
        predecessors: HashSet::from([pred_b]),
    };
    assert_ne!(block_hash(&creator, &c1), block_hash(&creator, &c2));
}

#[test]
fn blake_hash_is_independent_of_predecessor_insertion_order() {
    let creator = NodeId(vec![1]);
    let pred_a = BlockIdentity {
        content_hash: [0x01; 32],
        creator: NodeId(vec![1]),
        signature: vec![],
    };
    let pred_b = BlockIdentity {
        content_hash: [0x02; 32],
        creator: NodeId(vec![2]),
        signature: vec![],
    };
    let mut set1 = HashSet::new();
    set1.insert(pred_a.clone());
    set1.insert(pred_b.clone());
    let mut set2 = HashSet::new();
    set2.insert(pred_b);
    set2.insert(pred_a);
    let c1 = BlockContent {
        payload: vec![10],
        predecessors: set1,
    };
    let c2 = BlockContent {
        payload: vec![10],
        predecessors: set2,
    };
    assert_eq!(block_hash(&creator, &c1), block_hash(&creator, &c2));
}

#[test]
fn content_hash_is_independent_of_predecessor_signature_proof() {
    let creator = NodeId(vec![1]);
    let first = BlockIdentity {
        content_hash: [0x01; 32],
        creator: NodeId(vec![1]),
        signature: vec![0xaa],
    };
    let mut alternate = first.clone();
    alternate.signature = vec![0xbb];

    let first_content = BlockContent {
        payload: vec![10],
        predecessors: HashSet::from([first]),
    };
    let alternate_content = BlockContent {
        payload: vec![10],
        predecessors: HashSet::from([alternate]),
    };

    assert_eq!(
        block_hash(&creator, &first_content),
        block_hash(&creator, &alternate_content)
    );
    assert_eq!(
        block_hash_ext(&creator, &first_content, &Sha256Hasher),
        block_hash_ext(&creator, &alternate_content, &Sha256Hasher)
    );
}

#[test]
fn blake_hash_output_is_32_bytes() {
    let creator = NodeId(vec![1]);
    let content = BlockContent {
        payload: vec![0xff; 100],
        predecessors: HashSet::new(),
    };
    assert_eq!(block_hash(&creator, &content).len(), 32);
}

// SHA256 Tests (Legacy Algorithm)

#[test]
fn sha256_hash_of_empty_genesis_is_deterministic() {
    let creator = NodeId(vec![1]);
    let content = BlockContent {
        payload: vec![],
        predecessors: HashSet::new(),
    };
    let h1 = block_hash_ext(&creator, &content, &Sha256Hasher);
    let h2 = block_hash_ext(&creator, &content, &Sha256Hasher);
    assert_eq!(h1, h2);
}

#[test]
fn sha256_different_payloads_produce_different_hashes() {
    let creator = NodeId(vec![1]);
    let c1 = BlockContent {
        payload: vec![1, 2, 3],
        predecessors: HashSet::new(),
    };
    let c2 = BlockContent {
        payload: vec![4, 5, 6],
        predecessors: HashSet::new(),
    };
    assert_ne!(
        block_hash_ext(&creator, &c1, &Sha256Hasher),
        block_hash_ext(&creator, &c2, &Sha256Hasher)
    );
}

#[test]
fn sha256_different_predecessors_produce_different_hashes() {
    let creator = NodeId(vec![1]);
    let pred_a = BlockIdentity {
        content_hash: [0x01; 32],
        creator: NodeId(vec![1]),
        signature: vec![],
    };
    let pred_b = BlockIdentity {
        content_hash: [0x02; 32],
        creator: NodeId(vec![2]),
        signature: vec![],
    };
    let c1 = BlockContent {
        payload: vec![],
        predecessors: HashSet::from([pred_a]),
    };
    let c2 = BlockContent {
        payload: vec![],
        predecessors: HashSet::from([pred_b]),
    };
    assert_ne!(
        block_hash_ext(&creator, &c1, &Sha256Hasher),
        block_hash_ext(&creator, &c2, &Sha256Hasher)
    );
}

#[test]
fn sha256_hash_is_independent_of_predecessor_insertion_order() {
    let creator = NodeId(vec![1]);
    let pred_a = BlockIdentity {
        content_hash: [0x01; 32],
        creator: NodeId(vec![1]),
        signature: vec![],
    };
    let pred_b = BlockIdentity {
        content_hash: [0x02; 32],
        creator: NodeId(vec![2]),
        signature: vec![],
    };
    let mut set1 = HashSet::new();
    set1.insert(pred_a.clone());
    set1.insert(pred_b.clone());
    let mut set2 = HashSet::new();
    set2.insert(pred_b);
    set2.insert(pred_a);
    let c1 = BlockContent {
        payload: vec![10],
        predecessors: set1,
    };
    let c2 = BlockContent {
        payload: vec![10],
        predecessors: set2,
    };
    assert_eq!(
        block_hash_ext(&creator, &c1, &Sha256Hasher),
        block_hash_ext(&creator, &c2, &Sha256Hasher)
    );
}

#[test]
fn sha256_hash_output_is_32_bytes() {
    let creator = NodeId(vec![1]);
    let content = BlockContent {
        payload: vec![0xff; 100],
        predecessors: HashSet::new(),
    };
    assert_eq!(block_hash_ext(&creator, &content, &Sha256Hasher).len(), 32);
}
