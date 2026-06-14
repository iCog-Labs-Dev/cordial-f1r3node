use std::collections::HashMap;

use cordial_f1r3node_adapter::block_translation::{
    BlockMessage, Body, F1r3flyState, Header, Justification,
};
use cordial_f1r3node_adapter::grpc_ingest::BlocklaceAdapter;
use cordial_f1r3node_adapter::http_observer::{
    HttpBlockInfo, HttpJustificationInfo, HttpLightBlockInfo, compare_mirror_against_http,
};
use cordial_f1r3node_adapter::live_ingress::LiveIngress;
use cordial_f1r3node_adapter::shard_conf::CasperShardConf;
use cordial_miners_core::Block;
use cordial_miners_core::crypto::{hash_content, sign};
use cordial_miners_core::execution::{BlockState, CordialBlockPayload};
use cordial_miners_core::types::{BlockContent, BlockIdentity, NodeId};

#[test]
#[allow(clippy::field_reassign_with_default)]
fn compare_mirror_against_http_reports_matching_view() {
    let mut ingress = finalized_wave_ingress();
    let (_leader, blocks) = finalized_wave_messages();

    for block in &blocks {
        ingress
            .ingest_block_message(block)
            .expect("wave should ingest");
    }

    let http_blocks: Vec<HttpLightBlockInfo> = blocks
        .iter()
        .map(|block| HttpLightBlockInfo {
            block_hash: hex(&block.block_hash),
            sender: hex(&block.sender),
            seq_num: i64::from(block.seq_num),
            sig: hex(&block.sig),
            sig_algorithm: block.sig_algorithm.clone(),
            shard_id: block.shard_id.clone(),
            extra_bytes: block.extra_bytes.clone(),
            version: block.header.version,
            timestamp: block.header.timestamp,
            header_extra_bytes: block.header.extra_bytes.clone(),
            parents_hash_list: block
                .header
                .parents_hash_list
                .iter()
                .map(|h| hex(h))
                .collect(),
            block_number: block.body.state.block_number,
            pre_state_hash: hex(&block.body.state.pre_state_hash),
            post_state_hash: hex(&block.body.state.post_state_hash),
            body_extra_bytes: block.body.extra_bytes.clone(),
            bonds: vec![],
            block_size: "0".to_string(),
            deploy_count: 0,
            fault_tolerance: 0.0,
            justifications: block
                .justifications
                .iter()
                .map(|j| HttpJustificationInfo {
                    validator: hex(&j.validator),
                    latest_block_hash: hex(&j.latest_block_hash),
                })
                .collect(),
            rejected_deploys: vec![],
        })
        .collect();

    let mirror_lfb = ingress
        .last_finalized_block_hash()
        .expect("lfb lookup should succeed")
        .expect("fixture should finalize some leader");
    let http_lfb = HttpBlockInfo {
        block_info: http_blocks
            .iter()
            .find(|block| block.block_hash == hex(&mirror_lfb))
            .expect("mirror lfb should be present in http block list")
            .clone(),
        deploys: vec![],
    };

    let report = compare_mirror_against_http(&ingress, &http_blocks, Some(&http_lfb));

    assert!(report.is_match());
    assert!(report.missing_from_http.is_empty());
    assert!(report.missing_from_mirror.is_empty());
    assert!(report.last_finalized_matches);
}

#[test]
#[allow(clippy::field_reassign_with_default)]
fn compare_mirror_against_http_reports_missing_blocks_and_lfb_mismatch() {
    let mut ingress = finalized_wave_ingress();
    let (leader, blocks) = finalized_wave_messages();

    for block in &blocks {
        ingress
            .ingest_block_message(block)
            .expect("wave should ingest");
    }

    let http_blocks = vec![HttpLightBlockInfo {
        block_hash: hex(&leader.block_hash),
        ..HttpLightBlockInfo::default()
    }];
    let http_lfb = HttpBlockInfo {
        block_info: HttpLightBlockInfo {
            block_hash: "deadbeef".to_string(),
            ..HttpLightBlockInfo::default()
        },
        deploys: vec![],
    };

    let report = compare_mirror_against_http(&ingress, &http_blocks, Some(&http_lfb));

    assert!(!report.is_match());
    assert!(!report.missing_from_http.is_empty());
    assert!(report.missing_from_mirror.is_empty());
    assert!(!report.last_finalized_matches);
}

fn finalized_wave_ingress() -> LiveIngress<RecordingAdapter> {
    let mut bonds = HashMap::new();
    let signing_key_1 = test_signing_key(21);
    let creator_1 = test_public_key(&signing_key_1);
    let signing_key_2 = test_signing_key(22);
    let creator_2 = test_public_key(&signing_key_2);
    let signing_key_3 = test_signing_key(23);
    let creator_3 = test_public_key(&signing_key_3);
    let signing_key_4 = test_signing_key(24);
    let creator_4 = test_public_key(&signing_key_4);

    bonds.insert(NodeId(creator_1), 100);
    bonds.insert(NodeId(creator_2), 100);
    bonds.insert(NodeId(creator_3), 100);
    bonds.insert(NodeId(creator_4), 100);

    let shard_conf = CasperShardConf {
        shard_name: "root".to_string(),
        max_number_of_parents: 16,
        fault_tolerance_threshold: 0.333,
        deploy_lifespan: 50,
        min_phlo_price: 1,
        ..CasperShardConf::default()
    };

    LiveIngress::with_consensus_view(RecordingAdapter, bonds, shard_conf, "root")
}

fn finalized_wave_messages() -> (BlockMessage, Vec<BlockMessage>) {
    let signing_key_1 = test_signing_key(21);
    let creator_1 = test_public_key(&signing_key_1);
    let signing_key_2 = test_signing_key(22);
    let creator_2 = test_public_key(&signing_key_2);
    let signing_key_3 = test_signing_key(23);
    let creator_3 = test_public_key(&signing_key_3);
    let signing_key_4 = test_signing_key(24);
    let creator_4 = test_public_key(&signing_key_4);

    let leader =
        build_test_block_message_with_state(&creator_1, &[], &signing_key_1, "secp256k1", 0, 1);

    let round1_v2 = build_test_block_message_with_state(
        &creator_2,
        &[(leader.block_hash.clone(), leader.sender.clone())],
        &signing_key_2,
        "secp256k1",
        1,
        2,
    );
    let round1_v3 = build_test_block_message_with_state(
        &creator_3,
        &[(leader.block_hash.clone(), leader.sender.clone())],
        &signing_key_3,
        "secp256k1",
        1,
        3,
    );
    let round1_v4 = build_test_block_message_with_state(
        &creator_4,
        &[(leader.block_hash.clone(), leader.sender.clone())],
        &signing_key_4,
        "secp256k1",
        1,
        4,
    );

    let round1_support = [
        (round1_v2.block_hash.clone(), round1_v2.sender.clone()),
        (round1_v3.block_hash.clone(), round1_v3.sender.clone()),
        (round1_v4.block_hash.clone(), round1_v4.sender.clone()),
    ];

    let round2_v2 = build_test_block_message_with_state(
        &creator_2,
        &round1_support,
        &signing_key_2,
        "secp256k1",
        2,
        5,
    );
    let round2_v3 = build_test_block_message_with_state(
        &creator_3,
        &round1_support,
        &signing_key_3,
        "secp256k1",
        2,
        6,
    );
    let round2_v4 = build_test_block_message_with_state(
        &creator_4,
        &round1_support,
        &signing_key_4,
        "secp256k1",
        2,
        7,
    );

    (
        leader.clone(),
        vec![
            round2_v3, round1_v2, round2_v2, leader, round1_v4, round2_v4, round1_v3,
        ],
    )
}

struct RecordingAdapter;

impl BlocklaceAdapter<BlockIdentity> for RecordingAdapter {
    fn on_block(&mut self, _block: Block) -> anyhow::Result<()> {
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
    sk.verifying_key()
        .to_encoded_point(true)
        .as_bytes()
        .to_vec()
}

fn build_test_block_message_with_state(
    creator: &[u8],
    parents: &[(Vec<u8>, Vec<u8>)],
    signing_key: &[u8],
    sig_algorithm: &str,
    block_number: u64,
    state_tag: u8,
) -> BlockMessage {
    let justifications: Vec<Justification> = parents
        .iter()
        .filter(|(hash, _)| hash.len() == 32)
        .map(|(hash, validator)| Justification {
            validator: validator.clone(),
            latest_block_hash: hash.clone(),
        })
        .collect();

    let payload = CordialBlockPayload {
        state: BlockState {
            pre_state_hash: vec![state_tag; 32],
            post_state_hash: vec![state_tag.wrapping_add(1); 32],
            bonds: vec![],
            block_number,
        },
        deploys: vec![],
        rejected_deploys: vec![],
        system_deploys: vec![],
    };
    let payload_bytes = payload.to_bytes();

    let mut predecessors = std::collections::HashSet::new();
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
            parents_hash_list: parents.iter().map(|(hash, _)| hash.clone()).collect(),
            timestamp: 0,
            version: 1,
            extra_bytes: vec![],
        },
        body: Body {
            state: F1r3flyState {
                pre_state_hash: vec![state_tag; 32],
                post_state_hash: vec![state_tag.wrapping_add(1); 32],
                bonds: vec![],
                block_number: i64::try_from(block_number).expect("test block number should fit"),
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
        shard_id: "root".to_string(),
        extra_bytes: vec![],
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
