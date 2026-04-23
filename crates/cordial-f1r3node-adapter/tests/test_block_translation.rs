//! Tests for `block_translation` — blocklace `Block` ↔ f1r3node `BlockMessage`.

use std::collections::HashSet;

use cordial_miners_core::Block;
use cordial_miners_core::crypto::hash_content;
use cordial_miners_core::execution::{
    BlockState, Bond as CmBond, CordialBlockPayload, Deploy as CmDeploy,
    ProcessedDeploy as CmProcessed, ProcessedSystemDeploy as CmSystem,
    SignedDeploy as CmSignedDeploy,
};
use cordial_miners_core::types::{BlockContent, BlockIdentity, NodeId};

use cordial_f1r3node_adapter::block_translation::{
    BlockMessage, Body, F1r3flyState, Header, ProcessedSystemDeploy, TranslationError,
    block_to_message, message_to_block,
};

// ── Helpers ──────────────────────────────────────────────────────────────

fn node(b: u8) -> NodeId {
    NodeId(vec![b])
}

fn build_block(
    creator: NodeId,
    payload: CordialBlockPayload,
    predecessors: HashSet<BlockIdentity>,
    signature: Vec<u8>,
) -> Block {
    let content = BlockContent {
        payload: payload.to_bytes(),
        predecessors,
    };
    Block {
        identity: BlockIdentity {
            content_hash: hash_content(&content),
            creator,
            signature,
        },
        content,
    }
}

fn sample_payload() -> CordialBlockPayload {
    CordialBlockPayload {
        state: BlockState {
            pre_state_hash: vec![0x11; 32],
            post_state_hash: vec![0x22; 32],
            bonds: vec![
                CmBond {
                    validator: node(1),
                    stake: 100,
                },
                CmBond {
                    validator: node(2),
                    stake: 200,
                },
            ],
            block_number: 5,
        },
        deploys: vec![CmProcessed {
            deploy: CmSignedDeploy {
                deploy: CmDeploy {
                    term: b"@0!(\"hello\")".to_vec(),
                    timestamp: 1_700_000_000_000,
                    phlo_price: 1,
                    phlo_limit: 10_000,
                    valid_after_block_number: 0,
                    shard_id: "root".to_string(),
                },
                deployer: vec![0xaa; 32],
                signature: vec![0xbb; 64],
            },
            cost: 100,
            is_failed: false,
        }],
        rejected_deploys: vec![],
        system_deploys: vec![CmSystem::CloseBlock { succeeded: true }],
    }
}

// ── Basic shape preservation ─────────────────────────────────────────────

#[test]
fn genesis_block_translates_with_empty_parents_and_justifications() {
    let block = build_block(node(1), sample_payload(), HashSet::new(), vec![0xff; 64]);

    let msg = block_to_message(&block, "root").unwrap();
    assert!(msg.header.parents_hash_list.is_empty());
    assert!(msg.justifications.is_empty());
    assert_eq!(msg.sender, vec![1]);
    assert_eq!(msg.block_hash, block.identity.content_hash.to_vec());
    assert_eq!(msg.sig_algorithm, "ed25519");
    assert_eq!(msg.shard_id, "root");
}

#[test]
fn block_with_predecessors_packs_into_parents_and_justifications() {
    // Build a genesis predecessor
    let parent_payload = CordialBlockPayload::genesis(vec![]);
    let parent = build_block(node(1), parent_payload, HashSet::new(), vec![0x01; 64]);

    // Child block with one predecessor
    let mut predecessors = HashSet::new();
    predecessors.insert(parent.identity.clone());

    let child = build_block(node(2), sample_payload(), predecessors, vec![0x02; 64]);
    let msg = block_to_message(&child, "root").unwrap();

    assert_eq!(msg.header.parents_hash_list.len(), 1);
    assert_eq!(
        msg.header.parents_hash_list[0],
        parent.identity.content_hash.to_vec()
    );

    assert_eq!(msg.justifications.len(), 1);
    assert_eq!(msg.justifications[0].validator, vec![1]); // parent's creator
    assert_eq!(
        msg.justifications[0].latest_block_hash,
        parent.identity.content_hash.to_vec()
    );
}

#[test]
fn body_fields_mirror_payload() {
    let block = build_block(node(1), sample_payload(), HashSet::new(), vec![0x00; 64]);
    let msg = block_to_message(&block, "root").unwrap();

    assert_eq!(msg.body.state.pre_state_hash, vec![0x11; 32]);
    assert_eq!(msg.body.state.post_state_hash, vec![0x22; 32]);
    assert_eq!(msg.body.state.block_number, 5);
    assert_eq!(msg.body.state.bonds.len(), 2);

    assert_eq!(msg.body.deploys.len(), 1);
    assert_eq!(msg.body.deploys[0].cost, 100);
    assert!(!msg.body.deploys[0].is_failed);

    assert_eq!(msg.body.system_deploys.len(), 1);
    assert!(matches!(
        msg.body.system_deploys[0],
        ProcessedSystemDeploy::CloseBlock { succeeded: true }
    ));
}

// ── Roundtrip ────────────────────────────────────────────────────────────

#[test]
fn block_to_message_and_back_preserves_payload_and_creator() {
    let block = build_block(node(1), sample_payload(), HashSet::new(), vec![0xaa; 64]);
    let msg = block_to_message(&block, "root").unwrap();
    let recovered = message_to_block(&msg).unwrap();

    // Creator is preserved
    assert_eq!(recovered.identity.creator, block.identity.creator);
    // Signature is preserved
    assert_eq!(recovered.identity.signature, block.identity.signature);
    // Recovered predecessors are empty (matching original)
    assert!(recovered.content.predecessors.is_empty());

    // Payload roundtrips
    let original = CordialBlockPayload::from_bytes(&block.content.payload).unwrap();
    let round = CordialBlockPayload::from_bytes(&recovered.content.payload).unwrap();
    assert_eq!(original.state.pre_state_hash, round.state.pre_state_hash);
    assert_eq!(original.state.post_state_hash, round.state.post_state_hash);
    assert_eq!(original.state.block_number, round.state.block_number);
    assert_eq!(original.state.bonds.len(), round.state.bonds.len());
    assert_eq!(original.deploys.len(), round.deploys.len());
    assert_eq!(original.system_deploys.len(), round.system_deploys.len());

    // Recovered content_hash is deterministic from the new content
    let expected_hash = hash_content(&recovered.content);
    assert_eq!(recovered.identity.content_hash, expected_hash);
}

#[test]
fn child_block_roundtrips_predecessor_set() {
    let parent_payload = CordialBlockPayload::genesis(vec![]);
    let parent = build_block(node(1), parent_payload, HashSet::new(), vec![0x01; 64]);
    let parent_id = parent.identity.clone();

    let mut preds = HashSet::new();
    preds.insert(parent_id.clone());
    let child = build_block(node(2), sample_payload(), preds, vec![0x02; 64]);

    let msg = block_to_message(&child, "root").unwrap();
    let recovered = message_to_block(&msg).unwrap();

    assert_eq!(recovered.content.predecessors.len(), 1);
    let rec_pred = recovered.content.predecessors.iter().next().unwrap();
    assert_eq!(rec_pred.content_hash, parent_id.content_hash);
    assert_eq!(rec_pred.creator, parent_id.creator);
}

// ── Predecessor union (f1r3node → blocklace) ─────────────────────────────

#[test]
fn message_to_block_takes_union_of_parents_and_justifications() {
    // Construct a BlockMessage directly with a parent in parents_hash_list
    // and a different hash in justifications — both should become predecessors.
    let msg = BlockMessage {
        block_hash: vec![0x00; 32],
        header: Header {
            parents_hash_list: vec![[0xaa; 32].to_vec()],
            timestamp: 0,
            version: 1,
            extra_bytes: vec![],
        },
        body: Body {
            state: F1r3flyState {
                pre_state_hash: vec![],
                post_state_hash: vec![],
                bonds: vec![],
                block_number: 1,
            },
            deploys: vec![],
            rejected_deploys: vec![],
            system_deploys: vec![],
            extra_bytes: vec![],
        },
        justifications: vec![
            cordial_f1r3node_adapter::block_translation::Justification {
                validator: vec![3],
                latest_block_hash: [0xbb; 32].to_vec(),
            },
            // Justification for the parent hash too — should dedupe
            cordial_f1r3node_adapter::block_translation::Justification {
                validator: vec![2],
                latest_block_hash: [0xaa; 32].to_vec(),
            },
        ],
        sender: vec![9],
        seq_num: 0,
        sig: vec![0xff; 64],
        sig_algorithm: "ed25519".to_string(),
        shard_id: "root".to_string(),
        extra_bytes: vec![],
    };

    let block = message_to_block(&msg).unwrap();
    assert_eq!(block.content.predecessors.len(), 2); // unioned, deduped

    // Find each predecessor
    let pred_a = block
        .content
        .predecessors
        .iter()
        .find(|p| p.content_hash == [0xaa; 32])
        .expect("parent hash missing");
    // Creator of [0xaa; 32] comes from justifications (validator 2), not the sender
    assert_eq!(pred_a.creator, NodeId(vec![2]));

    let pred_b = block
        .content
        .predecessors
        .iter()
        .find(|p| p.content_hash == [0xbb; 32])
        .expect("justification-only hash missing");
    assert_eq!(pred_b.creator, NodeId(vec![3]));
}

#[test]
fn parent_without_matching_justification_falls_back_to_sender_as_creator() {
    let msg = BlockMessage {
        block_hash: vec![0x00; 32],
        header: Header {
            parents_hash_list: vec![[0xcd; 32].to_vec()],
            timestamp: 0,
            version: 1,
            extra_bytes: vec![],
        },
        body: Body {
            state: F1r3flyState {
                pre_state_hash: vec![],
                post_state_hash: vec![],
                bonds: vec![],
                block_number: 1,
            },
            deploys: vec![],
            rejected_deploys: vec![],
            system_deploys: vec![],
            extra_bytes: vec![],
        },
        justifications: vec![], // no justifications
        sender: vec![42],
        seq_num: 0,
        sig: vec![0xff; 64],
        sig_algorithm: "ed25519".to_string(),
        shard_id: "root".to_string(),
        extra_bytes: vec![],
    };

    let block = message_to_block(&msg).unwrap();
    assert_eq!(block.content.predecessors.len(), 1);
    let pred = block.content.predecessors.iter().next().unwrap();
    assert_eq!(pred.creator, NodeId(vec![42])); // fallback to sender
}

// ── Error paths ──────────────────────────────────────────────────────────

#[test]
fn invalid_payload_bytes_fails_translation() {
    let content = BlockContent {
        payload: vec![0xff, 0x00, 0xab], // not a valid CordialBlockPayload
        predecessors: HashSet::new(),
    };
    let block = Block {
        identity: BlockIdentity {
            content_hash: hash_content(&content),
            creator: node(1),
            signature: vec![],
        },
        content,
    };
    let err = block_to_message(&block, "root").unwrap_err();
    assert!(matches!(err, TranslationError::PayloadDecodeFailed(_)));
}

#[test]
fn wrong_predecessor_hash_length_fails_translation() {
    let msg = BlockMessage {
        block_hash: vec![0x00; 32],
        header: Header {
            parents_hash_list: vec![vec![0xaa, 0xbb]], // only 2 bytes, not 32
            timestamp: 0,
            version: 1,
            extra_bytes: vec![],
        },
        body: Body {
            state: F1r3flyState {
                pre_state_hash: vec![],
                post_state_hash: vec![],
                bonds: vec![],
                block_number: 1,
            },
            deploys: vec![],
            rejected_deploys: vec![],
            system_deploys: vec![],
            extra_bytes: vec![],
        },
        justifications: vec![],
        sender: vec![1],
        seq_num: 0,
        sig: vec![0xff; 64],
        sig_algorithm: "ed25519".to_string(),
        shard_id: "root".to_string(),
        extra_bytes: vec![],
    };

    let err = message_to_block(&msg).unwrap_err();
    assert!(matches!(
        err,
        TranslationError::InvalidPredecessorHash { got: 2, .. }
    ));
}

// ── Numeric edge cases ───────────────────────────────────────────────────

#[test]
fn u64_that_fits_in_i64_translates_fine() {
    let mut payload = sample_payload();
    payload.state.block_number = 1_000_000;
    let block = build_block(node(1), payload, HashSet::new(), vec![0x00; 64]);
    let msg = block_to_message(&block, "root").unwrap();
    assert_eq!(msg.body.state.block_number, 1_000_000);
}

#[test]
fn u64_overflowing_i64_fails_translation() {
    let mut payload = sample_payload();
    payload.state.block_number = u64::MAX; // doesn't fit in i64
    let block = build_block(node(1), payload, HashSet::new(), vec![0x00; 64]);
    let err = block_to_message(&block, "root").unwrap_err();
    assert!(matches!(
        err,
        TranslationError::NumericOverflow("block_number")
    ));
}

#[test]
fn negative_i64_fails_to_u64_translation() {
    let mut msg = BlockMessage {
        block_hash: vec![0x00; 32],
        header: Header {
            parents_hash_list: vec![],
            timestamp: 0,
            version: 1,
            extra_bytes: vec![],
        },
        body: Body {
            state: F1r3flyState {
                pre_state_hash: vec![],
                post_state_hash: vec![],
                bonds: vec![],
                block_number: -1, // negative — can't become u64
            },
            deploys: vec![],
            rejected_deploys: vec![],
            system_deploys: vec![],
            extra_bytes: vec![],
        },
        justifications: vec![],
        sender: vec![1],
        seq_num: 0,
        sig: vec![0xff; 64],
        sig_algorithm: "ed25519".to_string(),
        shard_id: "root".to_string(),
        extra_bytes: vec![],
    };
    let err = message_to_block(&msg).unwrap_err();
    assert!(matches!(err, TranslationError::NumericOverflow(_)));

    // Also test bond stake overflow
    msg.body.state.block_number = 0;
    msg.body.state.bonds = vec![cordial_f1r3node_adapter::block_translation::Bond {
        validator: vec![1],
        stake: -5,
    }];
    let err = message_to_block(&msg).unwrap_err();
    assert!(matches!(err, TranslationError::NumericOverflow(_)));
}

// ── Deterministic ordering ───────────────────────────────────────────────

#[test]
fn translation_is_deterministic_across_predecessor_insertion_order() {
    let p1_payload = CordialBlockPayload::genesis(vec![]);
    let p1 = build_block(node(1), p1_payload, HashSet::new(), vec![0x01; 64]);
    let p2_payload = CordialBlockPayload::genesis(vec![]);
    let p2 = build_block(node(2), p2_payload, HashSet::new(), vec![0x02; 64]);

    // Two predecessor sets with same members, different insertion order
    let mut preds_a = HashSet::new();
    preds_a.insert(p1.identity.clone());
    preds_a.insert(p2.identity.clone());

    let mut preds_b = HashSet::new();
    preds_b.insert(p2.identity.clone());
    preds_b.insert(p1.identity.clone());

    let block_a = build_block(node(3), sample_payload(), preds_a, vec![0x03; 64]);
    let block_b = build_block(node(3), sample_payload(), preds_b, vec![0x03; 64]);

    let msg_a = block_to_message(&block_a, "root").unwrap();
    let msg_b = block_to_message(&block_b, "root").unwrap();

    // Sorted output → same parents_hash_list and justifications regardless of order
    assert_eq!(
        msg_a.header.parents_hash_list,
        msg_b.header.parents_hash_list
    );
    assert_eq!(msg_a.justifications, msg_b.justifications);
}
