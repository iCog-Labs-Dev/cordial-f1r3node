use std::collections::HashMap;

use block_storage::rust::key_value_block_store::KeyValueBlockStore;
use casper::rust::storage::rnode_key_value_store_manager::new_key_value_store_manager;
use casper::rust::util::proto_util;
use casper::rust::validator_identity::ValidatorIdentity;
use cordial_f1r3node_adapter::grpc_ingest::BlocklaceAdapter;
use cordial_f1r3node_adapter::live_ingress::{AlreadyValidatedVerifier, LiveIngress};
use cordial_f1r3node_adapter::native_block_source::LocalNativeBlockSource;
use cordial_f1r3node_adapter::native_live_runner::{
    NativeBackfillError, apply_native_height_window, ingest_with_native_backfill,
    replay_native_height_window,
};
use cordial_f1r3node_adapter::repository::RSpaceBlocklaceRepository;
use cordial_f1r3node_adapter::shard_conf::CasperShardConf;
use cordial_miners_core::Block;
use cordial_miners_core::types::BlockIdentity;
use crypto::rust::private_key::PrivateKey;
use models::rust::casper::protocol::casper_message::{
    BlockMessage, Body, F1r3flyState, Header, Justification,
};
use prost::bytes::Bytes;
use tempfile::tempdir;

struct PassthroughAdapter;

impl BlocklaceAdapter<BlockIdentity> for PassthroughAdapter {
    fn on_block(&mut self, _block: Block) -> anyhow::Result<()> {
        Ok(())
    }
}

fn signed_block(
    validator: &ValidatorIdentity,
    parent: Option<&BlockMessage>,
    height: i64,
) -> BlockMessage {
    let mut block = validator.sign_block(&BlockMessage {
        block_hash: Bytes::new(),
        header: Header {
            parents_hash_list: parent
                .map(|block| vec![block.block_hash.clone()])
                .unwrap_or_default(),
            timestamp: height,
            version: 1,
            extra_bytes: Bytes::new(),
        },
        body: Body {
            state: F1r3flyState {
                pre_state_hash: Bytes::new(),
                post_state_hash: Bytes::from(vec![height as u8; 32]),
                bonds: Vec::new(),
                block_number: height,
            },
            deploys: Vec::new(),
            rejected_deploys: Vec::new(),
            system_deploys: Vec::new(),
            extra_bytes: Bytes::new(),
            applied_from_scope: Vec::new(),
            merge_base: Bytes::new(),
        },
        justifications: parent
            .map(|block| {
                vec![Justification {
                    validator: block.sender.clone(),
                    latest_block_hash: block.block_hash.clone(),
                }]
            })
            .unwrap_or_default(),
        sender: Bytes::new(),
        seq_num: 0,
        sig: Bytes::new(),
        sig_algorithm: String::new(),
        shard_id: "root".into(),
        extra_bytes: Bytes::new(),
    });
    if parent.is_some_and(|parent| parent.sender.is_empty()) {
        block.justifications[0].validator = block.sender.clone();
        block = validator.sign_block(&block);
    }
    block
}

fn recover(repo: &RSpaceBlocklaceRepository) -> LiveIngress<PassthroughAdapter> {
    LiveIngress::with_persistent_store(
        PassthroughAdapter,
        HashMap::new(),
        CasperShardConf::default(),
        "root",
        repo,
        &AlreadyValidatedVerifier,
    )
    .unwrap()
    .0
}

#[tokio::test]
async fn child_first_backfill_and_cursor_survive_restart() {
    let host_dir = tempdir().unwrap();
    let mirror_dir = tempdir().unwrap();
    let mut manager = new_key_value_store_manager(host_dir.path().to_path_buf(), None);
    let host_store = KeyValueBlockStore::create_from_kvm(&mut manager)
        .await
        .unwrap();
    let validator = ValidatorIdentity::new(&PrivateKey::from_bytes(&[7u8; 32]));
    let parent_template = signed_block(&validator, None, 0);
    let parent = proto_util::unsigned_block_proto(
        parent_template.body,
        parent_template.header,
        Vec::new(),
        "root".into(),
        None,
    );
    assert!(parent.sender.is_empty() && parent.sig.is_empty());
    let child = signed_block(&validator, Some(&parent), 1);
    host_store.put_block_message(&parent).unwrap();
    host_store.put_block_message(&child).unwrap();
    let source = LocalNativeBlockSource::open(host_dir.path()).await.unwrap();

    {
        let repo = RSpaceBlocklaceRepository::open(mirror_dir.path(), 10 * 1024 * 1024).unwrap();
        let mut ingress = recover(&repo);
        assert!(
            ingress
                .ingest_native_block_message_and_persist(&parent, &repo)
                .is_err()
        );
        assert!(repo.native_block_mappings().unwrap().is_empty());
        assert!(matches!(
            ingest_with_native_backfill(&mut ingress, &repo, &source, child.clone(), 0).await,
            Err(NativeBackfillError::BackfillLimit(0))
        ));
        assert!(repo.native_block_mappings().unwrap().is_empty());
        assert_eq!(
            ingest_with_native_backfill(&mut ingress, &repo, &source, child.clone(), 8)
                .await
                .unwrap(),
            2
        );
        assert_eq!(ingress.blocklace().dom().len(), 2);
        assert_eq!(repo.native_height_cursor().unwrap(), None);
        assert_eq!(
            apply_native_height_window(&mut ingress, &repo, &source, vec![parent.clone()], 0, 0, 8)
                .await
                .unwrap(),
            0
        );
        assert_eq!(repo.native_height_cursor().unwrap(), Some(0));
    }

    let repo = RSpaceBlocklaceRepository::open(mirror_dir.path(), 10 * 1024 * 1024).unwrap();
    let mut ingress = recover(&repo);
    assert_eq!(ingress.blocklace().dom().len(), 2);
    assert_eq!(
        apply_native_height_window(&mut ingress, &repo, &source, vec![child.clone()], 1, 1, 8)
            .await
            .unwrap(),
        0
    );
    assert_eq!(repo.native_height_cursor().unwrap(), Some(1));
    assert!(repo.put_native_height_cursor(0).is_err());
    assert_eq!(
        replay_native_height_window(&mut ingress, &repo, &source, vec![child], 1, 1, 8)
            .await
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn incomplete_height_window_does_not_advance_cursor() {
    let host_dir = tempdir().unwrap();
    let mirror_dir = tempdir().unwrap();
    let mut manager = new_key_value_store_manager(host_dir.path().to_path_buf(), None);
    KeyValueBlockStore::create_from_kvm(&mut manager)
        .await
        .unwrap();
    let source = LocalNativeBlockSource::open(host_dir.path()).await.unwrap();
    let repo = RSpaceBlocklaceRepository::open(mirror_dir.path(), 10 * 1024 * 1024).unwrap();
    let mut ingress = recover(&repo);

    assert!(matches!(
        apply_native_height_window(&mut ingress, &repo, &source, vec![], 0, 0, 8).await,
        Err(NativeBackfillError::MissingHeight(0))
    ));
    assert_eq!(repo.native_height_cursor().unwrap(), None);
    assert!(matches!(
        apply_native_height_window(&mut ingress, &repo, &source, vec![], 1, 1, 8).await,
        Err(NativeBackfillError::CursorStart { .. })
    ));
}
