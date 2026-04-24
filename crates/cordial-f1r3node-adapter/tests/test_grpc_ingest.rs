//! Integration tests for `grpc_ingest` — network message ingestion and validation pipeline.
//!
//! Tests the full pipeline from network-level `Message` enums through the
//! `GrpcBlockMapper` (structural validation) and into `BlocklaceAdapter`
//! (semantic validation and consensus callbacks).

use std::collections::HashSet;

use cordial_miners_core::Block;
use cordial_miners_core::crypto::{hash_content, sign};
use cordial_miners_core::network::Message;
use cordial_miners_core::types::{BlockContent, BlockIdentity, NodeId};

use cordial_f1r3node_adapter::grpc_ingest::{BlocklaceAdapter, GrpcBlockMapper};

// ── Test Helpers ─────────────────────────────────────────────────────────

/// Generate a 32-byte ED25519 signing key from a single byte seed.
/// **WARNING**: For testing only! Not suitable for production.
fn test_signing_key(seed: u8) -> [u8; 32] {
    let mut key = [0u8; 32];
    key[0] = seed;
    key
}

/// Derive the ED25519 public key from a signing key.
fn test_public_key(signing_key: &[u8; 32]) -> Vec<u8> {
    use ed25519_dalek::SigningKey;
    let sk = SigningKey::from_bytes(signing_key);
    sk.verifying_key().to_bytes().to_vec()
}

/// Build a test block with a given creator and predecessors.
fn build_test_block(
    creator: NodeId,
    payload: Vec<u8>,
    predecessors: HashSet<BlockIdentity>,
    signing_key: &[u8; 32],
) -> Block {
    let content = BlockContent {
        payload,
        predecessors,
    };
    let content_hash = hash_content(&content);
    let signature = sign(&content_hash, signing_key);

    Block {
        identity: BlockIdentity {
            content_hash,
            creator,
            signature,
        },
        content,
    }
}

// ── Mock Adapter for Integration Testing ──────────────────────────────────

/// A mock blocklace adapter that records all received blocks and callbacks.
struct RecordingAdapter {
    received_blocks: Vec<Block>,
    callback_count: usize,
    should_reject_next: bool,
}

impl RecordingAdapter {
    fn new() -> Self {
        Self {
            received_blocks: Vec::new(),
            callback_count: 0,
            should_reject_next: false,
        }
    }

    fn received_blocks(&self) -> &[Block] {
        &self.received_blocks
    }

    fn callback_count(&self) -> usize {
        self.callback_count
    }

    fn reject_next_block(&mut self) {
        self.should_reject_next = true;
    }
}

impl BlocklaceAdapter<BlockIdentity> for RecordingAdapter {
    fn on_block(&mut self, block: Block) -> anyhow::Result<()> {
        self.callback_count += 1;

        if self.should_reject_next {
            self.should_reject_next = false;
            return Err(anyhow::anyhow!("Adapter rejected block"));
        }

        self.received_blocks.push(block);
        Ok(())
    }
}

// ── Integration Tests ────────────────────────────────────────────────────

#[test]
fn full_pipeline_valid_block_from_network_to_adapter() {
    let mapper: GrpcBlockMapper<(), (), ()> = GrpcBlockMapper::new();
    let mut adapter = RecordingAdapter::new();

    let signing_key = test_signing_key(1);
    let creator = NodeId(test_public_key(&signing_key));
    let block = build_test_block(creator, vec![0x42; 16], HashSet::new(), &signing_key);

    let network_msg = Message::BroadcastBlock {
        block: block.clone(),
    };

    // Step 1: Mapper validates and extracts block
    let mapped = mapper
        .to_block(&network_msg)
        .expect("Mapper should accept valid block");

    // Step 2: Adapter receives and records block
    adapter
        .on_block(mapped)
        .expect("Adapter should accept structurally valid block");

    // Verify results
    assert_eq!(adapter.callback_count(), 1);
    assert_eq!(adapter.received_blocks().len(), 1);
    assert_eq!(
        adapter.received_blocks()[0].identity.content_hash,
        block.identity.content_hash
    );
}

#[test]
fn pipeline_rejects_non_broadcast_block_messages() {
    let mapper: GrpcBlockMapper<(), (), ()> = GrpcBlockMapper::new();
    let adapter = RecordingAdapter::new();

    let non_block_messages = vec![
        Message::Ping,
        Message::Pong,
        Message::Hello {
            node_id: vec![1, 2, 3],
            listen_port: 8080,
        },
        Message::HelloAck {
            node_id: vec![1, 2, 3],
        },
        Message::SyncRequest,
        Message::RequestBlock {
            id: BlockIdentity {
                content_hash: [0u8; 32],
                creator: NodeId(vec![0u8; 32]),
                signature: vec![0u8; 64],
            },
        },
    ];

    for msg in non_block_messages {
        let result = mapper.to_block(&msg);
        assert!(
            result.is_err(),
            "Mapper should reject non-BroadcastBlock: {:?}",
            msg
        );
        assert_eq!(
            adapter.callback_count(),
            0,
            "Adapter should never be called for rejected messages"
        );
    }
}

#[test]
fn pipeline_rejects_corrupted_blocks_before_adapter() {
    let mapper: GrpcBlockMapper<(), (), ()> = GrpcBlockMapper::new();
    let adapter = RecordingAdapter::new();

    let signing_key = test_signing_key(1);
    let creator = NodeId(test_public_key(&signing_key));
    let mut block = build_test_block(creator, vec![0x42; 16], HashSet::new(), &signing_key);

    // Corrupt the signature
    block.identity.signature[0] ^= 0xFF;

    let network_msg = Message::BroadcastBlock { block };

    // Mapper should reject
    let result = mapper.to_block(&network_msg);
    assert!(result.is_err(), "Mapper should reject corrupted block");

    // Adapter should never be called
    assert_eq!(adapter.callback_count(), 0);
}

#[test]
fn pipeline_sequence_multiple_valid_blocks() {
    let mapper: GrpcBlockMapper<(), (), ()> = GrpcBlockMapper::new();
    let mut adapter = RecordingAdapter::new();

    // Create multiple blocks from different creators
    let blocks: Vec<_> = (1u8..=5)
        .map(|seed| {
            let signing_key = test_signing_key(seed);
            let creator = NodeId(test_public_key(&signing_key));
            build_test_block(creator, vec![seed; 16], HashSet::new(), &signing_key)
        })
        .collect();

    // Process each block through the pipeline
    for block in &blocks {
        let msg = Message::BroadcastBlock {
            block: block.clone(),
        };
        let mapped = mapper.to_block(&msg).expect("Valid block should map");
        adapter.on_block(mapped).expect("Adapter should accept");
    }

    // Verify all blocks were received in order
    assert_eq!(adapter.callback_count(), 5);
    assert_eq!(adapter.received_blocks().len(), 5);

    for (i, block) in blocks.iter().enumerate() {
        assert_eq!(
            adapter.received_blocks()[i].identity.content_hash,
            block.identity.content_hash,
            "Block {} content hash mismatch",
            i
        );
    }
}

#[test]
fn pipeline_with_block_predecessors() {
    let mapper: GrpcBlockMapper<(), (), ()> = GrpcBlockMapper::new();
    let mut adapter = RecordingAdapter::new();

    // Create genesis block
    let signing_key_1 = test_signing_key(1);
    let creator_1 = NodeId(test_public_key(&signing_key_1));
    let genesis = build_test_block(creator_1, vec![1, 2, 3], HashSet::new(), &signing_key_1);

    // Process genesis
    let genesis_msg = Message::BroadcastBlock {
        block: genesis.clone(),
    };
    mapper
        .to_block(&genesis_msg)
        .and_then(|b| adapter.on_block(b))
        .expect("Genesis should be accepted");

    // Create child block with genesis as predecessor
    let signing_key_2 = test_signing_key(2);
    let creator_2 = NodeId(test_public_key(&signing_key_2));
    let mut preds = HashSet::new();
    preds.insert(genesis.identity.clone());

    let child = build_test_block(creator_2, vec![4, 5, 6], preds, &signing_key_2);

    // Process child
    let child_msg = Message::BroadcastBlock {
        block: child.clone(),
    };
    mapper
        .to_block(&child_msg)
        .and_then(|b| adapter.on_block(b))
        .expect("Child should be accepted");

    // Verify both blocks recorded
    assert_eq!(adapter.callback_count(), 2);
    assert_eq!(
        adapter.received_blocks()[0].identity.content_hash,
        genesis.identity.content_hash
    );
    assert_eq!(
        adapter.received_blocks()[1].identity.content_hash,
        child.identity.content_hash
    );
    assert_eq!(adapter.received_blocks()[1].content.predecessors.len(), 1);
}

#[test]
fn adapter_can_reject_valid_blocks() {
    let mapper: GrpcBlockMapper<(), (), ()> = GrpcBlockMapper::new();
    let mut adapter = RecordingAdapter::new();

    let signing_key = test_signing_key(1);
    let creator = NodeId(test_public_key(&signing_key));
    let block = build_test_block(creator, vec![0x42; 16], HashSet::new(), &signing_key);

    let msg = Message::BroadcastBlock {
        block: block.clone(),
    };

    // Mapper accepts
    let mapped = mapper.to_block(&msg).expect("Valid block");

    // Adapter can reject even structurally valid blocks
    adapter.reject_next_block();
    let result = adapter.on_block(mapped);
    assert!(result.is_err(), "Adapter should be able to reject blocks");
}

#[test]
fn mapper_determinism_across_instances() {
    let mapper1: GrpcBlockMapper<(), (), ()> = GrpcBlockMapper::new();
    let mapper2: GrpcBlockMapper<(), (), ()> = GrpcBlockMapper::new();

    let signing_key = test_signing_key(42);
    let creator = NodeId(test_public_key(&signing_key));
    let block = build_test_block(creator, vec![99], HashSet::new(), &signing_key);

    let msg = Message::BroadcastBlock {
        block: block.clone(),
    };

    let result1 = mapper1.to_block(&msg).expect("mapper1 maps");
    let result2 = mapper2.to_block(&msg).expect("mapper2 maps");

    // Both instances should produce identical results
    assert_eq!(result1.identity.content_hash, result2.identity.content_hash);
    assert_eq!(result1.identity.creator, result2.identity.creator);
    assert_eq!(result1.content.payload, result2.content.payload);
}

#[test]
fn mapper_idempotence_same_instance() {
    let mapper: GrpcBlockMapper<(), (), ()> = GrpcBlockMapper::new();

    let signing_key = test_signing_key(7);
    let creator = NodeId(test_public_key(&signing_key));
    let block = build_test_block(creator, vec![13, 14, 15], HashSet::new(), &signing_key);

    let msg = Message::BroadcastBlock {
        block: block.clone(),
    };

    // Map same message multiple times
    let r1 = mapper.to_block(&msg).expect("First map");
    let r2 = mapper.to_block(&msg).expect("Second map");
    let r3 = mapper.to_block(&msg).expect("Third map");

    // All results should be identical
    assert_eq!(r1.identity, r2.identity);
    assert_eq!(r2.identity, r3.identity);
    assert_eq!(r1.content.payload, r2.content.payload);
    assert_eq!(r2.content.payload, r3.content.payload);
    assert_eq!(r1.content.predecessors, r2.content.predecessors);
    assert_eq!(r2.content.predecessors, r3.content.predecessors);
}

#[test]
fn error_messages_are_descriptive() {
    let mapper: GrpcBlockMapper<(), (), ()> = GrpcBlockMapper::new();

    // Test 1: Wrong message type
    let result = mapper.to_block(&Message::Ping);
    assert!(result.is_err());
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("BroadcastBlock"),
        "Error should mention expected type: {}",
        err_msg
    );

    // Test 2: Corrupted content hash
    let signing_key = test_signing_key(1);
    let creator = NodeId(test_public_key(&signing_key));
    let mut block = build_test_block(creator, vec![0x42; 16], HashSet::new(), &signing_key);
    block.identity.content_hash[0] ^= 0xFF;

    let msg = Message::BroadcastBlock { block };
    let result = mapper.to_block(&msg);
    assert!(result.is_err());
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("hash mismatch"),
        "Error should describe hash mismatch: {}",
        err_msg
    );

    // Test 3: Invalid signature
    let signing_key = test_signing_key(1);
    let creator = NodeId(test_public_key(&signing_key));
    let mut block = build_test_block(creator, vec![0x42; 16], HashSet::new(), &signing_key);
    block.identity.signature[0] ^= 0xFF;

    let msg = Message::BroadcastBlock { block };
    let result = mapper.to_block(&msg);
    assert!(result.is_err());
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("Signature"),
        "Error should mention signature: {}",
        err_msg
    );
}

#[test]
fn complex_predecessor_chain() {
    let mapper: GrpcBlockMapper<(), (), ()> = GrpcBlockMapper::new();
    let mut adapter = RecordingAdapter::new();

    // Build a chain: genesis -> child1 -> child2 -> child3
    let mut current_block = {
        let signing_key = test_signing_key(1);
        let creator = NodeId(test_public_key(&signing_key));
        build_test_block(creator, vec![1], HashSet::new(), &signing_key)
    };

    let mut blocks = vec![current_block.clone()];

    for seed in 2u8..=4 {
        let signing_key = test_signing_key(seed);
        let creator = NodeId(test_public_key(&signing_key));
        let mut preds = HashSet::new();
        preds.insert(current_block.identity.clone());

        current_block = build_test_block(creator, vec![seed], preds, &signing_key);
        blocks.push(current_block.clone());
    }

    // Process all blocks
    for block in &blocks {
        let msg = Message::BroadcastBlock {
            block: block.clone(),
        };
        mapper
            .to_block(&msg)
            .and_then(|b| adapter.on_block(b))
            .expect("Block in chain should be accepted");
    }

    // Verify complete chain recorded
    assert_eq!(adapter.received_blocks().len(), 4);

    for (i, block) in blocks.iter().enumerate() {
        assert_eq!(
            adapter.received_blocks()[i].identity.content_hash,
            block.identity.content_hash,
            "Block {} in chain",
            i
        );
    }

    // Verify predecessor relationships preserved
    assert_eq!(
        adapter.received_blocks()[0].content.predecessors.len(),
        0,
        "Genesis has no predecessors"
    );
    for i in 1..4 {
        assert_eq!(
            adapter.received_blocks()[i].content.predecessors.len(),
            1,
            "Block {} should have exactly 1 predecessor",
            i
        );
    }
}

// ── Unit Tests from grpc_ingest.rs ───────────────────────────────────────

// ── Valid Block Unit Tests ───────────────────────────────────────────────

#[test]
fn valid_genesis_block_maps_to_block() {
    let mapper: GrpcBlockMapper<(), (), ()> = GrpcBlockMapper::new();
    let signing_key = test_signing_key(1);
    let creator = NodeId(test_public_key(&signing_key));

    let block = build_test_block(creator, vec![0x42; 16], HashSet::new(), &signing_key);
    let msg = Message::BroadcastBlock {
        block: block.clone(),
    };

    let result = mapper.to_block(&msg);
    assert!(result.is_ok());
    let mapped = result.unwrap();
    assert_eq!(mapped.identity.content_hash, block.identity.content_hash);
    assert_eq!(mapped.identity.creator, block.identity.creator);
    assert_eq!(mapped.content.payload, block.content.payload);
    assert_eq!(mapped.content.predecessors, block.content.predecessors);
}

#[test]
fn valid_block_with_predecessors_maps_deterministically() {
    let mapper: GrpcBlockMapper<(), (), ()> = GrpcBlockMapper::new();

    // Create a genesis block
    let signing_key_1 = test_signing_key(1);
    let creator_1 = NodeId(test_public_key(&signing_key_1));
    let genesis = build_test_block(creator_1, vec![1, 2, 3], HashSet::new(), &signing_key_1);

    // Create a second block with genesis as predecessor
    let signing_key_2 = test_signing_key(2);
    let creator_2 = NodeId(test_public_key(&signing_key_2));
    let mut preds = HashSet::new();
    preds.insert(genesis.identity.clone());

    let block_2 = build_test_block(creator_2, vec![4, 5, 6], preds, &signing_key_2);
    let msg = Message::BroadcastBlock {
        block: block_2.clone(),
    };

    let result = mapper.to_block(&msg);
    assert!(result.is_ok());

    // Verify idempotence: same input → same output
    let msg2 = Message::BroadcastBlock {
        block: block_2.clone(),
    };
    let result2 = mapper.to_block(&msg2);
    assert!(result2.is_ok());
    assert_eq!(result.unwrap().identity, result2.unwrap().identity);
}

#[test]
fn mapper_is_stateless_and_idempotent() {
    let mapper1: GrpcBlockMapper<(), (), ()> = GrpcBlockMapper::new();
    let mapper2: GrpcBlockMapper<(), (), ()> = GrpcBlockMapper::new();

    let signing_key = test_signing_key(42);
    let creator = NodeId(test_public_key(&signing_key));
    let block = build_test_block(creator, vec![99], HashSet::new(), &signing_key);
    let msg = Message::BroadcastBlock {
        block: block.clone(),
    };

    // Same message mapped by different mapper instances
    let r1 = mapper1.to_block(&msg).unwrap();
    let r2 = mapper2.to_block(&msg).unwrap();
    assert_eq!(r1.identity, r2.identity);

    // Same mapper, same message, multiple times
    let r3 = mapper1.to_block(&msg).unwrap();
    let r4 = mapper1.to_block(&msg).unwrap();
    assert_eq!(r3.identity, r4.identity);
}

// ── Invalid Message Type Unit Tests ──────────────────────────────────────

#[test]
fn non_broadcast_block_message_rejected() {
    let mapper: GrpcBlockMapper<(), (), ()> = GrpcBlockMapper::new();
    let msg = Message::Ping;
    assert!(mapper.to_block(&msg).is_err());
}

#[test]
fn hello_message_rejected() {
    let mapper: GrpcBlockMapper<(), (), ()> = GrpcBlockMapper::new();
    let msg = Message::Hello {
        node_id: vec![1, 2, 3],
        listen_port: 8080,
    };
    assert!(mapper.to_block(&msg).is_err());
}

#[test]
fn sync_request_message_rejected() {
    let mapper: GrpcBlockMapper<(), (), ()> = GrpcBlockMapper::new();
    let msg = Message::SyncRequest;
    assert!(mapper.to_block(&msg).is_err());
}

// ── Content Hash Validation Unit Tests ───────────────────────────────────

#[test]
fn block_with_corrupted_content_hash_rejected() {
    let mapper: GrpcBlockMapper<(), (), ()> = GrpcBlockMapper::new();
    let signing_key = test_signing_key(1);
    let creator = NodeId(test_public_key(&signing_key));

    let mut block = build_test_block(creator, vec![0x42; 16], HashSet::new(), &signing_key);
    // Corrupt the content hash
    block.identity.content_hash[0] ^= 0xFF;

    let msg = Message::BroadcastBlock { block };
    let result = mapper.to_block(&msg);
    assert!(result.is_err());
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("Content hash mismatch")
    );
}

// ── Signature Validation Unit Tests ──────────────────────────────────────

#[test]
fn block_with_invalid_signature_rejected() {
    let mapper: GrpcBlockMapper<(), (), ()> = GrpcBlockMapper::new();
    let signing_key = test_signing_key(1);
    let creator = NodeId(test_public_key(&signing_key));

    let mut block = build_test_block(
        creator.clone(),
        vec![0x42; 16],
        HashSet::new(),
        &signing_key,
    );
    // Corrupt the signature
    block.identity.signature[0] ^= 0xFF;

    let msg = Message::BroadcastBlock { block };
    let result = mapper.to_block(&msg);
    assert!(result.is_err());
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("Signature verification failed")
    );
}

#[test]
fn block_with_wrong_creator_key_rejected() {
    let mapper: GrpcBlockMapper<(), (), ()> = GrpcBlockMapper::new();
    let signing_key_1 = test_signing_key(1);
    let signing_key_2 = test_signing_key(2);
    let creator_1 = NodeId(test_public_key(&signing_key_1));
    let creator_2 = NodeId(test_public_key(&signing_key_2));

    let mut block = build_test_block(creator_1, vec![0x42; 16], HashSet::new(), &signing_key_1);
    // Change the creator to a different key, but keep the old signature
    block.identity.creator = creator_2;

    let msg = Message::BroadcastBlock { block };
    let result = mapper.to_block(&msg);
    assert!(result.is_err());
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("Signature verification failed")
    );
}

#[test]
fn block_with_short_signature_rejected() {
    let mapper: GrpcBlockMapper<(), (), ()> = GrpcBlockMapper::new();
    let signing_key = test_signing_key(1);
    let creator = NodeId(test_public_key(&signing_key));

    let mut block = build_test_block(creator, vec![0x42; 16], HashSet::new(), &signing_key);
    // Truncate the signature
    block.identity.signature.truncate(32);

    let msg = Message::BroadcastBlock { block };
    let result = mapper.to_block(&msg);
    assert!(result.is_err());
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("Invalid signature")
    );
}

#[test]
fn block_with_short_creator_key_rejected() {
    let mapper: GrpcBlockMapper<(), (), ()> = GrpcBlockMapper::new();
    let signing_key = test_signing_key(1);

    let mut block = build_test_block(
        NodeId(test_public_key(&signing_key)),
        vec![0x42; 16],
        HashSet::new(),
        &signing_key,
    );
    // Truncate the creator key
    block.identity.creator.0.truncate(16);

    let msg = Message::BroadcastBlock { block };
    let result = mapper.to_block(&msg);
    assert!(result.is_err());
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("Invalid creator public key")
    );
}

// ── Parent Validation Unit Tests ─────────────────────────────────────────

#[test]
fn block_with_empty_signature_in_parent_rejected() {
    let mapper: GrpcBlockMapper<(), (), ()> = GrpcBlockMapper::new();
    let signing_key = test_signing_key(1);
    let creator = NodeId(test_public_key(&signing_key));

    // Create a parent with an empty signature
    let invalid_parent = BlockIdentity {
        content_hash: [0u8; 32],
        creator: creator.clone(),
        signature: vec![], // Invalid: empty
    };

    let mut preds = HashSet::new();
    preds.insert(invalid_parent);

    let block = build_test_block(creator, vec![0x42; 16], preds, &signing_key);
    let msg = Message::BroadcastBlock { block };
    let result = mapper.to_block(&msg);
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("empty signature"));
}

#[test]
fn block_with_malformed_parent_key_rejected() {
    let mapper: GrpcBlockMapper<(), (), ()> = GrpcBlockMapper::new();
    let signing_key = test_signing_key(1);
    let creator = NodeId(test_public_key(&signing_key));

    // Create a parent with a malformed (short) creator key
    let invalid_parent = BlockIdentity {
        content_hash: [0u8; 32],
        creator: NodeId(vec![1, 2, 3]), // Invalid: too short
        signature: vec![0u8; 64],
    };

    let mut preds = HashSet::new();
    preds.insert(invalid_parent);

    let block = build_test_block(creator, vec![0x42; 16], preds, &signing_key);
    let msg = Message::BroadcastBlock { block };
    let result = mapper.to_block(&msg);
    assert!(result.is_err());
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("Parent creator has invalid key size")
    );
}

// ── Mock Adapter Unit Tests ──────────────────────────────────────────────

struct MockBlocklaceAdapter {
    blocks_received: Vec<Block>,
    callback_count: usize,
}

impl MockBlocklaceAdapter {
    fn new() -> Self {
        Self {
            blocks_received: Vec::new(),
            callback_count: 0,
        }
    }

    fn received_blocks(&self) -> &[Block] {
        &self.blocks_received
    }

    fn callback_count(&self) -> usize {
        self.callback_count
    }
}

impl BlocklaceAdapter<BlockIdentity> for MockBlocklaceAdapter {
    fn on_block(&mut self, block: Block) -> anyhow::Result<()> {
        self.blocks_received.push(block);
        self.callback_count += 1;
        Ok(())
    }
}

#[test]
fn valid_block_triggers_on_block_callback() {
    let mapper: GrpcBlockMapper<(), (), ()> = GrpcBlockMapper::new();
    let mut adapter = MockBlocklaceAdapter::new();

    let signing_key = test_signing_key(1);
    let creator = NodeId(test_public_key(&signing_key));
    let block = build_test_block(creator, vec![0x42; 16], HashSet::new(), &signing_key);
    let msg = Message::BroadcastBlock {
        block: block.clone(),
    };

    let mapped = mapper.to_block(&msg).unwrap();
    adapter.on_block(mapped).unwrap();

    assert_eq!(adapter.callback_count(), 1);
    assert_eq!(adapter.received_blocks().len(), 1);
    assert_eq!(
        adapter.received_blocks()[0].identity.content_hash,
        block.identity.content_hash
    );
}

#[test]
fn multiple_valid_blocks_trigger_multiple_callbacks() {
    let mapper: GrpcBlockMapper<(), (), ()> = GrpcBlockMapper::new();
    let mut adapter = MockBlocklaceAdapter::new();

    let signing_key_1 = test_signing_key(1);
    let creator_1 = NodeId(test_public_key(&signing_key_1));
    let block_1 = build_test_block(creator_1, vec![1], HashSet::new(), &signing_key_1);

    let signing_key_2 = test_signing_key(2);
    let creator_2 = NodeId(test_public_key(&signing_key_2));
    let block_2 = build_test_block(creator_2, vec![2], HashSet::new(), &signing_key_2);

    let msg_1 = Message::BroadcastBlock {
        block: block_1.clone(),
    };
    let msg_2 = Message::BroadcastBlock {
        block: block_2.clone(),
    };

    let mapped_1 = mapper.to_block(&msg_1).unwrap();
    let mapped_2 = mapper.to_block(&msg_2).unwrap();

    adapter.on_block(mapped_1).unwrap();
    adapter.on_block(mapped_2).unwrap();

    assert_eq!(adapter.callback_count(), 2);
    assert_eq!(adapter.received_blocks().len(), 2);
}

#[test]
fn adapter_fails_to_receive_invalid_block() {
    let mapper: GrpcBlockMapper<(), (), ()> = GrpcBlockMapper::new();
    let adapter = MockBlocklaceAdapter::new();

    let signing_key = test_signing_key(1);
    let creator = NodeId(test_public_key(&signing_key));

    let mut block = build_test_block(creator, vec![0x42; 16], HashSet::new(), &signing_key);
    block.identity.signature[0] ^= 0xFF; // Corrupt signature

    let msg = Message::BroadcastBlock { block };
    let result = mapper.to_block(&msg);

    // Mapper should reject before adapter sees it
    assert!(result.is_err());
    assert_eq!(adapter.callback_count(), 0);
}
