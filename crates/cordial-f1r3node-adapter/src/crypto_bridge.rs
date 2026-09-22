//! Cryptographic primitives and adapter-local snapshot hashing.
//!
//! [`Hasher`], [`Signer`], and [`Verifier`] provide Blake2b-256/SHA-256
//! hashing and Secp256k1/Ed25519 signatures. Sharing these primitives with
//! f1r3node does not make different block encodings hash-compatible.
//!
//! There are three separate hash domains:
//!
//! - [`compute_adapter_snapshot_hash`] hashes a deterministic, selected-field
//!   layout of the adapter-owned [`BlockMessage`]. It includes the sender to
//!   distinguish otherwise identical snapshots from different validators.
//! - Cordial's [`hash_content`] hashes the internal [`BlockContent`]; local
//!   adapter messages carry signatures over this internal content hash.
//! - f1r3node's `casper::rust::util::proto_util::hash_block` hashes the original
//!   protobuf representation. [`crate::grpc_ingest::GrpcBlockMapper::from_protobuf`]
//!   uses that function and verifies the wire signature before lossy translation.
//!
//! The adapter snapshot hash is neither the f1r3node wire hash nor a digest of
//! every message field. Never use it to validate network protobuf blocks.

use blake2::Blake2b;
use blake2::digest::consts::U32;
use k256::ecdsa::signature::hazmat::{PrehashSigner, PrehashVerifier};
use k256::ecdsa::{
    Signature as K256Signature, SigningKey as K256SigningKey, VerifyingKey as K256VerifyingKey,
};
use sha2::{Digest, Sha256};

use crate::block_translation::BlockMessage;

use cordial_miners_core::crypto::{CryptoVerifier, hash_content};
use cordial_miners_core::types::{BlockContent, NodeId}; // The data types we need to work with

// ═══════════════════════════════════════════════════════════════════════════
// Hashing
// ═══════════════════════════════════════════════════════════════════════════

/// Trait for 32-byte content hashers.
///
/// Implemented for [`Sha256Hasher`] and [`Blake2b256Hasher`].
pub trait Hasher {
    /// Name of the algorithm, e.g. `"sha256"` or `"blake2b256"`.
    fn name(&self) -> &'static str;

    /// Hash `input` into a 32-byte digest.
    fn hash(&self, input: &[u8]) -> [u8; 32];
}

/// SHA-256 hasher for legacy content hashing.
#[derive(Debug, Clone, Copy, Default)]
pub struct Sha256Hasher;

impl Hasher for Sha256Hasher {
    fn name(&self) -> &'static str {
        "sha256"
    }
    fn hash(&self, input: &[u8]) -> [u8; 32] {
        let mut h = Sha256::new();
        h.update(input);
        h.finalize().into()
    }
}

/// Blake2b with a 32-byte output. Matches f1r3node's `Blake2b256::hash()`.
#[derive(Debug, Clone, Copy, Default)]
pub struct Blake2b256Hasher;

impl Hasher for Blake2b256Hasher {
    fn name(&self) -> &'static str {
        "blake2b256"
    }
    fn hash(&self, input: &[u8]) -> [u8; 32] {
        // Blake2b with a fixed 32-byte output length is the same primitive
        // f1r3node calls "Blake2b256".
        let mut h = Blake2b::<U32>::new();
        h.update(input);
        let out: [u8; 32] = h.finalize().into();
        out
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Signing / verification
// ═══════════════════════════════════════════════════════════════════════════

/// Signature algorithm identifiers as used on the f1r3node wire.
///
/// These match the strings f1r3node puts in `BlockMessage.sig_algorithm`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SigAlgorithm {
    Ed25519,
    Secp256k1,
}

impl SigAlgorithm {
    pub fn as_str(&self) -> &'static str {
        match self {
            SigAlgorithm::Ed25519 => "ed25519",
            SigAlgorithm::Secp256k1 => "secp256k1",
        }
    }
}

/// Errors from signing / verifying.
#[derive(Debug, Clone, PartialEq)]
pub enum CryptoError {
    /// Private key has the wrong length for the chosen algorithm.
    InvalidPrivateKeyLength { expected: usize, actual: usize },
    /// Public key has the wrong length for the chosen algorithm.
    InvalidPublicKeyLength { expected: usize, actual: usize },
    /// Public key bytes don't decode as a valid point on the curve.
    InvalidPublicKey,
    /// Signature has the wrong length.
    InvalidSignatureLength { expected: usize, actual: usize },
    /// Signature bytes don't decode as a valid signature.
    InvalidSignature,
}

/// Trait for signing a 32-byte hash into a signature.
pub trait Signer {
    fn algorithm(&self) -> SigAlgorithm;
    fn sign(&self, hash: &[u8; 32], private_key: &[u8]) -> Result<Vec<u8>, CryptoError>;
}

/// Trait for verifying a signature over a 32-byte hash.
pub trait Verifier {
    fn algorithm(&self) -> SigAlgorithm;
    fn verify(
        &self,
        hash: &[u8; 32],
        public_key: &[u8],
        signature: &[u8],
    ) -> Result<bool, CryptoError>;
}

// ── ED25519 ──────────────────────────────────────────────────────────────

/// ED25519 signer/verifier. Same primitive the blocklace core crate uses.
#[derive(Debug, Clone, Copy, Default)]
pub struct Ed25519;

impl Signer for Ed25519 {
    fn algorithm(&self) -> SigAlgorithm {
        SigAlgorithm::Ed25519
    }
    fn sign(&self, hash: &[u8; 32], private_key: &[u8]) -> Result<Vec<u8>, CryptoError> {
        use ed25519_dalek::{Signer as _, SigningKey};
        let pk_bytes: &[u8; 32] =
            private_key
                .try_into()
                .map_err(|_| CryptoError::InvalidPrivateKeyLength {
                    expected: 32,
                    actual: private_key.len(),
                })?;
        let signing_key = SigningKey::from_bytes(pk_bytes);
        Ok(signing_key.sign(hash).to_bytes().to_vec())
    }
}

impl Verifier for Ed25519 {
    fn algorithm(&self) -> SigAlgorithm {
        SigAlgorithm::Ed25519
    }
    fn verify(
        &self,
        hash: &[u8; 32],
        public_key: &[u8],
        signature: &[u8],
    ) -> Result<bool, CryptoError> {
        use ed25519_dalek::{Signature, Verifier as _, VerifyingKey};
        let pk_bytes: &[u8; 32] =
            public_key
                .try_into()
                .map_err(|_| CryptoError::InvalidPublicKeyLength {
                    expected: 32,
                    actual: public_key.len(),
                })?;
        let sig_bytes: &[u8; 64] =
            signature
                .try_into()
                .map_err(|_| CryptoError::InvalidSignatureLength {
                    expected: 64,
                    actual: signature.len(),
                })?;
        let verifying_key =
            VerifyingKey::from_bytes(pk_bytes).map_err(|_| CryptoError::InvalidPublicKey)?;
        let sig = Signature::from_bytes(sig_bytes);
        Ok(verifying_key.verify(hash, &sig).is_ok())
    }
}

// ── Secp256k1 (ECDSA) ────────────────────────────────────────────────────

/// Secp256k1 ECDSA signer/verifier. The primary algorithm f1r3node uses
/// for validator identities.
///
/// Private key: 32 bytes. Public key: 33 bytes compressed or 65 bytes uncompressed SEC1. Signature: DER-encoded ECDSA, 70–72 bytes.
#[derive(Debug, Clone, Copy, Default)]
pub struct Secp256k1;

impl Signer for Secp256k1 {
    fn algorithm(&self) -> SigAlgorithm {
        SigAlgorithm::Secp256k1
    }
    fn sign(&self, hash: &[u8; 32], private_key: &[u8]) -> Result<Vec<u8>, CryptoError> {
        if private_key.len() != 32 {
            return Err(CryptoError::InvalidPrivateKeyLength {
                expected: 32,
                actual: private_key.len(),
            });
        }
        let signing_key = K256SigningKey::from_slice(private_key).map_err(|_| {
            CryptoError::InvalidPrivateKeyLength {
                expected: 32,
                actual: private_key.len(),
            }
        })?;
        // Sign the already-computed block hash directly. This matches the
        // core Secp256k1Scheme and f1r3node-style block verification.
        let sig: K256Signature = signing_key
            .sign_prehash(hash)
            .map_err(|_| CryptoError::InvalidSignature)?;
        Ok(sig.to_der().to_bytes().to_vec())
    }
}

impl Verifier for Secp256k1 {
    fn algorithm(&self) -> SigAlgorithm {
        SigAlgorithm::Secp256k1
    }
    fn verify(
        &self,
        hash: &[u8; 32],
        public_key: &[u8],
        signature: &[u8],
    ) -> Result<bool, CryptoError> {
        if public_key.len() != 33 && public_key.len() != 65 {
            return Err(CryptoError::InvalidPublicKeyLength {
                expected: 33,
                actual: public_key.len(),
            });
        }
        let verifying_key = K256VerifyingKey::from_sec1_bytes(public_key)
            .map_err(|_| CryptoError::InvalidPublicKey)?;
        let sig = K256Signature::from_der(signature).map_err(|_| CryptoError::InvalidSignature)?;
        Ok(verifying_key.verify_prehash(hash, &sig).is_ok())
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Adapter-local snapshot hash
// ═══════════════════════════════════════════════════════════════════════════

/// Compute a deterministic adapter-local snapshot hash using Blake2b-256.
///
/// The sender participates in the hash so otherwise identical snapshots from
/// different validators have distinct digests. This selected-field encoding
/// is not f1r3node's protobuf encoding and must not be used for wire validation.
///
/// The hash excludes `block_hash`, `sig`, `justifications`,
/// `body.rejected_deploys`, `body.extra_bytes`, and deploy fields other than
/// those listed below. It is not a commitment to the entire message.
///
/// ## Layout (deterministic)
///
/// All fields are length-prefixed with an 8-byte little-endian length
/// where they are variable-size, and concatenated in this order:
///
/// 1. `header.parents_hash_list` — count (u64 LE), then each hash's len + bytes
/// 2. `header.timestamp` (i64 LE)
/// 3. `header.version` (i64 LE)
/// 4. `header.extra_bytes` (len + bytes)
/// 5. `body.state.pre_state_hash` (len + bytes)
/// 6. `body.state.post_state_hash` (len + bytes)
/// 7. `body.state.block_number` (i64 LE)
/// 8. `body.state.bonds` — sorted by validator bytes, count (u64 LE) then
///    each `(validator_len, validator_bytes, stake_i64_le)`
/// 9. `body.deploys` — count (u64 LE), then each deploy's
///    `(signature_len, signature_bytes, cost_i64_le, is_failed_u8)`
/// 10. `body.system_deploys` — count (u64 LE), then tagged encoding of each
/// 11. `sender` (len + bytes) — **critical for uniqueness across validators**
/// 12. `sig_algorithm` (len + bytes)
/// 13. `seq_num` (i32 LE)
/// 14. `shard_id` (len + bytes)
/// 15. `extra_bytes` (len + bytes)
///
/// Hash = `Blake2b256(layout_bytes)`.
///
/// Use [`crate::grpc_ingest::GrpcBlockMapper::from_protobuf`] for f1r3node
/// wire hash and signature validation.
pub fn compute_adapter_snapshot_hash(msg: &BlockMessage) -> [u8; 32] {
    let mut buf: Vec<u8> = Vec::new();

    // Header
    put_u64_len(&mut buf, msg.header.parents_hash_list.len() as u64);
    for parent in &msg.header.parents_hash_list {
        put_bytes(&mut buf, parent);
    }
    buf.extend_from_slice(&msg.header.timestamp.to_le_bytes());
    buf.extend_from_slice(&msg.header.version.to_le_bytes());
    put_bytes(&mut buf, &msg.header.extra_bytes);

    // Body.state
    put_bytes(&mut buf, &msg.body.state.pre_state_hash);
    put_bytes(&mut buf, &msg.body.state.post_state_hash);
    buf.extend_from_slice(&msg.body.state.block_number.to_le_bytes());

    // Body.bonds — sorted for determinism
    let mut sorted_bonds = msg.body.state.bonds.clone();
    sorted_bonds.sort_by(|a, b| a.validator.cmp(&b.validator));
    put_u64_len(&mut buf, sorted_bonds.len() as u64);
    for b in &sorted_bonds {
        put_bytes(&mut buf, &b.validator);
        buf.extend_from_slice(&b.stake.to_le_bytes());
    }

    // Body.deploys
    put_u64_len(&mut buf, msg.body.deploys.len() as u64);
    for pd in &msg.body.deploys {
        put_bytes(&mut buf, &pd.deploy.sig);
        buf.extend_from_slice(&pd.cost.to_le_bytes());
        buf.push(pd.is_failed as u8);
    }

    // Body.system_deploys — tagged encoding
    put_u64_len(&mut buf, msg.body.system_deploys.len() as u64);
    for sd in &msg.body.system_deploys {
        use crate::block_translation::ProcessedSystemDeploy;
        match sd {
            ProcessedSystemDeploy::Slash {
                validator,
                succeeded,
            } => {
                buf.push(0u8);
                put_bytes(&mut buf, validator);
                buf.push(*succeeded as u8);
            }
            ProcessedSystemDeploy::CloseBlock { succeeded } => {
                buf.push(1u8);
                buf.push(*succeeded as u8);
            }
        }
    }

    // Sender — **this is what prevents same-content cross-validator collisions**.
    put_bytes(&mut buf, &msg.sender);
    put_bytes(&mut buf, msg.sig_algorithm.as_bytes());
    buf.extend_from_slice(&msg.seq_num.to_le_bytes());
    put_bytes(&mut buf, msg.shard_id.as_bytes());
    put_bytes(&mut buf, &msg.extra_bytes);

    Blake2b256Hasher.hash(&buf)
}

// Tiny helpers for the canonical layout.
fn put_u64_len(buf: &mut Vec<u8>, n: u64) {
    buf.extend_from_slice(&n.to_le_bytes());
}

fn put_bytes(buf: &mut Vec<u8>, b: &[u8]) {
    put_u64_len(buf, b.len() as u64);
    buf.extend_from_slice(b);
}

// ═══════════════════════════════════════════════════════════════════════════
// The F1r2flyCrypto Adapter
// ═══════════════════════════════════════════════════════════════════════════
#[derive(Debug)]
pub struct F1r3flyCryptoAdapter {
    algorithm: SigAlgorithm, // Chosen Algorithm from SigAlgorithm
}

// This implementation in here are the functions that belong to F1r3flyCryptoAdapter struct".
impl F1r3flyCryptoAdapter {
    // Creates new adapter based on chosen algorithm
    pub fn new(algorithm: SigAlgorithm) -> Self {
        Self { algorithm }
    }
    // return the chosen algorithm for the adapter
    pub fn algorithm(&self) -> SigAlgorithm {
        self.algorithm
    }
    // Create adapter from algorithm string ("secp256k1", "ed25519"). Returns Ok or Error
    pub fn from_algorithm_str(s: &str) -> Result<Self, CryptoError> {
        // makes the input case-insensitive because of diffrent forms of writing.
        match s.to_lowercase().as_str() {
            // empty string is treated as secp256k1 by default, matching f1r3node's behavior.
            "" => Ok(Self::new(SigAlgorithm::Secp256k1)),
            "secp256k1" => Ok(Self::new(SigAlgorithm::Secp256k1)),
            "ed25519" => Ok(Self::new(SigAlgorithm::Ed25519)),
            // anything else is an error
            _other => Err(CryptoError::InvalidSignature),
        }
    }
}

// ── The Actual Verification Logic ────────────────────────────────────────────────────// ═══════════════════════════════════════════════════════════════════════════
// Implementing the CryptoVerifier Trait for our adapter.
// This checks if a block's signature is valid according to the chosen algorithm.

impl CryptoVerifier for F1r3flyCryptoAdapter {
    type Error = CryptoError;
    // Verify block is function blocklace calls on every new block.
    fn verify_block(
        &self,
        content: &BlockContent,
        signature: &[u8],
        creator: &NodeId,
    ) -> Result<(), Self::Error> {
        // Recompute the content hash; the creator's signature must match it, so changed content fails verification.
        let hash: [u8; 32] = hash_content(content); // Get the 32-byte hash of the block content.

        // reject empty signatures
        if signature.is_empty() {
            return Err(CryptoError::InvalidSignatureLength {
                expected: 1,
                actual: 0,
            });
        }

        // Verify the signature with the chosen algorithm.
        let is_valid = match self.algorithm {
            SigAlgorithm::Secp256k1 => Secp256k1.verify(&hash, &creator.0, signature)?,
            SigAlgorithm::Ed25519 => Ed25519.verify(&hash, &creator.0, signature)?,
        };

        if is_valid {
            Ok(())
        } else {
            Err(CryptoError::InvalidSignature)
        }
    }
}
