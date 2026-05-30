use cordial_miners_core::consensus::{CordialEvidencePool, EvidencePool};
use cordial_miners_core::{Block, BlockContent, BlockIdentity, NodeId};
use std::collections::HashSet;

fn node(byte: u8) -> NodeId {
    NodeId(vec![byte])
}

fn identity(creator: NodeId, tag: u8) -> BlockIdentity {
    let mut content_hash = [0u8; 32];
    content_hash[0] = creator.0[0];
    content_hash[1] = tag;

    BlockIdentity {
        content_hash,
        creator,
        signature: vec![tag, tag.wrapping_add(1)],
    }
}

fn block(creator: NodeId, tag: u8, payload: Vec<u8>) -> Block {
    Block {
        identity: identity(creator, tag),
        content: BlockContent {
            payload,
            predecessors: HashSet::new(),
        },
    }
}

#[test]
fn duplicate_conflicting_pair_is_recorded_once() {
    let validator = node(1);
    let left = block(validator.clone(), 1, vec![0xa1]);
    let right = block(validator.clone(), 2, vec![0xb1]);
    let mut pool = CordialEvidencePool::new();

    assert!(pool.record_equivocation(validator.clone(), 0, vec![left.clone(), right.clone()]));
    assert!(!pool.record_equivocation(validator.clone(), 0, vec![left.clone(), right.clone()]));
    assert!(!pool.record_equivocation(validator.clone(), 0, vec![right, left]));

    let evidence = pool.evidence_for(&validator);
    assert_eq!(evidence.len(), 1);
    assert_eq!(pool.len(), 1);
}

#[test]
fn evidence_retains_original_cordial_blocks() {
    let validator = node(1);
    let left = block(validator.clone(), 0x0a, vec![0xde, 0xad]);
    let right = block(validator.clone(), 0x0b, vec![0xbe, 0xef]);
    let mut pool = CordialEvidencePool::new();

    assert!(pool.record_equivocation(validator.clone(), 0, vec![left.clone(), right.clone()]));

    let evidence = pool.evidence_for(&validator);
    assert_eq!(evidence.len(), 1);
    assert_eq!(evidence[0].validator, validator);
    assert_eq!(evidence[0].round, 0);
    assert_eq!(evidence[0].blocks, vec![left, right]);
    assert_eq!(evidence[0].blocks[0].content.payload, vec![0xde, 0xad]);
    assert_eq!(evidence[0].blocks[0].identity.signature, vec![0x0a, 0x0b]);
    assert_eq!(evidence[0].blocks[1].content.payload, vec![0xbe, 0xef]);
    assert_eq!(evidence[0].blocks[1].identity.signature, vec![0x0b, 0x0c]);
}

#[test]
fn evidence_for_validator_is_deterministically_ordered() {
    let validator = node(1);
    let other_validator = node(2);
    let round_zero_left = block(validator.clone(), 1, vec![1]);
    let round_zero_right = block(validator.clone(), 2, vec![2]);
    let round_one_left = block(validator.clone(), 3, vec![3]);
    let round_one_right = block(validator.clone(), 4, vec![4]);
    let other_left = block(other_validator.clone(), 5, vec![5]);
    let other_right = block(other_validator, 6, vec![6]);
    let mut pool = CordialEvidencePool::new();

    assert!(pool.record_equivocation(
        validator.clone(),
        1,
        vec![round_one_right.clone(), round_one_left.clone()],
    ));
    assert!(pool.record_equivocation(
        validator.clone(),
        0,
        vec![round_zero_right.clone(), round_zero_left.clone()],
    ));
    assert!(pool.record_equivocation(node(2), 0, vec![other_right, other_left]));

    let evidence = pool.evidence_for(&validator);
    assert_eq!(evidence.len(), 2);
    assert_eq!(evidence[0].round, 0);
    assert_eq!(
        evidence[0]
            .blocks
            .iter()
            .map(|block| block.identity.clone())
            .collect::<Vec<_>>(),
        vec![round_zero_left.identity, round_zero_right.identity]
    );
    assert_eq!(evidence[1].round, 1);
    assert_eq!(
        evidence[1]
            .blocks
            .iter()
            .map(|block| block.identity.clone())
            .collect::<Vec<_>>(),
        vec![round_one_left.identity, round_one_right.identity]
    );
}
