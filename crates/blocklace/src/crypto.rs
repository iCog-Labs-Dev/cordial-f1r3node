use sha2::{Sha256, Digest};
use ed25519_dalek::{SigningKey, Signer, Verifier, VerifyingKey, Signature};
use crate::types::BlockContent;

/// Compute a deterministic SHA-256 hash of the block content.
/// Serialization format: [payload_len (8 bytes) | payload | num_preds (8 bytes) | sorted predecessors]
/// Each predecessor is serialized as: [content_hash (32 bytes) | creator_len (8 bytes) | creator | sig_len (8 bytes) | signature]
/// Predecessors are sorted by content_hash to ensure deterministic ordering.
pub fn hash_content(content: &BlockContent) -> [u8; 32] {
    let mut hasher = Sha256::new();

    // Hash the payload with its length prefix
    hasher.update((content.payload.len() as u64).to_le_bytes());
    hasher.update(&content.payload);

    // Sort predecessors by content_hash for deterministic ordering
    let mut preds: Vec<_> = content.predecessors.iter().collect();
    preds.sort_by_key(|p| p.content_hash);

    hasher.update((preds.len() as u64).to_le_bytes());
    for pred in &preds {
        hasher.update(pred.content_hash);
        hasher.update((pred.creator.0.len() as u64).to_le_bytes());
        hasher.update(&pred.creator.0);
        hasher.update((pred.signature.len() as u64).to_le_bytes());
        hasher.update(&pred.signature);
    }

    hasher.finalize().into()
}

/// Sign a content hash with node's ED25519 private key.
/// The private_key must be exactly 32 bytes (an ED25519 secret key).
pub fn sign(hash: &[u8; 32], private_key: &[u8]) -> Vec<u8> {
    let signing_key = SigningKey::from_bytes(
        private_key.try_into().expect("private key must be 32 bytes")
    );
    let signature = signing_key.sign(hash);
    signature.to_bytes().to_vec()
}

/// Verify an ED25519 signature over a content hash.
/// The public_key must be exactly 32 bytes (an ED25519 verifying key).
pub fn verify(hash: &[u8; 32], public_key: &[u8], signature: &[u8]) -> bool {
    let Ok(verifying_key) = VerifyingKey::from_bytes(
        public_key.try_into().expect("public key must be 32 bytes")
    ) else {
        return false;
    };
    let sig = Signature::from_bytes(
        signature.try_into().expect("signature must be 64 bytes")
    );
    verifying_key.verify(hash, &sig).is_ok()
}
