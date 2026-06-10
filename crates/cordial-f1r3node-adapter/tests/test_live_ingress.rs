use std::collections::HashSet;

use cordial_f1r3node_adapter::block_translation::{
    BlockMessage, Body, F1r3flyState, Header, Justification,
};
use cordial_f1r3node_adapter::grpc_ingest::BlocklaceAdapter;
use cordial_f1r3node_adapter::live_ingress::{LiveIngress, LiveIngressError, LiveIngressPhase};
use cordial_miners_core::Block;
use cordial_miners_core::crypto::{hash_content, sign};
use cordial_miners_core::execution::{BlockState, CordialBlockPayload};
use cordial_miners_core::types::{BlockContent, BlockIdentity, NodeId};

#[test]
fn new_live_ingress_starts_in_defined_phase() {
    let ingress = LiveIngress::new(());
    assert_eq!(ingress.phase(), LiveIngressPhase::Defined);
}

#[test]
fn live_ingress_phase_can_progress_without_changing_adapter() {
    let mut ingress = LiveIngress::new(String::from("adapter"));

    ingress.mark_traced();
    assert_eq!(ingress.phase(), LiveIngressPhase::Traced);
    assert_eq!(ingress.adapter(), "adapter");

    ingress.mark_connected();
    assert_eq!(ingress.phase(), LiveIngressPhase::Connected);
    assert_eq!(ingress.into_inner(), "adapter");
}

#[test]
fn live_ingress_routes_valid_block_messages_through_grpc_ingest() {
    let signing_key = test_signing_key(7);
    let creator = test_public_key(&signing_key);
    let block_msg = build_test_block_message(&creator, &[], &signing_key, "secp256k1");

    let mut ingress = LiveIngress::new(RecordingAdapter::default());
    let block = ingress
        .ingest_block_message(&block_msg)
        .expect("live ingress should accept a valid block message");

    assert_eq!(ingress.phase(), LiveIngressPhase::Connected);
    assert_eq!(
        block.identity.content_hash,
        <[u8; 32]>::try_from(block_msg.block_hash.as_slice()).expect("valid 32-byte block hash")
    );
    assert_eq!(ingress.adapter().callback_count, 1);
    assert_eq!(ingress.adapter().received_blocks.len(), 1);
}

#[test]
fn live_ingress_surfaces_adapter_rejections_after_mapping() {
    let signing_key = test_signing_key(9);
    let creator = test_public_key(&signing_key);
    let block_msg = build_test_block_message(&creator, &[], &signing_key, "secp256k1");

    let mut ingress = LiveIngress::new(RecordingAdapter::rejecting());
    let err = ingress
        .ingest_block_message(&block_msg)
        .expect_err("adapter rejection should be surfaced");

    match err {
        LiveIngressError::Adapter(inner) => {
            assert!(inner.to_string().contains("Adapter rejected block"));
        }
        other => panic!("expected adapter error, got {other:?}"),
    }

    assert_eq!(ingress.phase(), LiveIngressPhase::Defined);
    assert_eq!(ingress.adapter().callback_count, 1);
    assert!(ingress.adapter().received_blocks.is_empty());
}

#[derive(Default)]
struct RecordingAdapter {
    received_blocks: Vec<Block>,
    callback_count: usize,
    reject: bool,
}

impl RecordingAdapter {
    fn rejecting() -> Self {
        Self {
            received_blocks: Vec::new(),
            callback_count: 0,
            reject: true,
        }
    }
}

impl BlocklaceAdapter<BlockIdentity> for RecordingAdapter {
    fn on_block(&mut self, block: Block) -> anyhow::Result<()> {
        self.callback_count += 1;

        if self.reject {
            return Err(anyhow::anyhow!("Adapter rejected block"));
        }

        self.received_blocks.push(block);
        Ok(())
    }
}

fn test_signing_key(seed: u8) -> Vec<u8> {
    let mut key = vec![0u8; 32];
    key[0] = seed;
    for (i, item) in key.iter_mut().enumerate().skip(1) {
        *item = ((seed as u16).wrapping_mul(i as u16 + 1)) as u8;
    }
    key
}

fn test_public_key(signing_key: &[u8]) -> Vec<u8> {
    use k256::ecdsa::SigningKey as SecpSigningKey;

    let sk = SecpSigningKey::from_slice(signing_key)
        .expect("failed to create secp256k1 signing key from seed");
    sk.verifying_key().to_encoded_point(true).as_bytes().to_vec()
}

fn build_test_block_message(
    creator: &[u8],
    parent_hashes: &[Vec<u8>],
    signing_key: &[u8],
    sig_algorithm: &str,
) -> BlockMessage {
    let justifications: Vec<Justification> = parent_hashes
        .iter()
        .enumerate()
        .filter(|(_, h)| h.len() == 32)
        .map(|(i, hash)| Justification {
            validator: vec![i as u8; 33],
            latest_block_hash: hash.clone(),
        })
        .collect();

    let payload = CordialBlockPayload {
        state: BlockState {
            pre_state_hash: vec![0u8; 32],
            post_state_hash: vec![1u8; 32],
            bonds: vec![],
            block_number: 0,
        },
        deploys: vec![],
        rejected_deploys: vec![],
        system_deploys: vec![],
    };
    let payload_bytes = payload.to_bytes();

    let mut predecessors = HashSet::new();
    for jus in &justifications {
        let mut hash_array = [0u8; 32];
        hash_array.copy_from_slice(&jus.latest_block_hash);
        predecessors.insert(BlockIdentity {
            content_hash: hash_array,
            creator: NodeId(jus.validator.clone()),
            signature: vec![],
        });
    }

    let content = BlockContent {
        payload: payload_bytes,
        predecessors,
    };

    let content_hash = hash_content(&content);
    let signature = sign(&content_hash, signing_key);

    BlockMessage {
        block_hash: content_hash.to_vec(),
        header: Header {
            parents_hash_list: parent_hashes.to_vec(),
            timestamp: 0,
            version: 1,
            extra_bytes: vec![],
        },
        body: Body {
            state: F1r3flyState {
                pre_state_hash: vec![0u8; 32],
                post_state_hash: vec![1u8; 32],
                bonds: vec![],
                block_number: 0,
            },
            deploys: vec![],
            rejected_deploys: vec![],
            system_deploys: vec![],
            extra_bytes: vec![],
        },
        justifications,
        sender: creator.to_vec(),
        seq_num: 0,
        sig: signature,
        sig_algorithm: sig_algorithm.to_string(),
        shard_id: "0".to_string(),
        extra_bytes: vec![],
    }
}
