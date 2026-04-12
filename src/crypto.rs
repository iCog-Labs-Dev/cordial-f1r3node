use crate::types::BlockContent;

/// compute a determinstic hash of the block content.
/// production implementation: seralize content e.g with bincode ot postcard, then SHA-256

pub fn hash_content(_content: &BlockContent) -> [u8; 32] {
    unimplemented!()
}

/// sign a content hash with node's private key.
/// production implementation: ed25519 signiture over 'hash'

pub fn sign(hash: &[u8; 32], _private_key: &[u8]) -> Vec<u8> {
    unimplemented!()
}