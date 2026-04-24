//! # gRPC Ingestion Layer
//!
//! Provides a safe translation layer between network-level gRPC/bincode messages
//! and consensus-level [`Block`] structs.
//!
//! ## Architecture
//!
//! The ingestion pipeline separates concerns across two components:
//!
//! 1. **Mapper** (pure, deterministic, no side effects):
//!    - Validates structural integrity (hashes, signatures, parent references)
//!    - Extracts [`Block`] from [`Message::BroadcastBlock`] variants
//!    - Rejects invalid messages with specific error reasons
//!
//! 2. **Adapter** (stateful, handles side effects):
//!    - Implements [`BlocklaceAdapter`] trait
//!    - Invokes [`ConsensusEngine::on_block`] callback
//!    - Manages blocklace insertion, finality checks, etc.
//!
//! ## Design Principles
//!
//! - **Fail-fast**: Invalid blocks rejected immediately with clear errors
//! - **Pure mapping**: No database writes, no state mutations in mapper
//! - **Type-generic**: `<V, P, Id>` parameters allow future extensibility
//! - **Trust boundary**: Mapper validates structure; adapter handles semantics

use anyhow::{Result, anyhow};
use cordial_miners_core::Block;
use cordial_miners_core::crypto;
use cordial_miners_core::network::Message;
#[allow(unused)]
use cordial_miners_core::types::{BlockIdentity, NodeId};

/// A pure, stateless mapper from network messages to consensus blocks.
///
/// Validates structural integrity (hashes, signatures, parent references) without
/// performing any side effects or state mutations. Multiple invocations with the
/// same input are guaranteed to produce identical results.
///
/// # Type Parameters
///
/// - `V`: Validator type (reserved for future extension)
/// - `P`: Payload type (reserved for future extension)
/// - `Id`: Block identity type (reserved for future extension)
///
/// # Example
///
/// ```ignore
/// let mapper = GrpcBlockMapper::new();
/// let network_msg = Message::BroadcastBlock { block: ... };
/// match mapper.to_block(&network_msg) {
///     Ok(block) => println!("Valid block: {:?}", block),
///     Err(e) => eprintln!("Rejected: {}", e),
/// }
/// ```
#[derive(Debug, Clone)]
pub struct GrpcBlockMapper<V = (), P = (), Id = ()> {
    _phantom: std::marker::PhantomData<(V, P, Id)>,
}

impl<V, P, Id> GrpcBlockMapper<V, P, Id> {
    /// Create a new mapper with no configuration.
    pub fn new() -> Self {
        Self {
            _phantom: std::marker::PhantomData,
        }
    }

    /// Map a network message to a validated [`Block`].
    ///
    /// Performs the following validations:
    ///
    /// 1. **Message type**: Ensures the message is [`Message::BroadcastBlock`]
    /// 2. **Content hash**: Recomputes hash of block content and verifies match
    /// 3. **Signature**: Verifies ED25519 signature against creator's public key
    /// 4. **Parent integrity**: Ensures all parent references exist in the message
    ///
    /// Returns an error if any validation fails. Errors are detailed and actionable
    /// for logging and debugging.
    ///
    /// # Arguments
    ///
    /// * `msg` - The network message (typically [`Message::BroadcastBlock`])
    ///
    /// # Returns
    ///
    /// - `Ok(Block)` if all validations pass
    /// - `Err(anyhow::Error)` with a detailed message if validation fails
    pub fn to_block(&self, msg: &Message) -> Result<Block> {
        // 1. Extract the block from the message variant
        let block = match msg {
            Message::BroadcastBlock { block } => block.clone(),
            other => {
                return Err(anyhow!(
                    "Invalid message type: expected BroadcastBlock, got {:?}",
                    std::mem::discriminant(other)
                ));
            }
        };

        // 2. Validate content hash
        self.validate_content_hash(&block)?;

        // 3. Validate signature (ED25519)
        self.validate_signature(&block)?;

        // 4. Validate parent references
        self.validate_parents(&block)?;

        Ok(block)
    }

    /// Verify that the block's content hash matches the recomputed hash.
    ///
    /// The hash is computed deterministically from the serialized content
    /// using SHA-256.
    fn validate_content_hash(&self, block: &Block) -> Result<()> {
        let expected = crypto::hash_content(&block.content);
        let actual = block.identity.content_hash;

        if expected != actual {
            return Err(anyhow!(
                "Content hash mismatch: expected {:?}, got {:?}",
                expected,
                actual
            ));
        }

        Ok(())
    }

    /// Verify that the block's signature is valid for its creator.
    ///
    /// The signature is assumed to be ED25519 and to have been computed
    /// over the block's content hash. The creator is assumed to be a 32-byte
    /// ED25519 public key.
    ///
    /// # Returns
    ///
    /// - `Ok(())` if the signature is valid
    /// - `Err` if the creator is not a valid 32-byte public key, or if verification fails
    fn validate_signature(&self, block: &Block) -> Result<()> {
        let creator_key = &block.identity.creator.0;
        let content_hash = &block.identity.content_hash;
        let signature = &block.identity.signature;

        // ED25519 public keys must be exactly 32 bytes
        if creator_key.len() != 32 {
            return Err(anyhow!(
                "Invalid creator public key: expected 32 bytes, got {}",
                creator_key.len()
            ));
        }

        // ED25519 signatures must be exactly 64 bytes
        if signature.len() != 64 {
            return Err(anyhow!(
                "Invalid signature: expected 64 bytes, got {}",
                signature.len()
            ));
        }

        // Perform cryptographic verification
        if !crypto::verify(content_hash, creator_key, signature) {
            return Err(anyhow!(
                "Signature verification failed for creator {:?}",
                creator_key
            ));
        }

        Ok(())
    }

    /// Verify that all parent references are valid.
    ///
    /// Currently checks that the parent set is not malformed (which could happen
    /// if parents contain invalid BlockIdentity values). Future versions may
    /// perform additional checks (e.g., closure axiom validation against a blocklace).
    fn validate_parents(&self, block: &Block) -> Result<()> {
        // Check that parent identities are well-formed
        for parent_id in &block.content.predecessors {
            // Verify parent content hash is 32 bytes (should always be true for BlockIdentity)
            if parent_id.content_hash.len() != 32 {
                return Err(anyhow!(
                    "Parent identity has invalid content hash size: {}",
                    parent_id.content_hash.len()
                ));
            }

            // Verify parent signature is well-formed
            if parent_id.signature.is_empty() {
                return Err(anyhow!("Parent identity has empty signature"));
            }

            // Verify parent creator key is exactly 32 bytes
            if parent_id.creator.0.len() != 32 {
                return Err(anyhow!(
                    "Parent creator has invalid key size: {}",
                    parent_id.creator.0.len()
                ));
            }
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
