//! # gRPC Ingestion Layer
//!
//! Provides a safe translation layer from f1r3node protobuf wire format
//! (`BlockMessage`) through validation to consensus-level [`Block`] structs.
//!
//! ## Architecture
//!
//! The ingestion pipeline separates concerns across three components:
//!
//! 1. **Translator** (protobuf decoding via `message_to_block()`):
//!    - Converts f1r3node [`BlockMessage`] (protobuf) to internal [`Block`] format
//!    - Merges parents and justifications into unified predecessor set
//!    - Extracts `sig_algorithm` for downstream validation
//!
//! 2. **Validator** (pure, deterministic, no side effects):
//!    - Validates structural integrity (hashes, signatures)
//!    - Uses `sig_algorithm` from protobuf message
//!    - Rejects invalid blocks with specific error reasons
//!
//! 3. **Adapter** (stateful, handles side effects):
//!    - Implements [`BlocklaceAdapter`] trait
//!    - Invokes [`ConsensusEngine::on_block`] callback
//!    - Manages blocklace insertion, finality checks, etc.
//!
//! ## Design Principles
//!
//! - **Protobuf-first**: Accepts f1r3node wire format directly
//! - **Algorithm-driven**: Signature verification dispatched from protobuf `sig_algorithm`
//! - **Fail-fast**: Invalid blocks rejected immediately with clear errors
//! - **Pure validation**: No database writes, no state mutations in mapper
//! - **Type-generic**: `<V, P, Id>` parameters allow future extensibility
//! - **Trust boundary**: Mapper validates structure; adapter handles semantics

use anyhow::{Result, anyhow};
use cordial_miners_core::Block;
use cordial_miners_core::crypto::{self, Ed25519Scheme, Secp256k1Scheme, SignatureScheme};
#[allow(unused)]
use cordial_miners_core::types::{BlockIdentity, NodeId};

use models::rust::casper::protocol::casper_message::BlockMessage as F1r3nodeBlockMessage;

use crate::block_translation::{
    BlockMessage as AdapterBlockMessage, message_from_f1r3node, message_to_block,
};

/// A pure, stateless validator for protobuf blocks from f1r3node.
///
/// Accepts f1r3node [`F1r3nodeBlockMessage`] (protobuf wire format), translates to internal
/// [`Block`] format, and validates structural integrity (hashes, signatures, parent
/// references). The signature algorithm is dispatched from the protobuf message.
///
/// Multiple invocations with the same input are guaranteed to produce identical results.
///
/// # Type Parameters
///
/// - `V`: Validator type (reserved for future extension)
/// - `P`: Payload type (reserved for future extension)
/// - `Id`: Block identity type (reserved for future extension)
///
/// # Cryptographic Algorithms
///
/// - **Hashing**: Blake2b-256 (f1r3node alignment)
/// - **Signature verification**: Extracted from protobuf `BlockMessage.sig_algorithm`
///   - `"secp256k1"`: Secp256k1 ECDSA with DER encoding
///   - `"ed25519"`: EdDSA with 64-byte signatures
///
/// # Example
///
/// ```ignore
/// let mapper = GrpcBlockMapper::new();
/// let block_msg = BlockMessage { sig_algorithm: "secp256k1".into(), ... };
/// match mapper.from_protobuf(&block_msg) {
///     Ok(block) => println!("Valid block: {:?}", block),
///     Err(e) => eprintln!("Rejected: {}", e),
/// }
/// ```
#[derive(Debug, Clone)]
pub struct GrpcBlockMapper<V = (), P = (), Id = ()> {
    _phantom: std::marker::PhantomData<(V, P, Id)>,
}

/// A wire-authenticated f1r3node block and its translated Cordial content.
///
/// The wire signature is valid for [`Self::wire_hash`] (`Hf`), not for the
/// translated block's Cordial content hash (`Hc`). Keeping the attestation
/// separate prevents downstream Cordial verifiers from interpreting an `Hf`
/// signature as an `Hc` signature.
#[derive(Debug, Clone)]
pub struct ValidatedF1r3nodeBlock {
    pub block: Block,
    pub wire_hash: [u8; 32],
    pub wire_signature: Vec<u8>,
    pub signature_algorithm: String,
}

impl<V, P, Id> GrpcBlockMapper<V, P, Id> {
    /// Create a new mapper with no configuration.
    pub fn new() -> Self {
        Self {
            _phantom: std::marker::PhantomData,
        }
    }

    /// Translate and validate a protobuf [`F1r3nodeBlockMessage`] to a consensus [`Block`].
    ///
    /// Performs the following steps:
    ///
    /// 1. **Wire hash validation**: Recomputes f1r3node's protobuf hash with
    ///    `proto_util::hash_block`
    /// 2. **Signature validation**: Verifies the signature over that wire hash
    /// 3. **Translation**: Converts the validated message into Cordial's internal
    ///    [`Block`] and independently derives its content hash
    /// 4. **Parent integrity**: Ensures all parent references are well-formed
    ///
    /// Returns an error if any step fails. Errors are detailed and actionable
    /// for logging and debugging.
    ///
    /// # Arguments
    ///
    /// * `block_msg` - The protobuf block message from f1r3node
    ///
    /// # Returns
    ///
    /// - `Ok(ValidatedF1r3nodeBlock)` if all validations pass
    /// - `Err(anyhow::Error)` with a detailed message if translation or validation fails
    pub fn from_protobuf(
        &self,
        block_msg: &F1r3nodeBlockMessage,
    ) -> Result<ValidatedF1r3nodeBlock> {
        // Validate the original f1r3node model before the intentionally lossy
        // translation drops execution-event details.
        let wire_hash = casper::rust::util::proto_util::hash_block(block_msg);
        if block_msg.block_hash.as_ref() != wire_hash.as_ref() {
            return Err(anyhow!(
                "Wire hash mismatch: block_hash {:?} does not match f1r3node hash {:?}",
                block_msg.block_hash,
                wire_hash
            ));
        }

        self.validate_signature_bytes(
            wire_hash.as_ref(),
            block_msg.sender.as_ref(),
            block_msg.sig.as_ref(),
            &block_msg.sig_algorithm,
        )?;

        let adapter_msg = message_from_f1r3node(block_msg)
            .map_err(|e| anyhow!("Failed to convert f1r3node BlockMessage: {e:?}"))?;
        let mut block = message_to_block(&adapter_msg)
            .map_err(|e| anyhow!("Failed to translate BlockMessage to Block: {e:?}"))?;

        self.validate_internal_content_hash(&block)?;
        self.validate_parents(&block)?;

        let wire_hash: [u8; 32] = wire_hash
            .as_ref()
            .try_into()
            .map_err(|_| anyhow!("f1r3node returned a non-32-byte block hash"))?;
        let wire_signature = std::mem::take(&mut block.identity.signature);

        Ok(ValidatedF1r3nodeBlock {
            block,
            wire_hash,
            wire_signature,
            signature_algorithm: block_msg.sig_algorithm.clone(),
        })
    }

    /// Translate an adapter-owned message that is already in Cordial's hash
    /// domain.
    ///
    /// This compatibility path is for locally constructed adapter messages;
    /// network protobuf messages must use [`Self::from_protobuf`] so their
    /// f1r3node wire hash and signature are verified before translation.
    pub fn from_adapter_message(&self, block_msg: &AdapterBlockMessage) -> Result<Block> {
        let block = message_to_block(block_msg)
            .map_err(|e| anyhow!("Failed to translate adapter BlockMessage to Block: {e:?}"))?;

        // 2. Extract signature algorithm (case-insensitive, default to secp256k1)
        let sig_algo = block_msg.sig_algorithm.to_lowercase();
        let sig_algo = if sig_algo.is_empty() {
            "secp256k1"
        } else {
            &sig_algo
        };

        // 3. Validate content hash (Blake2b-256):
        //    First verify the wire-format block_hash matches what we recompute from
        //    content, then verify it matches the identity stored in the translated block.
        //    This catches corruption of block_msg.block_hash before translation silently
        //    discards the tampered value.
        self.validate_adapter_content_hash(block_msg, &block)?;

        // 4. Validate signature (algorithm-specific from protobuf)
        self.validate_signature(&block, sig_algo)?;

        // 5. Validate parent references
        self.validate_parents(&block)?;

        Ok(block)
    }

    /// Verify the wire-format block_hash matches the recomputed hash from content,
    /// and that the translated identity also matches.
    ///
    /// Checks both the raw wire `block_msg.block_hash` (before translation can discard it)
    /// and the translated `block.identity.content_hash`, using Blake2b-256 (f1r3node alignment).
    fn validate_adapter_content_hash(
        &self,
        block_msg: &AdapterBlockMessage,
        block: &Block,
    ) -> Result<()> {
        let recomputed = crypto::hash_content(&block.content);

        // Verify wire-format hash against recomputed hash (catches tampering of block_hash field)
        if block_msg.block_hash.len() != 32 {
            return Err(anyhow!(
                "Content hash mismatch: wire block_hash has invalid length {}",
                block_msg.block_hash.len()
            ));
        }
        let mut wire_hash = [0u8; 32];
        wire_hash.copy_from_slice(&block_msg.block_hash);
        if wire_hash != recomputed {
            return Err(anyhow!(
                "Content hash mismatch: wire block_hash {wire_hash:?} does not match recomputed {recomputed:?}"
            ));
        }

        // Sanity-check translated identity
        if block.identity.content_hash != recomputed {
            return Err(anyhow!(
                "Content hash mismatch: translated identity {:?} does not match recomputed {recomputed:?}",
                block.identity.content_hash
            ));
        }

        Ok(())
    }

    fn validate_internal_content_hash(&self, block: &Block) -> Result<()> {
        let recomputed = crypto::hash_content(&block.content);
        if block.identity.content_hash != recomputed {
            return Err(anyhow!(
                "Content hash mismatch: translated identity {:?} does not match recomputed {recomputed:?}",
                block.identity.content_hash
            ));
        }
        Ok(())
    }

    /// Verify that the block's signature is valid for its creator.
    ///
    /// The signature verification algorithm is dispatched based on the `sig_algorithm`
    /// parameter (default: "secp256k1" for f1r3node alignment).
    ///
    /// Supported algorithms:
    /// - `"secp256k1"`: Secp256k1 ECDSA (DER-encoded, typically 71-72 bytes)
    /// - `"ed25519"`: EdDSA (64 bytes)
    ///
    /// # Arguments
    ///
    /// * `block` - The block to verify
    /// * `sig_algorithm` - Algorithm identifier (case-insensitive, e.g. "secp256k1")
    ///
    /// # Returns
    ///
    /// - `Ok(())` if the signature is valid
    /// - `Err` if the signature verification fails, public key is invalid, or algorithm is unknown
    fn validate_signature(&self, block: &Block, sig_algorithm: &str) -> Result<()> {
        self.validate_signature_bytes(
            &block.identity.content_hash,
            &block.identity.creator.0,
            &block.identity.signature,
            sig_algorithm,
        )
    }

    fn validate_signature_bytes(
        &self,
        signed_hash: &[u8],
        creator_key: &[u8],
        signature: &[u8],
        sig_algorithm: &str,
    ) -> Result<()> {
        let sig_algorithm = sig_algorithm.to_lowercase();
        let signed_hash: &[u8; 32] = signed_hash.try_into().map_err(|_| {
            anyhow!(
                "Signed hash has invalid length: {} bytes (expected 32)",
                signed_hash.len()
            )
        })?;

        // Signature must not be empty
        if signature.is_empty() {
            return Err(anyhow!("Signature cannot be empty"));
        }

        // Dispatch verification based on algorithm
        let valid = match sig_algorithm.as_str() {
            "secp256k1" => Secp256k1Scheme.verify(signed_hash, creator_key, signature),
            "ed25519" => Ed25519Scheme.verify(signed_hash, creator_key, signature),
            other => {
                return Err(anyhow!(
                    "Unknown signature algorithm: {other} (expected 'secp256k1' or 'ed25519')"
                ));
            }
        };

        if !valid {
            return Err(anyhow!(
                "Signature verification failed for creator {creator_key:?} using algorithm '{sig_algorithm}'"
            ));
        }

        Ok(())
    }

    /// Verify that all parent references are structurally valid.
    ///
    /// This is a **pure, stateless, byte-level** check only. Parent *existence* is not
    /// verified here — that is a semantic concern belonging to the consensus layer which
    /// has access to the full blocklace. No external lookups or state mutations occur.
    ///
    /// Checks performed per predecessor [`BlockIdentity`]:
    /// - `content_hash`: statically `[u8; 32]` — no runtime check needed.
    /// - `creator` key: must be 33 bytes (Secp256k1 compressed) or 65 bytes (uncompressed).
    /// - `signature`: intentionally **not** checked — in the wire format only the hash is
    ///   transmitted; predecessor signatures are legitimately absent and will be empty.
    fn validate_parents(&self, block: &Block) -> Result<()> {
        for parent_id in &block.content.predecessors {
            // Validate creator public key byte length.
            // Valid Secp256k1 sizes: 33 bytes (compressed) or 65 bytes (uncompressed).
            // A key of any other length cannot be a well-formed Secp256k1 public key.
            let key_len = parent_id.creator.0.len();
            if key_len != 33 && key_len != 65 {
                return Err(anyhow!(
                    "Parent creator has invalid key length: {key_len} bytes (expected 33 or 65)"
                ));
            }

            // Signatures are NOT checked: wire-format predecessors carry only content_hash
            // and a creator key; their signatures are absent by design. Full predecessor
            // validation (including signature verification) is deferred to the consensus layer.
        }

        Ok(())
    }
}

impl<V, P, Id> Default for GrpcBlockMapper<V, P, Id> {
    fn default() -> Self {
        Self::new()
    }
}

/// Trait for handling successfully-mapped blocks and invoking consensus callbacks.
///
/// This trait abstracts the consensus engine integration, separating the pure
/// mapping logic from side-effect-carrying consensus operations.
///
/// # Type Parameter
///
/// `Id` - The block identifier type (typically [`BlockIdentity`])
///
/// # Example
///
/// ```ignore
/// struct MyAdapter {
///     engine: Box<dyn ConsensusEngine<BlockId = BlockIdentity>>,
/// }
///
/// impl BlocklaceAdapter<BlockIdentity> for MyAdapter {
///     fn on_block(&mut self, block: Block) -> Result<()> {
///         self.engine.on_block(block.identity)?;
///         Ok(())
///     }
/// }
/// ```
pub trait BlocklaceAdapter<Id> {
    /// Invoke consensus callback for a successfully-validated block.
    ///
    /// This method receives a block that has passed structural validation
    /// and is ready for semantic validation and insertion into the blocklace.
    ///
    /// # Arguments
    ///
    /// * `block` - A structurally-valid [`Block`]
    ///
    /// # Returns
    ///
    /// - `Ok(())` if the callback succeeds
    /// - `Err` if the consensus engine rejects the block
    ///
    /// # Semantics
    ///
    /// This method is responsible for:
    /// - Invoking the consensus engine's `on_block` callback
    /// - Potentially inserting the block into a blocklace
    /// - Triggering finality checks, fork choice updates, etc.
    fn on_block(&mut self, block: Block) -> Result<()>;
}
