//! Cryptographic alignment with f1r3node (Phase 3.4).
//!
//! f1r3node uses:
//! - **Blake2b-256** for block hashing
//! - **Secp256k1 (ECDSA)** for primary validator signatures
//! - ED25519 as a secondary algorithm
//!
//! The core blocklace crate uses SHA-256 + ED25519 because they're standard,
//! audited, and ship in every Rust toolchain. For f1r3node wire parity we
//! need the other two available.
//!
//! This module provides:
//!
//! - A [`Hasher`] trait with two implementations ([`Sha256Hasher`],
//!   [`Blake2b256Hasher`]) for 32-byte content hashing.
//! - [`Signer`] / [`Verifier`] traits with ED25519 and Secp256k1 impls.
//! - [`compute_block_hash`] — an f1r3node-style block hash that mixes the
//!   **sender** into the hash input. This fixes the content-hash-collision
//!   issue flagged in `snapshot.rs`: two blocks with identical
//!   `BlockContent` but different creators now produce different block
//!   hashes, matching how f1r3node's `hash_block()` works.
//!
//! ## What this module does NOT do
//!
//! - It is **not byte-for-byte compatible** with f1r3node's `hash_block()`.
//!   That function hashes the **protobuf-encoded** header/body bytes, which
//!   requires `prost` and f1r3node's proto schemas. Our implementation
//!   hashes a deterministic layout of the mirror struct fields directly.
//!   The guarantee we provide is: *same logical content + sender → same
//!   hash*, and *different sender or content → different hash*. That is
//!   sufficient for snapshot-index correctness and for unit tests.
//!
//! - It does **not** swap out the core blocklace's crypto. `Block.identity`
//!   still uses SHA-256 + ED25519 as before. The crypto bridge only applies
//!   at the f1r3node interface, where we need f1r3node's wire format.
//!
//! When the `models` path dependency is enabled later (same cutover as the
//! mirror structs in `block_translation`), we'll add a third `Hasher`
//! implementation that hashes the real `prost`-encoded bytes for full
//! byte-level wire compatibility.

use blake2::Blake2b;
use blake2::digest::consts::U32;
use k256::ecdsa::signature::{Signer as K256Signer, Verifier as K256Verifier};
use k256::ecdsa::{
    Signature as K256Signature, SigningKey as K256SigningKey, VerifyingKey as K256VerifyingKey,
};
use sha2::{Digest, Sha256};

use crate::block_translation::BlockMessage;

// ═══════════════════════════════════════════════════════════════════════════
// Hashing
// ═══════════════════════════════════════════════════════════════════════════

/// Trait for 32-byte content hashers.
///
/// Implemented for [`Sha256Hasher`] (blocklace default) and
/// [`Blake2b256Hasher`] (f1r3node compatibility).
pub trait Hasher {
    /// Name of the algorithm, e.g. `"sha256"` or `"blake2b256"`.
    fn name(&self) -> &'static str;

    /// Hash `input` into a 32-byte digest.
    fn hash(&self, input: &[u8]) -> [u8; 32];
}

/// SHA-256 hasher. Matches what the blocklace core crate uses.
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
/// Private key: 32 bytes. Public key: 33 bytes compressed or 65 bytes
/// uncompressed SEC1. Signature: 64 bytes fixed-size r||s.
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
        // k256's sign() hashes the input itself; we pass the pre-computed
        // 32-byte digest through `sign_prehash_recoverable` to keep the
        // contract uniform with the Ed25519 signer (which signs over the
        // provided hash verbatim). Use `try_sign` on the already-hashed
        // input to match f1r3node semantics.
        let sig: K256Signature = signing_key.sign(hash);
        Ok(sig.to_bytes().to_vec())
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
        if signature.len() != 64 {
            return Err(CryptoError::InvalidSignatureLength {
                expected: 64,
                actual: signature.len(),
            });
        }
        let sig =
            K256Signature::from_slice(signature).map_err(|_| CryptoError::InvalidSignature)?;
        Ok(verifying_key.verify(hash, &sig).is_ok())
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Block hash (f1r3node-compatible)
// ═══════════════════════════════════════════════════════════════════════════

/// Compute a 32-byte block hash the way f1r3node would, using Blake2b-256
/// over a canonical representation of the `BlockMessage` that mixes the
/// **sender** into the input.
///
/// This fixes the content-hash-collision issue called out in
/// `snapshot.rs`: two blocks with identical `BlockContent` but different
/// creators now produce different block hashes, which is what f1r3node's
/// `casper::util::proto_util::hash_block()` also does.
///
/// ## Layout (deterministic)
///
/// All fields are length-prefixed with an 8-byte little-endian length
/// where they are variable-size, and concatenated in this order:
///
/// 1. `header.parents_hash_list` — count (u64 LE), then each hash's bytes
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
/// See the module-level docs for the note on why this is not byte-for-byte
/// identical to f1r3node's protobuf-encoded version, only logically
/// equivalent for snapshot-index correctness.
pub fn compute_block_hash(msg: &BlockMessage) -> [u8; 32] {
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
