use std::collections::HashMap;

use casper::rust::validator_identity::ValidatorIdentity;
use cordial_f1r3node_adapter::grpc_ingest::{BlocklaceAdapter, GrpcBlockMapper};
use cordial_f1r3node_adapter::live_ingress::{
    AlreadyValidatedVerifier, LiveIngress, LiveIngressError, LiveIngressPhase, MirrorDisposition,
};
use cordial_f1r3node_adapter::repository::RSpaceBlocklaceRepository;
use cordial_f1r3node_adapter::shard_conf::CasperShardConf;
use cordial_miners_core::Block;
use cordial_miners_core::types::BlockIdentity;
use crypto::rust::private_key::PrivateKey;
use models::rust::casper::protocol::casper_message::{
    BlockMessage, Body, F1r3flyState, Header, Justification,
};
use tempfile::tempdir;

const MAP_SIZE: usize = 10 * 1024 * 1024;

fn recovered_ingress(repo: &RSpaceBlocklaceRepository) -> LiveIngress<RecordingAdapter> {
    LiveIngress::with_persistent_store(
        RecordingAdapter::default(),
        HashMap::new(),
        CasperShardConf::default(),
        "root",
        repo,
        &AlreadyValidatedVerifier,
    )
    .unwrap()
    .0
}

#[derive(Default)]
struct RecordingAdapter {
    accepted: Vec<BlockIdentity>,
    calls: usize,
    reject_next: bool,
}

impl BlocklaceAdapter<BlockIdentity> for RecordingAdapter {
    fn on_block(&mut self, block: Block) -> anyhow::Result<()> {
        self.calls += 1;
        if self.reject_next {
            self.reject_next = false;
            anyhow::bail!("adapter rejected native block");
        }
        self.accepted.push(block.identity);
        Ok(())
    }
}

fn signed_block(
    validator: &ValidatorIdentity,
    parent: Option<&BlockMessage>,
    block_number: i64,
) -> BlockMessage {
    let unsigned = BlockMessage {
        block_hash: vec![].into(),
        header: Header {
            parents_hash_list: parent
                .map(|block| vec![block.block_hash.clone()])
                .unwrap_or_default(),
            timestamp: block_number,
            version: 1,
            extra_bytes: vec![].into(),
        },
        body: Body {
            state: F1r3flyState {
                pre_state_hash: vec![].into(),
                post_state_hash: vec![block_number as u8; 32].into(),
                bonds: vec![],
                block_number,
            },
            deploys: vec![],
            rejected_deploys: vec![],
            system_deploys: vec![],
            extra_bytes: vec![].into(),
            applied_from_scope: vec![],
            merge_base: vec![].into(),
        },
        justifications: parent
            .map(|block| {
                vec![Justification {
                    validator: block.sender.clone(),
                    latest_block_hash: block.block_hash.clone(),
                }]
            })
            .unwrap_or_default(),
        sender: vec![].into(),
        seq_num: 0,
        sig: vec![].into(),
        sig_algorithm: String::new(),
        shard_id: "root".to_string(),
        extra_bytes: vec![].into(),
    };
    validator.sign_block(&unsigned)
}

#[test]
fn native_child_waits_for_parent_and_duplicates_do_not_repeat_callbacks() {
    let parent_validator = ValidatorIdentity::new(&PrivateKey::from_bytes(&[7u8; 32]));
    let child_validator = ValidatorIdentity::new(&PrivateKey::from_bytes(&[8u8; 32]));
    let parent = signed_block(&parent_validator, None, 0);
    let child = signed_block(&child_validator, Some(&parent), 1);
    let mut ingress = LiveIngress::new(RecordingAdapter::default());

    let error = ingress.ingest_native_block_message(&child).unwrap_err();
    assert!(matches!(error, LiveIngressError::Mapping(_)));
    assert_eq!(ingress.phase(), LiveIngressPhase::Defined);
    assert_eq!(ingress.adapter().calls, 0);
    assert!(ingress.blocklace().dom().is_empty());

    let parent_update = ingress.ingest_native_block_message(&parent).unwrap();
    assert_eq!(parent_update.disposition, MirrorDisposition::Applied);
    let child_update = ingress.ingest_native_block_message(&child).unwrap();
    assert_eq!(child_update.disposition, MirrorDisposition::Applied);
    assert_eq!(ingress.phase(), LiveIngressPhase::Connected);
    assert_eq!(ingress.adapter().calls, 2);
    assert_eq!(ingress.blocklace().dom().len(), 2);
    let parent_identity = &ingress.adapter().accepted[0];
    let child_identity = &ingress.adapter().accepted[1];
    assert!(
        ingress
            .blocklace()
            .content(child_identity)
            .unwrap()
            .predecessors
            .contains(parent_identity)
    );

    let duplicate = ingress.ingest_native_block_message(&child).unwrap();
    assert_eq!(duplicate.disposition, MirrorDisposition::Duplicate);
    assert_eq!(ingress.adapter().calls, 2);
}

#[test]
fn rejected_native_block_can_be_retried_without_publishing_a_mapping() {
    let validator = ValidatorIdentity::new(&PrivateKey::from_bytes(&[7u8; 32]));
    let parent = signed_block(&validator, None, 0);
    let mut ingress = LiveIngress::new(RecordingAdapter {
        reject_next: true,
        ..RecordingAdapter::default()
    });

    let error = ingress.ingest_native_block_message(&parent).unwrap_err();
    assert!(matches!(error, LiveIngressError::Adapter(_)));
    assert!(ingress.blocklace().dom().is_empty());
    assert_eq!(ingress.phase(), LiveIngressPhase::Defined);

    let retry = ingress.ingest_native_block_message(&parent).unwrap();
    assert_eq!(retry.disposition, MirrorDisposition::Applied);
    assert_eq!(ingress.adapter().calls, 2);
    assert_eq!(ingress.blocklace().dom().len(), 1);
}

#[test]
fn recovered_block_without_native_index_is_rejected() {
    let validator = ValidatorIdentity::new(&PrivateKey::from_bytes(&[7u8; 32]));
    let native = signed_block(&validator, None, 0);
    let mapper: GrpcBlockMapper<(), (), ()> = GrpcBlockMapper::new();
    let translated = mapper.from_protobuf(&native).unwrap();
    let mut ingress = LiveIngress::new(RecordingAdapter::default());
    ingress.ingest_trusted_block(translated.block).unwrap();
    let calls_before = ingress.adapter().calls;

    let error = ingress.ingest_native_block_message(&native).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("without its native wire-hash mapping")
    );
    assert_eq!(ingress.adapter().calls, calls_before);
    assert_eq!(ingress.blocklace().dom().len(), 1);
}

#[test]
fn native_identity_collision_is_rejected_before_adapter_callback() {
    let validator = ValidatorIdentity::new(&PrivateKey::from_bytes(&[7u8; 32]));
    let first = signed_block(&validator, None, 0);
    let mut second = first.clone();
    second.header.timestamp += 1;
    let second = validator.sign_block(&second);
    let mut ingress = LiveIngress::new(RecordingAdapter::default());

    ingress.ingest_native_block_message(&first).unwrap();
    let error = ingress.ingest_native_block_message(&second).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("collapse to one Cordial identity")
    );
    assert_eq!(ingress.adapter().calls, 1);
    assert_eq!(ingress.blocklace().dom().len(), 1);
}

#[test]
fn persistent_native_parent_mapping_survives_restart() {
    let dir = tempdir().unwrap();
    let validator = ValidatorIdentity::new(&PrivateKey::from_bytes(&[7u8; 32]));
    let parent = signed_block(&validator, None, 0);
    let child = signed_block(&validator, Some(&parent), 1);

    {
        let repo = RSpaceBlocklaceRepository::open(dir.path(), MAP_SIZE).unwrap();
        let mut ingress = recovered_ingress(&repo);
        assert_eq!(
            ingress
                .ingest_native_block_message_and_persist(&parent, &repo)
                .unwrap()
                .disposition,
            MirrorDisposition::Applied
        );
        assert_eq!(repo.native_block_mappings().unwrap().len(), 1);
    }

    let repo = RSpaceBlocklaceRepository::open(dir.path(), MAP_SIZE).unwrap();
    let mut ingress = recovered_ingress(&repo);
    assert_eq!(ingress.blocklace().dom().len(), 1);
    assert_eq!(
        ingress
            .ingest_native_block_message_and_persist(&parent, &repo)
            .unwrap()
            .disposition,
        MirrorDisposition::Duplicate
    );
    assert_eq!(ingress.adapter().calls, 0);
    assert_eq!(
        ingress
            .ingest_native_block_message_and_persist(&child, &repo)
            .unwrap()
            .disposition,
        MirrorDisposition::Applied
    );
    assert_eq!(ingress.blocklace().dom().len(), 2);
    assert_eq!(repo.native_block_mappings().unwrap().len(), 2);
}

#[test]
fn rejected_native_block_does_not_reach_persistent_store() {
    let dir = tempdir().unwrap();
    let repo = RSpaceBlocklaceRepository::open(dir.path(), MAP_SIZE).unwrap();
    let validator = ValidatorIdentity::new(&PrivateKey::from_bytes(&[7u8; 32]));
    let parent = signed_block(&validator, None, 0);
    let mut ingress = LiveIngress::new(RecordingAdapter {
        reject_next: true,
        ..RecordingAdapter::default()
    });

    assert!(matches!(
        ingress.ingest_native_block_message_and_persist(&parent, &repo),
        Err(LiveIngressError::Adapter(_))
    ));
    assert!(repo.native_block_mappings().unwrap().is_empty());
    assert!(recovered_ingress(&repo).blocklace().dom().is_empty());
}

#[test]
fn invalid_stored_native_identity_fails_closed_at_startup() {
    let dir = tempdir().unwrap();
    let repo = RSpaceBlocklaceRepository::open(dir.path(), MAP_SIZE).unwrap();
    let validator = ValidatorIdentity::new(&PrivateKey::from_bytes(&[7u8; 32]));
    let native = signed_block(&validator, None, 0);
    let mapper: GrpcBlockMapper<(), (), ()> = GrpcBlockMapper::new();
    let mut translated = mapper.from_protobuf(&native).unwrap().block;
    translated.identity.content_hash = [42u8; 32];
    let wire_hash: [u8; 32] = native.block_hash.as_ref().try_into().unwrap();
    repo.put_native_block(&translated, &wire_hash).unwrap();

    let result = LiveIngress::with_persistent_store(
        RecordingAdapter::default(),
        HashMap::new(),
        CasperShardConf::default(),
        "root",
        &repo,
        &AlreadyValidatedVerifier,
    );
    assert!(result.is_err());
    let error = result.err().unwrap();
    assert!(
        error
            .to_string()
            .contains("invalid translated Cordial identity"),
        "unexpected recovery error: {error}"
    );
}

#[test]
fn persistent_identity_collision_does_not_write_partial_mapping() {
    let dir = tempdir().unwrap();
    let repo = RSpaceBlocklaceRepository::open(dir.path(), MAP_SIZE).unwrap();
    let validator = ValidatorIdentity::new(&PrivateKey::from_bytes(&[7u8; 32]));
    let first = signed_block(&validator, None, 0);
    let mut second = first.clone();
    second.header.timestamp += 1;
    let second = validator.sign_block(&second);
    let mapper: GrpcBlockMapper<(), (), ()> = GrpcBlockMapper::new();
    let first = mapper.from_protobuf(&first).unwrap();
    let second = mapper.from_protobuf(&second).unwrap();
    assert_eq!(first.block.identity, second.block.identity);

    repo.put_native_block(&first.block, &first.wire_hash)
        .unwrap();
    let error = repo
        .put_native_block(&second.block, &second.wire_hash)
        .unwrap_err();
    assert!(error.to_string().contains("distinct native wire hashes"));
    assert_eq!(repo.native_block_mappings().unwrap().len(), 1);
}
