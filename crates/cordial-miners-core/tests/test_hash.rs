use cordial_miners_core::crypto::hash_content;
use cordial_miners_core::{BlockContent, BlockIdentity, NodeId};
use std::collections::HashSet;

#[test]
fn hash_of_empty_genesis_is_deterministic() {
    let content = BlockContent {
        payload: vec![],
        predecessors: HashSet::new(),
    };
    let h1 = hash_content(&content);
    let h2 = hash_content(&content);
    assert_eq!(h1, h2);
}

#[test]
fn different_payloads_produce_different_hashes() {
    let c1 = BlockContent {
        payload: vec![1, 2, 3],
        predecessors: HashSet::new(),
    };
    let c2 = BlockContent {
        payload: vec![4, 5, 6],
        predecessors: HashSet::new(),
    };
    assert_ne!(hash_content(&c1), hash_content(&c2));
}

#[test]
fn different_predecessors_produce_different_hashes() {
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
        predecessors: HashSet::from([pred_a.clone()]),
    };
    let c2 = BlockContent {
        payload: vec![],
        predecessors: HashSet::from([pred_b.clone()]),
    };
    assert_ne!(hash_content(&c1), hash_content(&c2));
}

#[test]
fn hash_is_independent_of_predecessor_insertion_order() {
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

    // Build sets in different insertion orders
    let mut set1 = HashSet::new();
    set1.insert(pred_a.clone());
    set1.insert(pred_b.clone());

    let mut set2 = HashSet::new();
    set2.insert(pred_b.clone());
    set2.insert(pred_a.clone());

    let c1 = BlockContent { payload: vec![10], predecessors: set1 };
    let c2 = BlockContent { payload: vec![10], predecessors: set2 };
    assert_eq!(hash_content(&c1), hash_content(&c2));
}

#[test]
fn hash_output_is_32_bytes() {
    let content = BlockContent {
        payload: vec![0xff; 100],
        predecessors: HashSet::new(),
    };
    let h = hash_content(&content);
    assert_eq!(h.len(), 32);
}
