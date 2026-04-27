use blake2::{Blake2b, Digest as Blake2Digest, digest::consts::U32};
use ed25519_dalek::{
    Signature as EdSignature, Signer as _, SigningKey as EdSigningKey, Verifier as _,
    VerifyingKey as EdVerifyingKey,
};
use k256::ecdsa::{
    Signature as SecpSignature, SigningKey as SecpSigningKey, VerifyingKey as SecpVerifyingKey,
    signature::hazmat::{PrehashSigner, PrehashVerifier},
};
use sha2::{Sha256};

use crate::types::BlockContent;

// 1. Traits and Enums (Algorithm Abstractions)

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HashAlgorithm {
    Sha256,
    Blake2b256,
}

pub trait Hasher {
    fn name(&self) -> String;
    fn hash(&self, data: &[u8]) -> [u8; 32];
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SigAlgorithm {
    Ed25519,
    Secp256k1,
}

pub trait SignatureScheme {
    fn name(&self) -> String;
    fn sign(&self, hash: &[u8; 32], private_key: &[u8]) -> Result<Vec<u8>, String>;
    fn verify(&self, hash: &[u8; 32], public_key: &[u8], signature: &[u8]) -> bool;
}

// 2. Hasher Implementations

/// Blake2b-256: default for f1r3node alignment.
pub struct Blake2b256Hasher;
impl Hasher for Blake2b256Hasher {
    fn name(&self) -> String {
        "blake2b256".to_string()
    }
    fn hash(&self, data: &[u8]) -> [u8; 32] {
        let mut hasher = Blake2b::<U32>::new();
        hasher.update(data);
        hasher.finalize().into()
    }
}

pub struct Sha256Hasher;
impl Hasher for Sha256Hasher {
    fn name(&self) -> String {
        "sha256".to_string()
    }
    fn hash(&self, data: &[u8]) -> [u8; 32] {
        let mut hasher = Sha256::new();
        hasher.update(data);
        hasher.finalize().into()
    }
}

// 3. Signature Scheme Implementations

/// Secp256k1: default for f1r3node validator alignment.
pub struct Secp256k1Scheme;
impl SignatureScheme for Secp256k1Scheme {
    fn name(&self) -> String {
        "secp256k1".to_string()
    }
    fn sign(&self, hash: &[u8; 32], private_key: &[u8]) -> Result<Vec<u8>, String> {
        let signing_key =
            SecpSigningKey::from_slice(private_key).map_err(|_| "Invalid Secp256k1 privkey")?;
        // Prehash signing: signs the 32-byte hash directly.
        let signature: SecpSignature = signing_key
            .sign_prehash(hash)
            .map_err(|_| "Signing failed")?;
        // Returns DER encoding to match f1r3node wire format.
        Ok(signature.to_der().as_bytes().to_vec())
    }
    fn verify(&self, hash: &[u8; 32], public_key: &[u8], signature: &[u8]) -> bool {
        let Ok(vk) = SecpVerifyingKey::from_sec1_bytes(public_key) else {
            return false;
        };
        let Ok(sig) = SecpSignature::from_der(signature) else {
            return false;
        };
        vk.verify_prehash(hash, &sig).is_ok()
    }
}

pub struct Ed25519Scheme;
impl SignatureScheme for Ed25519Scheme {
    fn name(&self) -> String {
        "ed25519".to_string()
    }
    fn sign(&self, hash: &[u8; 32], private_key: &[u8]) -> Result<Vec<u8>, String> {
        let key_bytes: [u8; 32] = private_key
            .try_into()
            .map_err(|_| "ED25519 privkey != 32 bytes")?;
        let signing_key = EdSigningKey::from_bytes(&key_bytes);
        Ok(signing_key.sign(hash).to_bytes().to_vec())
    }
    fn verify(&self, hash: &[u8; 32], public_key: &[u8], signature: &[u8]) -> bool {
        let Ok(pk_bytes) = public_key.try_into() else {
            return false;
        };
        let Ok(vk) = EdVerifyingKey::from_bytes(pk_bytes) else {
            return false;
        };
        let Ok(sig) = EdSignature::from_slice(signature) else {
            return false;
        };
        vk.verify(hash, &sig).is_ok()
    }
}

// 4. Content Hashing Logic (Logical Parity)

/// Generic content hashing.
/// In f1r3node, the creator is usually part of the block message structure
/// that gets hashed. We ensure the logical sequence is preserved.
pub fn hash_content_ext(content: &BlockContent, hasher: &dyn Hasher) -> [u8; 32] {
    let mut buf = Vec::new();

    // 1. Payload
    buf.extend_from_slice(&(content.payload.len() as u64).to_le_bytes());
    buf.extend_from_slice(&content.payload);

    // 2. Predecessors (Sorted for determinism)
    let mut preds: Vec<_> = content.predecessors.iter().collect();
    preds.sort_by_key(|p| p.content_hash);

    buf.extend_from_slice(&(preds.len() as u64).to_le_bytes());
    for pred in &preds {
        buf.extend_from_slice(&pred.content_hash);
        buf.extend_from_slice(&(pred.creator.0.len() as u64).to_le_bytes());
        buf.extend_from_slice(&pred.creator.0);
        buf.extend_from_slice(&(pred.signature.len() as u64).to_le_bytes());
        buf.extend_from_slice(&pred.signature);
    }

    hasher.hash(&buf)
}

// 5. Default Implementations (ALIGNED WITH FIRE NODE)

/// Default hash uses Blake2b-256 for f1r3node alignment.
pub fn hash_content(content: &BlockContent) -> [u8; 32] {
    hash_content_ext(content, &Blake2b256Hasher)
}

/// Default sign uses Secp256k1 for f1r3node alignment.
pub fn sign(hash: &[u8; 32], private_key: &[u8]) -> Vec<u8> {
    Secp256k1Scheme
        .sign(hash, private_key)
        .expect("Default Secp256k1 sign failed")
}

/// Default verify uses Secp256k1 for f1r3node alignment.
pub fn verify(hash: &[u8; 32], public_key: &[u8], signature: &[u8]) -> bool {
    Secp256k1Scheme.verify(hash, public_key, signature)
}
