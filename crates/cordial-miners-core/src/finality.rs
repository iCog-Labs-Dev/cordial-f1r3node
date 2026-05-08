//! Finality module - implements approval logic from Definition 18 of the Cordial Miners paper.
//!
//! This module provides the core building block for the consensus protocol:
//! **Approval** — a block approves a candidate if it observes the candidate in its causal
//! history and does not observe any conflicting equivocations from the candidate's creator.
//!
//! ## Definition 18: Approves
//!
//! From the Cordial Miners paper:
//! - Block `b` **approves** candidate block `b'` if:
//!   1. `b` observes `b'` (i.e., `b'` is in `b`'s causal history)
//!   2. `b` does NOT observe any equivocating block of `b'`
//!
//! An equivocating block of `b'` is any other block `b''` created by the same node as `b'`
//! (i.e., `node(b') = node(b'')`) that violates the chain axiom (both `b'` and `b''` exist
//! in the blocklace but neither precedes the other).
//!
//! ## Proof-of-Stake Extension
//!
//! The paper's original definition is extended to support Proof-of-Stake consensus by
//! weighing approval via ValidatorSet. The weight of an approval is determined by the
//! stake of the block's creator in the ValidatorSet.
//!
//! For a candidate block to reach **approval threshold**:
//! - The sum of weights of all blocks in an observer's causal history that approve
//!   the candidate must exceed the threshold defined by ApprovalThreshold.
//! - Threshold arithmetic avoids floating-point using integer cross-multiplication:
//!   `support_weight * threshold.denominator > total_weight * threshold.numerator`

use crate::blocklace::Blocklace;
use crate::cordiality::ValidatorSet;
use crate::types::BlockIdentity;

/// Threshold for approval via weighted stake.
///
/// Represents a rational number `numerator / denominator` that defines the minimum
/// fraction of total validator stake required for a block to be approved.
///
/// # Examples
///
/// - `2/3` majority: `ApprovalThreshold { numerator: 2, denominator: 3 }`
/// - `1/2` majority: `ApprovalThreshold { numerator: 1, denominator: 2 }`
/// - Unanimous: `ApprovalThreshold { numerator: 1, denominator: 1 }`
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ApprovalThreshold {
    /// Numerator of the threshold fraction
    pub numerator: u128,
    /// Denominator of the threshold fraction
    pub denominator: u128,
}

impl ApprovalThreshold {
    /// Create a new ApprovalThreshold with the given numerator and denominator.
    ///
    /// # Panics
    ///
    /// Panics if denominator is zero or if numerator > denominator (invalid threshold).
    pub fn new(numerator: u128, denominator: u128) -> Self {
        assert!(denominator > 0, "denominator must be > 0");
        assert!(
            numerator <= denominator,
            "numerator ({}) must be <= denominator ({})",
            numerator,
            denominator
        );
        Self {
            numerator,
            denominator,
        }
    }

    /// Check if a weight exceeds this threshold relative to total weight.
    ///
    /// Uses integer cross-multiplication to avoid floating-point arithmetic:
    /// - `support_weight * threshold.denominator > total_weight * threshold.numerator`
    ///
    /// # Returns
    ///
    /// `true` if the weight threshold is crossed, `false` otherwise.
    pub fn is_exceeded(&self, support_weight: u128, total_weight: u128) -> bool {
        support_weight * self.denominator > total_weight * self.numerator
    }
}

/// Determine whether a block in the causal history approves a candidate block.
///
/// # Definition
///
/// Block `b` **approves** candidate block `b'` if:
/// 1. `b` observes `b'` (i.e., `b'` is in `b`'s causal history, inclusive)
/// 2. `b` does NOT observe any equivocating block of `b'`
///
/// An equivocating block of `b'` is any other block created by the same node as `b'`
/// that is not comparable under the precedence relation (violating the chain axiom).
///
/// # Parameters
///
/// - `blocklace`: The current blocklace containing all known blocks
/// - `b`: The block that may approve the candidate
/// - `b_prime`: The candidate block to check approval for
///
/// # Returns
///
/// `true` if `b` approves `b_prime`, `false` otherwise.
pub fn approves(blocklace: &Blocklace, b: &BlockIdentity, b_prime: &BlockIdentity) -> bool {
    // Step 1: Check if b observes b_prime
    let observed = blocklace.observe(b);
    if !observed.contains(b_prime) {
        return false;
    }

    // Step 2: Get all blocks created by the creator of b_prime
    let b_prime_block = match blocklace.get(b_prime) {
        Some(block) => block,
        None => return false, // b_prime not in blocklace
    };
    let b_prime_creator = &b_prime_block.identity.creator;
    let creator_blocks = blocklace.blocks_by(b_prime_creator);

    // Step 3: Check if b observes any equivocating block of b_prime
    for other_block in creator_blocks {
        if other_block.identity == *b_prime {
            // Skip b_prime itself
            continue;
        }

        // Check if both blocks are observed by b and neither precedes the other
        // (i.e., they are incomparable under precedence)
        if observed.contains(&other_block.identity) {
            let b_prime_precedes_other = blocklace.precedes(b_prime, &other_block.identity);
            let other_precedes_b_prime = blocklace.precedes(&other_block.identity, b_prime);

            // If neither precedes the other, they are equivocating
            if !b_prime_precedes_other && !other_precedes_b_prime {
                return false;
            }
        }
    }

    // b observes b_prime and no equivocations
    true
}

/// Determine weighted approval: a block approves a candidate if it observes the candidate
/// and no equivocations, considering the ApprovalThreshold.
///
/// # Definition
///
/// An observer block `b` **approves** a candidate block `b'` under a ValidatorSet if:
/// 1. `b` approves `b'` according to the basic approval rules (no equivocations observed)
/// 2. The sum of weights (from ValidatorSet) of all blocks in `b`'s causal history that
///    approve `b'` exceeds the ApprovalThreshold relative to the total validator stake
///
/// # Parameters
///
/// - `blocklace`: The current blocklace
/// - `b`: The observer block
/// - `b_prime`: The candidate block
/// - `threshold`: The approval threshold (e.g., 2/3 majority)
/// - `validators`: The ValidatorSet providing stake weights
///
/// # Returns
///
/// `true` if the weighted approval threshold is crossed, `false` otherwise.
pub fn approve<VS>(
    blocklace: &Blocklace,
    b: &BlockIdentity,
    b_prime: &BlockIdentity,
    threshold: ApprovalThreshold,
    validators: &VS,
) -> bool
where
    VS: for<'a> ValidatorSet<&'a crate::types::NodeId, Weight = u128>,
{
    // Step 1: Check if b itself approves b_prime (basic approval check)
    if !approves(blocklace, b, b_prime) {
        return false;
    }

    // Step 2: Find all blocks in b's causal history that approve b_prime
    let observed = blocklace.observe(b);
    let mut support_weight: u128 = 0;

    for block_id in &observed {
        if approves(blocklace, block_id, b_prime) {
            // Get the creator of this approving block
            if let Some(block) = blocklace.get(block_id) {
                // Get the weight of this validator
                // The ValidatorSet expects a reference to the validator ID
                if let Some(weight) = validators.weight_of(&&block.identity.creator) {
                    support_weight += weight;
                }
            }
        }
    }

    // Step 3: Get total weight and check threshold
    let total_weight = validators.total_weight();
    if total_weight == 0 {
        return false; // No validators, cannot meet threshold
    }

    threshold.is_exceeded(support_weight, total_weight)
}
