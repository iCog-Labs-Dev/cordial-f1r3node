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

// --- Tests for evidence_by_round ---

#[test]
fn evidence_by_round_returns_only_matching_round() {
    // validator equivocates in round 0 and round 2.
    // evidence_by_round(0) must return only the round-0 record,
    // and evidence_by_round(2) only the round-2 record.
    let validator = node(1);
    let r0_left = block(validator.clone(), 10, vec![1]);
    let r0_right = block(validator.clone(), 11, vec![2]);
    let r2_left = block(validator.clone(), 20, vec![3]);
    let r2_right = block(validator.clone(), 21, vec![4]);
    let mut pool = CordialEvidencePool::new();

    assert!(pool.record_equivocation(
        validator.clone(),
        0,
        vec![r0_left.clone(), r0_right.clone()],
    ));
    assert!(pool.record_equivocation(
        validator.clone(),
        2,
        vec![r2_left.clone(), r2_right.clone()],
    ));

    let round_0 = pool.evidence_by_round(0);
    assert_eq!(round_0.len(), 1);
    assert_eq!(round_0[0].round, 0);
    assert_eq!(round_0[0].validator, validator);

    let round_2 = pool.evidence_by_round(2);
    assert_eq!(round_2.len(), 1);
    assert_eq!(round_2[0].round, 2);
    assert_eq!(round_2[0].validator, validator);

    // round 1 had no activity — must return empty
    assert!(pool.evidence_by_round(1).is_empty());
}

#[test]
fn evidence_by_round_empty_when_pool_has_no_equivocation_in_that_round() {
    let validator = node(1);
    let left = block(validator.clone(), 1, vec![0xaa]);
    let right = block(validator.clone(), 2, vec![0xbb]);
    let mut pool = CordialEvidencePool::new();

    assert!(pool.record_equivocation(validator.clone(), 5, vec![left, right]));

    // querying any round that never received evidence returns empty
    assert!(pool.evidence_by_round(0).is_empty());
    assert!(pool.evidence_by_round(4).is_empty());
    assert!(pool.evidence_by_round(6).is_empty());
    assert!(pool.evidence_by_round(99).is_empty());
}

#[test]
fn evidence_by_round_aggregates_across_multiple_validators() {
    // Alice and Bob both equivocate in round 3.
    // Charlie equivocates only in round 7.
    // evidence_by_round(3) must return 2 records (one per validator).
    // evidence_by_round(7) must return 1 record.
    let alice = node(1);
    let bob = node(2);
    let charlie = node(3);

    let alice_left = block(alice.clone(), 10, vec![1]);
    let alice_right = block(alice.clone(), 11, vec![2]);
    let bob_left = block(bob.clone(), 20, vec![3]);
    let bob_right = block(bob.clone(), 21, vec![4]);
    let charlie_left = block(charlie.clone(), 30, vec![5]);
    let charlie_right = block(charlie.clone(), 31, vec![6]);

    let mut pool = CordialEvidencePool::new();
    assert!(pool.record_equivocation(alice.clone(), 3, vec![alice_left, alice_right]));
    assert!(pool.record_equivocation(bob.clone(), 3, vec![bob_left, bob_right]));
    assert!(pool.record_equivocation(charlie.clone(), 7, vec![charlie_left, charlie_right]));

    let round_3 = pool.evidence_by_round(3);
    assert_eq!(round_3.len(), 2, "expected 2 evidence records for round 3");
    // all records must belong to round 3
    assert!(round_3.iter().all(|e| e.round == 3));

    let round_7 = pool.evidence_by_round(7);
    assert_eq!(round_7.len(), 1);
    assert_eq!(round_7[0].validator, charlie);
    assert_eq!(round_7[0].round, 7);
}

// --- Tests for equivocating_validators_in_round ---

#[test]
fn equivocating_validators_in_round_returns_correct_validators() {
    let alice = node(1);
    let bob = node(2);
    let carol = node(3);

    // alice and bob equivocate in round 4; carol equivocates only in round 9
    let mut pool = CordialEvidencePool::new();
    assert!(pool.record_equivocation(
        alice.clone(),
        4,
        vec![
            block(alice.clone(), 1, vec![1]),
            block(alice.clone(), 2, vec![2]),
        ],
    ));
    assert!(pool.record_equivocation(
        bob.clone(),
        4,
        vec![
            block(bob.clone(), 3, vec![3]),
            block(bob.clone(), 4, vec![4]),
        ],
    ));
    assert!(pool.record_equivocation(
        carol.clone(),
        9,
        vec![
            block(carol.clone(), 5, vec![5]),
            block(carol.clone(), 6, vec![6]),
        ],
    ));

    let mut validators_round_4 = pool
        .equivocating_validators_in_round(4)
        .into_iter()
        .cloned()
        .collect::<Vec<_>>();
    validators_round_4.sort();
    assert_eq!(validators_round_4, vec![alice.clone(), bob.clone()]);

    let validators_round_9 = pool.equivocating_validators_in_round(9);
    assert_eq!(validators_round_9.len(), 1);
    assert_eq!(*validators_round_9[0], carol);

    // round with no equivocators returns empty
    assert!(pool.equivocating_validators_in_round(0).is_empty());
}

#[test]
fn equivocating_validators_in_round_is_deduplicated() {
    // record two separate equivocation pairs for the same validator in the same
    // round — the validator must appear only once in the result.
    let validator = node(1);
    let mut pool = CordialEvidencePool::new();

    // first pair
    assert!(pool.record_equivocation(
        validator.clone(),
        2,
        vec![
            block(validator.clone(), 1, vec![0xaa]),
            block(validator.clone(), 2, vec![0xbb]),
        ],
    ));
    // second, distinct pair in the same round
    assert!(pool.record_equivocation(
        validator.clone(),
        2,
        vec![
            block(validator.clone(), 3, vec![0xcc]),
            block(validator.clone(), 4, vec![0xdd]),
        ],
    ));

    let validators = pool.equivocating_validators_in_round(2);
    assert_eq!(
        validators.len(),
        1,
        "same validator with multiple evidence records in one round must appear only once"
    );
    assert_eq!(*validators[0], validator);
}
