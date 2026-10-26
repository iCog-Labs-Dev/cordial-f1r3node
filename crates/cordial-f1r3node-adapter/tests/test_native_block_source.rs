use block_storage::rust::key_value_block_store::KeyValueBlockStore;
use casper::rust::storage::rnode_key_value_store_manager::new_key_value_store_manager;
use casper::rust::validator_identity::ValidatorIdentity;
use cordial_f1r3node_adapter::grpc_ingest::GrpcBlockMapper;
use cordial_f1r3node_adapter::native_block_source::{
    LocalNativeBlockSource, NativeBlockSourceError,
};
use crypto::rust::private_key::PrivateKey;
use models::rust::casper::protocol::casper_message::{BlockMessage, Body, F1r3flyState, Header};
use prost::bytes::Bytes;
use tempfile::tempdir;

fn signed_root() -> BlockMessage {
    let validator = ValidatorIdentity::new(&PrivateKey::from_bytes(&[7u8; 32]));
    validator.sign_block(&BlockMessage {
        block_hash: Bytes::new(),
        header: Header {
            parents_hash_list: Vec::new(),
            timestamp: 1,
            version: 1,
            extra_bytes: Bytes::new(),
        },
        body: Body {
            state: F1r3flyState {
                pre_state_hash: Bytes::new(),
                post_state_hash: Bytes::from(vec![1u8; 32]),
                bonds: Vec::new(),
                block_number: 0,
            },
            deploys: Vec::new(),
            rejected_deploys: Vec::new(),
            system_deploys: Vec::new(),
            extra_bytes: Bytes::new(),
            applied_from_scope: Vec::new(),
            merge_base: Bytes::new(),
        },
        justifications: Vec::new(),
        sender: Bytes::new(),
        seq_num: 0,
        sig: Bytes::new(),
        sig_algorithm: String::new(),
        shard_id: "root".into(),
        extra_bytes: Bytes::new(),
    })
}

#[tokio::test]
async fn reads_full_signed_block_from_host_store() {
    let dir = tempdir().unwrap();
    let mut manager = new_key_value_store_manager(dir.path().to_path_buf(), None);
    let store = KeyValueBlockStore::create_from_kvm(&mut manager)
        .await
        .unwrap();
    let signed = signed_root();
    store.put_block_message(&signed).unwrap();
    let hash: [u8; 32] = signed.block_hash.as_ref().try_into().unwrap();

    let source = LocalNativeBlockSource::open(dir.path()).await.unwrap();
    let loaded = source.read(hash).await.unwrap();
    assert_eq!(loaded, signed);
    let mapper: GrpcBlockMapper<(), (), ()> = GrpcBlockMapper::new();
    mapper.from_protobuf(&loaded).unwrap();
}

#[tokio::test]
async fn missing_or_mismatched_store_key_fails_closed() {
    let dir = tempdir().unwrap();
    assert!(matches!(
        LocalNativeBlockSource::open(dir.path()).await,
        Err(NativeBlockSourceError::MissingStore(_))
    ));
    assert!(!dir.path().join("blockstorage").exists());

    let mut manager = new_key_value_store_manager(dir.path().to_path_buf(), None);
    let store = KeyValueBlockStore::create_from_kvm(&mut manager)
        .await
        .unwrap();
    let signed = signed_root();
    let wrong_hash = [9u8; 32];
    store
        .put(Bytes::copy_from_slice(&wrong_hash), &signed)
        .unwrap();
    let source = LocalNativeBlockSource::open(dir.path()).await.unwrap();
    assert!(matches!(
        source.read([8u8; 32]).await,
        Err(NativeBlockSourceError::MissingBlock(_))
    ));
    assert!(matches!(
        source.read(wrong_hash).await,
        Err(NativeBlockSourceError::HashMismatch(_))
    ));
}
