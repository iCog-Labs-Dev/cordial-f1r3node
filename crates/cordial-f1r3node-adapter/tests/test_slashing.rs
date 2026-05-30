use std::collections::HashSet;

use cordial_f1r3node_adapter::slashing::{F1r3SlashDeployFormatter, SlashDeployFormatter};
use cordial_miners_core::consensus::EquivocationEvidence;
use cordial_miners_core::{Block, BlockContent, BlockIdentity, NodeId};
use models::casper::{
    ProcessedSystemDeployProto, SlashSystemDeployDataProto, SystemDeployDataProto,
    system_deploy_data_proto::SystemDeploy,
};
use prost::Message;
use prost::bytes::Bytes;

fn node(byte: u8) -> NodeId {
    NodeId(vec![byte; 33])
}

fn block(creator: NodeId, tag: u8, signature: Vec<u8>) -> Block {
    let mut content_hash = [0u8; 32];
    content_hash[0] = tag;
    content_hash[31] = tag.wrapping_add(1);

    Block {
        identity: BlockIdentity {
            content_hash,
            creator,
            signature,
        },
        content: BlockContent {
            payload: vec![tag, tag.wrapping_add(2)],
            predecessors: HashSet::new(),
        },
    }
}

fn expected_slash_bytes(invalid_block_hash: [u8; 32], issuer_public_key: &[u8]) -> Vec<u8> {
    ProcessedSystemDeployProto {
        system_deploy: Some(SystemDeployDataProto {
            system_deploy: Some(SystemDeploy::SlashSystemDeploy(
                SlashSystemDeployDataProto {
                    invalid_block_hash: Bytes::copy_from_slice(&invalid_block_hash),
                    issuer_public_key: Bytes::copy_from_slice(issuer_public_key),
                },
            )),
        }),
        deploy_log: vec![],
        error_msg: String::new(),
    }
    .encode_to_vec()
}

#[test]
fn evidence_serializes_to_byte_identical_f1r3fly_slash_payload() {
    let validator = node(7);
    let issuer_public_key = vec![0xaa; 33];
    let lower_hash_block = block(validator.clone(), 0x01, vec![0x10; 64]);
    let higher_hash_block = block(validator.clone(), 0x02, vec![0x20; 64]);
    let evidence = vec![EquivocationEvidence::new(
        validator,
        3,
        vec![higher_hash_block.clone(), lower_hash_block.clone()],
    )];
    let formatter = F1r3SlashDeployFormatter::new(issuer_public_key.clone());

    let slash_deploys = formatter.to_slash_system_deploys(&evidence).unwrap();

    assert_eq!(slash_deploys.len(), 1);
    assert_eq!(
        slash_deploys[0],
        expected_slash_bytes(lower_hash_block.identity.content_hash, &issuer_public_key)
    );

    let decoded = ProcessedSystemDeployProto::decode(slash_deploys[0].as_slice()).unwrap();
    let system_deploy = decoded.system_deploy.unwrap().system_deploy.unwrap();
    match system_deploy {
        SystemDeploy::SlashSystemDeploy(slash) => {
            assert_eq!(
                slash.invalid_block_hash,
                Bytes::copy_from_slice(&lower_hash_block.identity.content_hash)
            );
            assert_eq!(
                slash.issuer_public_key,
                Bytes::copy_from_slice(&issuer_public_key)
            );
        }
        other => panic!("expected slash system deploy, got {other:?}"),
    }
}

#[test]
fn evidence_envelope_preserves_original_block_signatures() {
    let validator = node(9);
    let left = block(validator.clone(), 0x01, vec![0x11, 0x12, 0x13]);
    let right = block(validator.clone(), 0x02, vec![0x21, 0x22, 0x23]);
    let evidence = vec![EquivocationEvidence::new(
        validator,
        4,
        vec![right.clone(), left.clone()],
    )];
    let formatter = F1r3SlashDeployFormatter::new(vec![0xbb; 33]);

    let envelopes = formatter.to_slash_evidence_envelopes(&evidence).unwrap();

    assert_eq!(envelopes.len(), 1);
    assert_eq!(envelopes[0].conflicting_block_bytes.len(), 2);

    let recovered = envelopes[0]
        .conflicting_block_bytes
        .iter()
        .map(|bytes| bincode::deserialize::<Block>(bytes).unwrap())
        .collect::<Vec<_>>();

    assert_eq!(recovered[0].identity.signature, left.identity.signature);
    assert_eq!(recovered[0].content.payload, left.content.payload);
    assert_eq!(recovered[1].identity.signature, right.identity.signature);
    assert_eq!(recovered[1].content.payload, right.content.payload);
}

#[test]
fn slash_formatter_rejects_non_conflicting_single_block_evidence() {
    let validator = node(1);
    let only = block(validator.clone(), 0x01, vec![0x55]);
    let evidence = vec![EquivocationEvidence::new(validator, 0, vec![only])];
    let formatter = F1r3SlashDeployFormatter::new(vec![0xcc; 33]);

    let err = formatter.to_slash_system_deploys(&evidence).unwrap_err();

    assert!(err.to_string().contains("at least two conflicting blocks"));
}
