use std::path::{Path, PathBuf};

use block_storage::rust::key_value_block_store::KeyValueBlockStore;
use casper::rust::storage::rnode_key_value_store_manager::new_key_value_store_manager;
use models::rust::casper::protocol::casper_message::BlockMessage;
use prost::bytes::Bytes;

#[derive(Debug, thiserror::Error)]
pub enum NativeBlockSourceError {
    #[error("host block storage is absent at {0}")]
    MissingStore(PathBuf),
    #[error("failed to open host block storage: {0}")]
    Open(String),
    #[error("failed to read host block {hash}: {reason}")]
    Read { hash: String, reason: String },
    #[error("host block {0} is missing from local storage")]
    MissingBlock(String),
    #[error("host block storage returned a different declared hash for {0}")]
    HashMismatch(String),
    #[error("host block read task failed: {0}")]
    Task(String),
}

#[derive(Clone)]
pub struct LocalNativeBlockSource {
    store: KeyValueBlockStore,
}

impl LocalNativeBlockSource {
    pub async fn open(data_dir: &Path) -> Result<Self, NativeBlockSourceError> {
        let blockstorage = data_dir.join("blockstorage").join("data.mdb");
        if !blockstorage.is_file() {
            return Err(NativeBlockSourceError::MissingStore(blockstorage));
        }
        let dagstorage = data_dir.join("dagstorage").join("data.mdb");
        if !dagstorage.is_file() {
            return Err(NativeBlockSourceError::MissingStore(dagstorage));
        }
        let mut manager = new_key_value_store_manager(data_dir.to_path_buf(), None);
        let store = KeyValueBlockStore::create_from_kvm(&mut manager)
            .await
            .map_err(|error| NativeBlockSourceError::Open(error.to_string()))?;
        Ok(Self { store })
    }

    pub async fn read(&self, hash: [u8; 32]) -> Result<BlockMessage, NativeBlockSourceError> {
        let store = self.store.clone();
        tokio::task::spawn_blocking(move || {
            let key = Bytes::copy_from_slice(&hash);
            let hash_hex = hex::encode(hash);
            let block = store
                .get(&key)
                .map_err(|error| NativeBlockSourceError::Read {
                    hash: hash_hex.clone(),
                    reason: error.to_string(),
                })?
                .ok_or_else(|| NativeBlockSourceError::MissingBlock(hash_hex.clone()))?;
            if block.block_hash.as_ref() != hash {
                return Err(NativeBlockSourceError::HashMismatch(hash_hex));
            }
            Ok(block)
        })
        .await
        .map_err(|error| NativeBlockSourceError::Task(error.to_string()))?
    }
}
