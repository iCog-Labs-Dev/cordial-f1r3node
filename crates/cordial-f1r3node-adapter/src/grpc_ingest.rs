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
//!    - Resolves native parent hashes through accepted Cordial identities
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
//! - **Pure validation**: No database writes or state mutations in mapper;
//!   accepted identity mappings are maintained separately
//! - **Type-generic**: `<V, P, Id>` parameters allow future extensibility
//! - **Trust boundary**: Mapper validates structure; adapter handles semantics

use std::collections::{HashMap, HashSet};

use anyhow::{Result, anyhow, bail};
use cordial_miners_core::Block;
use cordial_miners_core::crypto::{self, Ed25519Scheme, Secp256k1Scheme, SignatureScheme};
#[allow(unused)]
use cordial_miners_core::types::{BlockIdentity, NodeId};

use models::rust::casper::protocol::casper_message::BlockMessage as F1r3nodeBlockMessage;

use crate::block_translation::{
    BlockMessage as AdapterBlockMessage, message_from_f1r3node, message_to_block,
};
use crate::crypto_bridge::compute_adapter_snapshot_hash;

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
/// signature as an `Hc` signature. The translated identity has an empty
/// signature and must be inserted only after this wire validation succeeds.
#[derive(Debug, Clone)]
pub struct ValidatedF1r3nodeBlock {
    pub block: Block,
    pub wire_hash: [u8; 32],
    pub wire_signature: Vec<u8>,
    pub signature_algorithm: String,
}

/// Native wire hashes for host blocks already accepted into the Cordial view.
///
/// Record a block only after its translated Cordial block has been accepted.
/// The two hash domains must remain separate: native parent hashes are looked
/// up here and replaced with complete Cordial identities before content hashing.
/// Persistent ingress restores this index from the block repository at startup.
#[derive(Debug, Clone, Default)]
pub struct NativeBlockIdentityIndex {
    by_wire_hash: HashMap<[u8; 32], BlockIdentity>,
    by_cordial_identity: HashMap<BlockIdentity, [u8; 32]>,
}

impl NativeBlockIdentityIndex {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn get(&self, wire_hash: &[u8; 32]) -> Option<&BlockIdentity> {
        self.by_wire_hash.get(wire_hash)
    }

    /// Check a translated block before inserting it into the Cordial view.
    /// Distinct wire blocks can collapse to one Cordial identity because the
    /// host-to-Cordial translation omits some host fields.
    pub fn check_candidate(&self, block: &ValidatedF1r3nodeBlock) -> Result<()> {
        let identity = &block.block.identity;
        if !identity.signature.is_empty()
            || identity.content_hash != crypto::block_hash(&identity.creator, &block.block.content)
        {
            bail!("native block has an invalid translated Cordial identity");
        }
        if let Some(existing) = self.by_wire_hash.get(&block.wire_hash)
            && existing != identity
        {
            bail!("native wire hash already maps to another Cordial identity");
        }
        if let Some(existing) = self.by_cordial_identity.get(identity)
            && existing != &block.wire_hash
        {
            bail!("distinct native wire blocks collapse to one Cordial identity");
        }
        Ok(())
    }

    /// Register a checked block after the caller has accepted it. Call
    /// [`Self::check_candidate`] before insertion so collisions cannot alter
    /// the blocklace before they are rejected.
    pub fn record_accepted(&mut self, block: &ValidatedF1r3nodeBlock) -> Result<()> {
        self.check_candidate(block)?;
        let identity = &block.block.identity;
        self.by_wire_hash.insert(block.wire_hash, identity.clone());
        self.by_cordial_identity
            .insert(identity.clone(), block.wire_hash);
        Ok(())
    }

    /// Rebuild an accepted mapping from a block that survived verified recovery.
    pub fn record_recovered(&mut self, wire_hash: [u8; 32], block: &Block) -> Result<()> {
        let recovered = ValidatedF1r3nodeBlock {
            block: block.clone(),
            wire_hash,
            wire_signature: Vec::new(),
            signature_algorithm: String::new(),
        };
        self.record_accepted(&recovered)
    }
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
    /// 4. **Parent integrity**: Rejects unresolved native parents; use
    ///    [`Self::from_protobuf_with_parents`] with accepted mappings for children
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
        self.from_protobuf_with_parents(block_msg, &NativeBlockIdentityIndex::new())
    }

    /// Translate the unsigned host genesis obtained from the trusted local
    /// node store. No later block may use this exception to signature checks.
    pub fn from_trusted_unsigned_genesis(
        &self,
        block_msg: &F1r3nodeBlockMessage,
    ) -> Result<ValidatedF1r3nodeBlock> {
        if block_msg.body.state.block_number != 0
            || !block_msg.header.parents_hash_list.is_empty()
            || !block_msg.justifications.is_empty()
            || !block_msg.sender.is_empty()
            || !block_msg.sig.is_empty()
            || !block_msg.sig_algorithm.is_empty()
            || block_msg.seq_num != 0
        {
            bail!("unsigned native block is not the host genesis shape");
        }
        let wire_hash = casper::rust::util::proto_util::hash_block(block_msg);
        if block_msg.block_hash.as_ref() != wire_hash.as_ref() {
            bail!("unsigned host genesis has an invalid wire hash");
        }
        let adapter_msg = message_from_f1r3node(block_msg)
            .map_err(|error| anyhow!("Failed to convert host genesis: {error:?}"))?;
        let block = message_to_block(&adapter_msg)
            .map_err(|error| anyhow!("Failed to translate host genesis: {error:?}"))?;
        self.validate_internal_content_hash(&block)?;
        self.validate_parents(&block)?;
        if !block.content.predecessors.is_empty() {
            bail!("unsigned host genesis has predecessors");
        }
        Ok(ValidatedF1r3nodeBlock {
            block,
            wire_hash: wire_hash
                .as_ref()
                .try_into()
                .map_err(|_| anyhow!("invalid genesis hash length"))?,
            wire_signature: Vec::new(),
            signature_algorithm: String::new(),
        })
    }

    /// Validate a native block and resolve every native predecessor hash to
    /// an accepted Cordial identity. Missing parents are errors, not silently
    /// treated as Cordial content hashes or removed from the blocklace.
    pub fn from_protobuf_with_parents(
        &self,
        block_msg: &F1r3nodeBlockMessage,
        accepted: &NativeBlockIdentityIndex,
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

        let mut predecessors = HashSet::with_capacity(block.content.predecessors.len());
        for parent in &block.content.predecessors {
            let resolved = accepted.get(&parent.content_hash).ok_or_else(|| {
                anyhow!(
                    "native parent {} has no accepted Cordial identity",
                    hex::encode(parent.content_hash)
                )
            })?;
            let key_len = resolved.creator.0.len();
            if key_len != 0 && key_len != 33 && key_len != 65 {
                bail!("accepted native parent has invalid creator key length: {key_len}");
            }
            for justification in &block_msg.justifications {
                if justification.latest_block_hash.as_ref() != parent.content_hash.as_slice() {
                    continue;
                }
                let validator_len = justification.validator.len();
                if validator_len != 33 && validator_len != 65 {
                    bail!("native justification has invalid validator key length: {validator_len}");
                }
                if key_len != 0 && justification.validator.as_ref() != resolved.creator.0.as_slice()
                {
                    bail!(
                        "native parent {} justification creator does not match accepted block",
                        hex::encode(parent.content_hash)
                    );
                }
            }
            predecessors.insert(resolved.clone());
        }
        block.content.predecessors = predecessors;
        block.identity.content_hash = crypto::block_hash(&block.identity.creator, &block.content);

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

    /// Translate a locally constructed adapter message whose signature covers
    /// Cordial's internal content hash.
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

        // 3. Accept an adapter snapshot hash or the internal content hash, then
        //    check the translated identity against the internal content hash.
        //    This catches corruption of block_msg.block_hash before translation silently
        //    discards the tampered value.
        self.validate_adapter_content_hash(block_msg, &block)?;

        // 4. Validate the signature over Cordial's internal content hash
        self.validate_signature(&block, sig_algo)?;

        // 5. Validate parent references
        self.validate_parents(&block)?;

        Ok(block)
    }

    /// Verify the adapter message's block_hash matches either the snapshot hash
    /// computed from the `BlockMessage` fields via [`compute_adapter_snapshot_hash`], or the
    /// internal Cordial `block_hash` (when constructed by `block_to_message`).
    /// Also verifies that the translated block's internal content_hash is self-consistent.
    fn validate_adapter_content_hash(
        &self,
        block_msg: &AdapterBlockMessage,
        block: &Block,
    ) -> Result<()> {
        if block_msg.block_hash.len() != 32 {
            return Err(anyhow!(
                "Content hash mismatch: adapter block_hash has invalid length {}",
                block_msg.block_hash.len()
            ));
        }
        let mut declared_hash = [0u8; 32];
        declared_hash.copy_from_slice(&block_msg.block_hash);

        let recomputed_internal = crypto::block_hash(&block.identity.creator, &block.content);
        let recomputed_adapter = compute_adapter_snapshot_hash(block_msg);

        // Both accepted hashes belong to local adapter messages. Real f1r3node
        // wire hashes are validated separately by from_protobuf.
        if declared_hash != recomputed_adapter && declared_hash != recomputed_internal {
            return Err(anyhow!(
                "Content hash mismatch: adapter block_hash does not match \
                 adapter-local hash recomputation (compute_adapter_snapshot_hash) or internal content hash"
            ));
        }

        // Sanity-check that the translated block's internal content_hash
        // matches what block_hash() produces over the translated content.
        if block.identity.content_hash != recomputed_internal {
            return Err(anyhow!(
                "Internal content hash mismatch: translated identity {:?} \
                 does not match recomputed block_hash {:?}",
                block.identity.content_hash,
                recomputed_internal
            ));
        }

        Ok(())
    }

    fn validate_internal_content_hash(&self, block: &Block) -> Result<()> {
        let recomputed = crypto::block_hash(&block.identity.creator, &block.content);
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
    /// This is a **pure, stateless, byte-level** check only. Native parent
    /// existence is checked separately against the accepted identity index in
    /// `from_protobuf_with_parents`. No state mutations occur here.
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
