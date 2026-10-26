use cordial_miners_core::crypto::{Sha256Hasher, block_hash, block_hash_ext};
use cordial_miners_core::{BlockContent, BlockIdentity, NodeId};
use std::collections::HashSet;

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

#[test]
fn same_content_different_creators_produce_different_hashes() {
    let content = BlockContent {
        payload: vec![1, 2, 3],
        predecessors: HashSet::new(),
    };
    let c1 = NodeId(vec![1]);
    let c2 = NodeId(vec![2]);
    assert_ne!(
        block_hash_ext(&c1, &content, &Sha256Hasher),
        block_hash_ext(&c2, &content, &Sha256Hasher)
    );
    assert_ne!(block_hash(&c1, &content), block_hash(&c2, &content));
}

#[test]
fn creator_payload_boundary_separation() {
    let empty_creator = NodeId(vec![]);
    let content_with_payload = BlockContent {
        payload: vec![1, 2, 3, 4],
        predecessors: HashSet::new(),
    };
    let creator_with_bytes = NodeId(vec![1, 2, 3, 4]);
    let empty_content = BlockContent {
        payload: vec![],
        predecessors: HashSet::new(),
    };
    assert_ne!(
        block_hash(&empty_creator, &content_with_payload),
        block_hash(&creator_with_bytes, &empty_content)
    );
}
