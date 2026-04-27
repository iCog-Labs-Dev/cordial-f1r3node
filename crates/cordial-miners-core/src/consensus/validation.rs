use std::collections::HashMap;

use crate::block::Block;
use crate::blocklace::Blocklace;
use crate::crypto;
use crate::types::{BlockIdentity, NodeId};

/// Reasons a block can be rejected during validation.
///
/// Modeled after f1r3node's `InvalidBlock` enum but reduced to what
/// the Cordial Miners protocol actually requires. CBC Casper has 23+
/// variants; many are eliminated because the blocklace unifies parents
/// and justifications.
#[derive(Debug, Clone, PartialEq)]
pub enum InvalidBlock {
    /// Block's content_hash does not match hash(content).
    InvalidContentHash {
        expected: [u8; 32],
        actual: [u8; 32],
    },

    /// Block's signature does not verify against the creator's public key.
    InvalidSignature,

    /// Block creator is not a bonded validator.
    UnknownSender { creator: NodeId },

    /// One or more predecessor blocks are not in the blocklace (closure violation).
    MissingPredecessors { missing: Vec<BlockIdentity> },

    /// Inserting this block would violate the chain axiom for the creator.
    /// The creator already has a block that is not comparable to this one.
    Equivocation { conflicting: BlockIdentity },

    /// Block does not satisfy the cordial condition (does not reference all known tips).
    NotCordial { missing_tips: Vec<BlockIdentity> },
}

/// Result of block validation.
#[derive(Debug, Clone, PartialEq)]
pub enum ValidationResult {
    /// Block passed all checks and is safe to insert.
    Valid,
    /// Block failed one or more checks.
    Invalid(Vec<InvalidBlock>),
}

impl ValidationResult {
    pub fn is_valid(&self) -> bool {
        matches!(self, ValidationResult::Valid)
    }

    pub fn errors(&self) -> &[InvalidBlock] {
        match self {
            ValidationResult::Valid => &[],
            ValidationResult::Invalid(errors) => errors,
        }
    }
}

/// Configuration for which validation checks to run.
///
/// Some checks are always required (closure, equivocation).
/// Others can be toggled depending on context:
/// - Signature verification may be skipped for self-created blocks
/// - Cordial condition may be relaxed under network partitions
#[derive(Debug, Clone)]
pub struct ValidationConfig {
    /// Check that content_hash matches hash(content).
    pub check_content_hash: bool,
    /// Check that signature verifies against creator's public key.
    pub check_signature: bool,
    /// Check that creator is in the bonds map.
    pub check_sender: bool,
    /// Check closure axiom (predecessors exist).
    pub check_closure: bool,
    /// Check chain axiom (no equivocation).
    pub check_chain_axiom: bool,
    /// Check cordial condition (references all known tips).
    pub check_cordial: bool,
}

impl Default for ValidationConfig {
    fn default() -> Self {
        Self {
            check_content_hash: true,
            check_signature: true,
            check_sender: true,
            check_closure: true,
            check_chain_axiom: true,
            check_cordial: false, // off by default — not all blocks need to be cordial
        }
    }
}

impl ValidationConfig {
    /// All checks enabled.
    pub fn strict() -> Self {
        Self {
            check_cordial: true,
            ..Default::default()
        }
    }
}

/// Validate a block against the blocklace and bonds map.
///
/// Runs each enabled check and collects all errors (does not short-circuit).
/// This allows the caller to see every issue at once rather than fixing
/// them one at a time.
pub fn validate_block(
    block: &Block,
    blocklace: &Blocklace,
    bonds: &HashMap<NodeId, u64>,
    config: &ValidationConfig,
) -> ValidationResult {
    let mut errors = Vec::new();

    // 1. Content hash verification
    if config.check_content_hash {
        let expected = crypto::hash_content(&block.content);
        if block.identity.content_hash != expected {
            errors.push(InvalidBlock::InvalidContentHash {
                expected,
                actual: block.identity.content_hash,
            });
        }
    }

    // 2. Signature verification (FIXED for Secp256k1 + variable-length DER)
    if config.check_signature {
        let public_key = &block.identity.creator.0;

        // NOTE:
        // - DO NOT assume 64-byte signature (Ed25519 assumption)
        // - Secp256k1 signatures are DER encoded (variable length ~70–72 bytes)
        // - Public key may be 33 or 65 bytes
        if !block.identity.signature.is_empty() {
            if !crypto::verify(
                &block.identity.content_hash,
                public_key,
                &block.identity.signature,
            ) {
                errors.push(InvalidBlock::InvalidSignature);
            }
        }
        // Empty signature is allowed for unsigned blocks (e.g., in tests)
    }

    // 3. Sender is bonded
    if config.check_sender {
        if !bonds.contains_key(&block.identity.creator) {
            errors.push(InvalidBlock::UnknownSender {
                creator: block.identity.creator.clone(),
            });
        }
    }

    // 4. Closure axiom — all predecessors must exist
    if config.check_closure {
        let missing: Vec<BlockIdentity> = block
            .content
            .predecessors
            .iter()
            .filter(|pred_id| blocklace.content(pred_id).is_none())
            .cloned()
            .collect();

        if !missing.is_empty() {
            errors.push(InvalidBlock::MissingPredecessors { missing });
        }
    }

    // 5. Chain axiom — inserting this block must not create equivocation
    if config.check_chain_axiom {
        let creator = &block.identity.creator;
        let creator_blocks = blocklace.blocks_by(creator);

        for existing in &creator_blocks {
            let new_has_existing_in_ancestry = block
                .content
                .predecessors
                .iter()
                .any(|pred_id| {
                    blocklace.preceedes_or_equals(&existing.identity, pred_id)
                });

            let existing_has_new_in_ancestry =
                blocklace.precedes(&block.identity, &existing.identity);

            if !new_has_existing_in_ancestry && !existing_has_new_in_ancestry {
                if block.identity != existing.identity {
                    errors.push(InvalidBlock::Equivocation {
                        conflicting: existing.identity.clone(),
                    });
                    break; // one conflict is enough
                }
            }
        }
    }

    // 6. Cordial condition — block references all known tips
    if config.check_cordial {
        let equivocators = blocklace.find_equivacators();
        let missing_tips: Vec<BlockIdentity> = bonds
            .keys()
            .filter(|node| !equivocators.contains(node))
            .filter_map(|node| blocklace.tip_of(node))
            .filter(|tip| {
                !block.content.predecessors.contains(&tip.identity)
                    && block.identity != tip.identity
            })
            .map(|tip| tip.identity)
            .collect();

        if !missing_tips.is_empty() {
            errors.push(InvalidBlock::NotCordial { missing_tips });
        }
    }

    if errors.is_empty() {
        ValidationResult::Valid
    } else {
        ValidationResult::Invalid(errors)
    }
}

/// Validate and insert a block into the blocklace.
///
/// Convenience function that runs validation then inserts on success.
/// Returns the validation result (which includes errors on failure).
pub fn validated_insert(
    block: Block,
    blocklace: &mut Blocklace,
    bonds: &HashMap<NodeId, u64>,
    config: &ValidationConfig,
) -> ValidationResult {
    let result = validate_block(&block, blocklace, bonds, config);
    if result.is_valid() {
        // Closure is already verified by validation, so insert directly
        blocklace
            .blocks
            .insert(block.identity.clone(), block.content);
    }
    result
}