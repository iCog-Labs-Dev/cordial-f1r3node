use prost::bytes::Bytes;

use cordial_f1r3node_adapter::live_grpc::{
    light_block_info_to_block_message, trusted_block_from_light_block_info,
};
use models::casper::{BondInfo, JustificationInfo, LightBlockInfo, RejectedDeployInfo};

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[test]
fn trusted_block_from_light_block_info_preserves_identity_and_state() {
    let parent_hash = [7u8; 32];
    let block_hash = [9u8; 32];
    let sender = vec![2u8; 33];
    let parent_creator = vec![3u8; 33];

    let info = LightBlockInfo {
        block_hash: hex(&block_hash),
        sender: hex(&sender),
        seq_num: 4,
        sig: hex(&[5u8; 64]),
        sig_algorithm: "secp256k1".to_string(),
        shard_id: "root".to_string(),
        extra_bytes: Bytes::new(),
        version: 1,
        timestamp: 123,
        header_extra_bytes: Bytes::new(),
        parents_hash_list: vec![hex(&parent_hash)],
        block_number: 11,
        pre_state_hash: hex(&[1u8; 32]),
        post_state_hash: hex(&[2u8; 32]),
        body_extra_bytes: Bytes::new(),
        bonds: vec![BondInfo {
            validator: hex(&sender),
            stake: 100,
        }],
        block_size: "0".to_string(),
        deploy_count: 0,
        fault_tolerance: 0.0,
        justifications: vec![JustificationInfo {
            validator: hex(&parent_creator),
            latest_block_hash: hex(&parent_hash),
        }],
        rejected_deploys: vec![],
    };

    let block = trusted_block_from_light_block_info(&info).expect("trusted block conversion");

    assert_eq!(block.identity.content_hash, block_hash);
    assert_eq!(block.identity.creator.0, sender);
    assert_eq!(block.content.predecessors.len(), 1);

    let predecessor = block
        .content
        .predecessors
        .iter()
        .next()
        .expect("predecessor should exist");
    assert_eq!(predecessor.content_hash, parent_hash);
    assert_eq!(predecessor.creator.0, parent_creator);

    let payload = cordial_miners_core::execution::CordialBlockPayload::from_bytes(&block.content.payload)
        .expect("payload should decode");
    assert_eq!(payload.state.block_number, 11);
    assert_eq!(payload.state.bonds.len(), 1);
    assert_eq!(payload.state.bonds[0].stake, 100);
}

#[test]
fn light_block_info_to_block_message_decodes_live_grpc_view() {
    let block_hash = [8u8; 32];
    let parent_hash = [1u8; 32];
    let sender = vec![2u8; 33];

    let info = LightBlockInfo {
        block_hash: hex(&block_hash),
        sender: hex(&sender),
        seq_num: 7,
        sig: hex(&[4u8; 64]),
        sig_algorithm: "secp256k1".to_string(),
        shard_id: "root".to_string(),
        extra_bytes: Bytes::from_static(b"x"),
        version: 1,
        timestamp: 77,
        header_extra_bytes: Bytes::from_static(b"h"),
        parents_hash_list: vec![hex(&parent_hash)],
        block_number: 5,
        pre_state_hash: hex(&[3u8; 32]),
        post_state_hash: hex(&[6u8; 32]),
        body_extra_bytes: Bytes::from_static(b"b"),
        bonds: vec![],
        block_size: "0".to_string(),
        deploy_count: 0,
        fault_tolerance: 0.0,
        justifications: vec![JustificationInfo {
            validator: hex(&sender),
            latest_block_hash: hex(&parent_hash),
        }],
        rejected_deploys: vec![RejectedDeployInfo {
            sig: "deadbeef".to_string(),
        }],
    };

    let msg = light_block_info_to_block_message(&info).expect("block message conversion");

    assert_eq!(msg.block_hash, block_hash.to_vec());
    assert_eq!(msg.sender, sender);
    assert_eq!(msg.seq_num, 7);
    assert_eq!(msg.header.parents_hash_list, vec![parent_hash.to_vec()]);
    assert_eq!(msg.body.state.block_number, 5);
    assert_eq!(msg.justifications.len(), 1);
    assert!(msg.body.deploys.is_empty());
}
