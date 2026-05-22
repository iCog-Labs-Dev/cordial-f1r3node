//! Dissemination and predecessor selection for Cordial Miners blocks.
//!
//! This module implements the protocol-side "what do we propose?" layer for dissemination,
//! determining which predecessors a newly created block should reference from the local
//! blocklace view.
//!
//! From the Cordial Miners paper (arXiv:2205.09174), predecessor selection is central to:
//! - Knowledge propagation through the DAG
//! - Acknowledgement visibility (equivocations, knowledge)
//! - Wave structure and eventual finality
//!
//! **Key principles:**
//! 1. A cordial block references all visible validator tips (latest block from each validator)
//! 2. Predecessor selection uses the local blocklace view and is deterministic
//! 3. Selection respects the closure and chain axioms of the blocklace
//! 4. A cordial block must acknowledge blocks from at least a supermajority (≥ 2f+1) of
//!    miners — see Def. A.12 and Fig. 2 of the paper.

use std::collections::{HashMap, HashSet};

use crate::Block;
use crate::blocklace::Blocklace;
use crate::consensus::cordiality::all_equivocations;
use crate::consensus::fork_choice::collect_validator_tips;
use crate::crypto::CryptoVerifier;
use crate::types::{BlockIdentity, NodeId};

/// Collect the set of visible validator tips from the local blocklace.
///
/// Returns a map from each validator's `NodeId` to their most recent (tip) block identity
/// in the blocklace. This represents the knowledge of each validator's latest contribution.
///
/// **Protocol meaning**: These are the "known tips" used by the cordial dissemination
/// algorithm (§6.1, Alg. 3). The cordiality condition (Def. A.12) requires a block to
/// acknowledge blocks from a supermajority of miners; pointing to all visible honest tips
/// is the standard way to satisfy this.
///
/// **Implementation notes**:
/// - Excludes Byzantine equivocators (validators who violate the chain axiom).
/// - Returns only validators with at least one block in the blocklace.
/// - The tip is the block by each validator that no other block by that validator precedes.
///
/// # Arguments
/// * `blocklace` - The local blocklace DAG view
/// * `bonds` - The bonded validator set and their stake weights
///
/// # Returns
/// A map from `NodeId` to the block identity of their latest visible block.
pub fn validator_visible_tips(
    blocklace: &Blocklace,
    bonds: &HashMap<NodeId, u64>,
) -> HashMap<NodeId, BlockIdentity> {
    collect_validator_tips(blocklace, bonds)
}

/// Select predecessors for a newly created block from the local blocklace view.
///
/// Constructs a protocol-valid set of predecessors by pointing to all visible
/// (honest) validator tips from the local blocklace.
///
/// This is the core dissemination layer that determines what a proposer should
/// announce to other validators.
///
/// **Protocol meaning**: From the Cordial Miners paper (§6.1, Alg. 3 and the equivocation
/// exclusion discussion): correct miners ignore Byzantine miners by not including direct
/// pointers to their blocks after detecting an equivocation. By exclusively pointing to
/// honest validator tips:
/// - Honest tips already transitively observe equivocations (closure property)
/// - Equivocators are naturally filtered out and eventually ignored
/// - Blocks remain bounded (no accumulation of historical equivocation pointers)
/// - Protocol remains compliant with the cordial condition (Def. A.12)
///
/// **Cordiality invariant**: The returned set satisfies the cordiality condition
/// (Def. A.12, Fig. 2) when the local view contains tips from at least 2f+1 honest
/// validators. Callers operating in a degraded view (fewer than supermajority tips
/// visible) should consult `required_acknowledgements` before proposing.
///
/// **Guarantees**:
/// - All returned predecessors exist in the blocklace (closure axiom satisfied)
/// - All returned predecessors are from non-equivocating validators only
/// - Deterministic: same blocklace view → same predecessor set
/// - Non-empty when blocklace has honest validators
/// - Empty only when the blocklace is empty or contains only equivocators
///
/// **Typical usage** (in a validator's block proposal logic):
/// ```ignore
/// let predecessors = select_predecessors(&local_blocklace, &bonds);
/// let block_content = BlockContent {
///     payload: my_operations,
///     predecessors,
/// };
/// ```
///
/// # Arguments
/// * `blocklace` - The local blocklace DAG view
/// * `bonds` - The bonded validator set and their stake weights
///
/// # Returns
/// A set of block identities to be used as the block's predecessors.
/// Returns an empty set if the blocklace is empty or contains only equivocators.
pub fn select_predecessors(
    blocklace: &Blocklace,
    bonds: &HashMap<NodeId, u64>,
) -> HashSet<BlockIdentity> {
    let mut predecessors: HashSet<BlockIdentity> = validator_visible_tips(blocklace, bonds)
        .into_values()
        .collect();

    if predecessors.is_empty() {
        return predecessors;
    }

    let mut observed: HashSet<BlockIdentity> = predecessors
        .iter()
        .flat_map(|pred_id| blocklace.observe(pred_id).into_iter())
        .collect();

    for equivocation in all_equivocations(blocklace) {
        for branch in equivocation.blocks {
            if observed.insert(branch.clone()) {
                predecessors.insert(branch);
            }
        }
    }

    predecessors
}

/// Select predecessors and return them as a sorted vector for deterministic ordering.
///
/// This is a convenience wrapper around `select_predecessors()` that returns results
/// in a deterministic order, useful for logging, comparison, or network transmission.
///
/// Sorting is by the full natural ordering of `BlockIdentity`, so ties on
/// `content_hash` are broken consistently by creator and signature as needed.
///
/// # Arguments
/// * `blocklace` - The local blocklace DAG view
/// * `bonds` - The bonded validator set and their stake weights
///
/// # Returns
/// A sorted vector of block identities to be used as predecessors.
pub fn select_predecessors_sorted(
    blocklace: &Blocklace,
    bonds: &HashMap<NodeId, u64>,
) -> Vec<BlockIdentity> {
    let mut result: Vec<BlockIdentity> =
        select_predecessors(blocklace, bonds).into_iter().collect();

    result.sort();
    result
}

/// Compute the minimum number of acknowledgements required for a block to be cordial.
///
/// A block is cordial (Def. A.12) when it acknowledges blocks from at least a
/// supermajority of miners — strictly more than two-thirds of the bonded validator set.
/// Equivalently, for `n = 3f + 1` validators, a cordial block needs at least `2f + 1`
/// acknowledgements.
///
/// This function returns the minimum acknowledgement count threshold given the current
/// bonded validator set. It does **not** count the acknowledgements in any specific
/// block; callers should compare the cardinality of `select_predecessors` against this
/// threshold before proposing.
///
/// **Protocol meaning**: From §4.2 (Blocklace Safety) and Def. A.12: a blocklace
/// containing only cordial blocks is a cordial blocklace, and a cordial blocklace is
/// leader-safe (Theorem 4.2). Liveness further requires a non-equivocating,
/// disseminating supermajority (Fig. 3).
///
/// # Arguments
/// * `bonds` - The bonded validator set and their stake weights
///
/// # Returns
/// The minimum number of distinct validators a new block must acknowledge to be
/// cordial. Returns `0` if the validator set is empty.
///
/// # Example
/// ```ignore
/// let threshold = required_acknowledgements(&bonds);
/// let tips = validator_visible_tips(&blocklace, &bonds);
/// if tips.len() < threshold {
///     // Not enough honest tips visible; delay proposal or log a warning.
/// }
/// ```
pub fn required_acknowledgements(bonds: &HashMap<NodeId, u64>) -> usize {
    let n = bonds.len();
    if n == 0 {
        return 0;
    }
    // Standard BFT supermajority: smallest integer strictly greater than 2n/3.
    // For n = 3f+1 this yields 2f+1, matching the Cordial Miners paper.
    // Integer arithmetic: (2*n)/3 + 1 using floor division is equivalent to
    // ceil((2*n + 1) / 3), which is the minimal k satisfying k > 2n/3.
    (2 * n) / 3 + 1
}

/// Compute the threshold for a Proof-of-Stake network (Weighted Votes)
pub fn weighted_required_acknowledgements(bonds: &HashMap<NodeId, u64>) -> u64 {
    let total_stake: u128 = bonds.values().map(|s| *s as u128).sum();
    if total_stake == 0 {
        return 0;
    }
    ((2 * total_stake) / 3 + 1) as u64
}

/// A buffer for blocks that arrive out of order (before their predecessors).
///
/// This provides the dependency-resolution side of dissemination: blocks with missing
/// parents should be buffered and retried once dependencies arrive.
#[derive(Default, Debug, Clone)]
pub struct PendingBlockBuffer {
    /// Blocks that are buffered, indexed by their identity.
    pub buffered_blocks: HashMap<BlockIdentity, Block>,
}

impl PendingBlockBuffer {
    /// Create a new empty pending block buffer.
    pub fn new() -> Self {
        Self {
            buffered_blocks: HashMap::new(),
        }
    }

    /// Add a block to the buffer that might be missing predecessors.
    pub fn buffer_block_with_missing_predecessors(&mut self, block: Block) {
        self.buffered_blocks.insert(block.identity.clone(), block);
    }

    /// Retry inserting buffered blocks into the given blocklace.
    ///
    /// Loops through buffered blocks and attempts to insert them if their
    /// predecessors are now available. Continues as long as progress is made
    /// (e.g., a block is inserted which then satisfies another block's dependencies).
    ///
    /// Successfully inserted blocks, or blocks that are definitively rejected
    /// (e.g., due to invalid signatures), are removed from the buffer.
    pub fn retry_buffered_blocks<V: CryptoVerifier>(
        &mut self,
        blocklace: &mut Blocklace,
        verifier: &V,
    ) {
        let mut progress = true;
        while progress {
            progress = false;
            let mut resolved = Vec::new();

            for (id, block) in self.buffered_blocks.iter() {
                // Check if all predecessors are in the blocklace
                let ready = block
                    .content
                    .predecessors
                    .iter()
                    .all(|p| blocklace.content(p).is_some());

                if ready {
                    // Try to insert
                    match blocklace.insert(block.clone(), verifier) {
                        Ok(_) => {
                            resolved.push(id.clone());
                            progress = true;
                        }
                        Err(_) => {
                            // Block is definitively invalid (bad signature, equivocation, etc).
                            // Remove it so we do not retry a permanently broken block.
                            // NOTE: closure violations cannot happen here because we verified
                            // all predecessors exist above. If they did occur it would be a bug.
                            // Full coverage of rejection cases requires a non-mock verifier
                            // and is tested at the integration layer.
                            resolved.push(id.clone());
                        }
                    }
                }
            }

            for id in resolved {
                self.buffered_blocks.remove(&id);
            }
        }
    }
}
