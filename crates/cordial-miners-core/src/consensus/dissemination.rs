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
use crate::consensus::validation::{InvalidBlock, ValidationConfig, validated_insert};
use crate::types::{BlockContent, BlockIdentity, NodeId};

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

/// Reasons local proposal construction can fail.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProposalError {
    /// The local view does not contain enough visible honest validator tips to
    /// satisfy the cordial acknowledgement threshold.
    InsufficientAcknowledgements { observed: usize, required: usize },

    /// No predecessors could be selected from the current local view.
    NoPredecessorsAvailable,
}

/// Select the predecessor set for the next locally proposed block.
///
/// This helper does not decide *when* a node should propose; it only answers
/// which predecessors should be referenced once an external scheduler requests
/// a proposal.
///
/// As a special bootstrap case, an empty blocklace yields an empty predecessor
/// set so the first block can be proposed before any tips exist.
///
/// For non-empty views, predecessor selection succeeds only when the local
/// blocklace contains enough visible honest validator tips to satisfy
/// `required_acknowledgements(...)`. The returned set itself comes from
/// `select_predecessors(...)`, which may include additional known equivocation
/// branches needed for cordiality.
pub fn next_block_predecessors(
    blocklace: &Blocklace,
    bonds: &HashMap<NodeId, u64>,
) -> Result<HashSet<BlockIdentity>, ProposalError> {
    if blocklace.dom().is_empty() {
        return Ok(HashSet::new());
    }

    let predecessors = select_predecessors(blocklace, bonds);
    if predecessors.is_empty() {
        return Err(ProposalError::NoPredecessorsAvailable);
    }

    let observed = validator_visible_tips(blocklace, bonds).len();
    let required = required_acknowledgements(bonds);

    if observed < required {
        return Err(ProposalError::InsufficientAcknowledgements { observed, required });
    }

    Ok(predecessors)
}

/// Build a deterministic block-content candidate from the current local view.
///
/// This helper does not decide *when* a node should propose; it only answers
/// *what* payload and predecessor set should be used once an external scheduler
/// requests a proposal.
pub fn build_block_candidate(
    blocklace: &Blocklace,
    bonds: &HashMap<NodeId, u64>,
    payload: Vec<u8>,
) -> Result<BlockContent, ProposalError> {
    let predecessors = next_block_predecessors(blocklace, bonds)?;

    Ok(BlockContent {
        payload,
        predecessors,
    })
}

/// Bounds on how many blocks may be held awaiting their causal history.
///
/// Without a bound, a peer can stream blocks whose predecessors never arrive and
/// grow the buffer until the node exhausts memory. The policy is expressed in
/// counts and retry passes rather than wall-clock time deliberately: consensus
/// state should stay a deterministic function of the messages received, and a
/// clock would make eviction depend on scheduling. "Age" here therefore means
/// *how many retry passes a block has failed to resolve*, not seconds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BufferPolicy {
    /// Total blocks the buffer may hold. Exceeding it evicts the oldest arrival.
    pub max_entries: usize,
    /// Blocks the buffer may hold on behalf of any single creator.
    ///
    /// This is the part that actually resists a flood. A global cap alone lets
    /// one abusive creator evict every honest block; a per-creator quota bounds
    /// the damage to that creator's own share. The creator is the right axis
    /// because blocks are signed and therefore attributable — the sending peer
    /// is not visible at this layer.
    pub max_entries_per_creator: usize,
    /// Retry passes a block may fail to resolve before it is evicted as stale.
    pub max_retry_passes: u32,
}

impl Default for BufferPolicy {
    fn default() -> Self {
        Self {
            max_entries: 4096,
            max_entries_per_creator: 256,
            max_retry_passes: 64,
        }
    }
}

/// Why a block was or was not admitted to the buffer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BufferOutcome {
    /// The block is now buffered.
    Buffered,
    /// An entry for this identity already existed; the buffer did not grow.
    AlreadyBuffered,
    /// The block was buffered, evicting the oldest entry to respect
    /// `max_entries`.
    BufferedEvicting(BlockIdentity),
    /// Refused: this creator already holds `max_entries_per_creator` slots.
    RejectedCreatorQuota { creator: NodeId },
}

/// Observability counters for buffer pressure.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct BufferStats {
    /// Blocks currently buffered.
    pub buffered: usize,
    /// Blocks evicted to respect `max_entries`.
    pub evicted_for_capacity: u64,
    /// Blocks evicted after exceeding `max_retry_passes`.
    pub evicted_stale: u64,
    /// Blocks refused because their creator was at quota.
    pub rejected_creator_quota: u64,
}

/// A buffer for blocks that arrive out of order (before their predecessors).
///
/// This provides the dependency-resolution side of dissemination: blocks with missing
/// parents should be buffered and retried once dependencies arrive.
///
/// Admission is bounded by [`BufferPolicy`]. Note that `buffered_blocks` is
/// public for historical reasons: writing to it directly bypasses the policy and
/// the bookkeeping below, so prefer
/// [`Self::buffer_block_with_missing_predecessors`].
#[derive(Default, Debug, Clone)]
pub struct PendingBlockBuffer {
    /// Blocks that are buffered, indexed by their identity.
    pub buffered_blocks: HashMap<BlockIdentity, Block>,
    policy: BufferPolicy,
    /// Arrival order, used to choose an eviction victim deterministically.
    arrival: HashMap<BlockIdentity, u64>,
    /// Retry passes each block has failed to resolve.
    retry_passes: HashMap<BlockIdentity, u32>,
    next_arrival: u64,
    stats: BufferStats,
}

fn should_keep_buffered_after_validation(errors: &[InvalidBlock]) -> bool {
    errors.iter().all(is_retryable_buffer_error)
}

fn is_retryable_buffer_error(error: &InvalidBlock) -> bool {
    matches!(error, InvalidBlock::MissingPredecessors { .. })
}

impl PendingBlockBuffer {
    /// Create a new empty pending block buffer with the default policy.
    pub fn new() -> Self {
        Self::with_policy(BufferPolicy::default())
    }

    /// Create a new empty pending block buffer with an explicit policy.
    pub fn with_policy(policy: BufferPolicy) -> Self {
        Self {
            buffered_blocks: HashMap::new(),
            policy,
            arrival: HashMap::new(),
            retry_passes: HashMap::new(),
            next_arrival: 0,
            stats: BufferStats::default(),
        }
    }

    pub fn policy(&self) -> BufferPolicy {
        self.policy
    }

    /// Current buffer pressure counters.
    pub fn stats(&self) -> BufferStats {
        BufferStats {
            buffered: self.buffered_blocks.len(),
            ..self.stats
        }
    }

    fn entries_by_creator(&self, creator: &NodeId) -> usize {
        self.buffered_blocks
            .keys()
            .filter(|id| &id.creator == creator)
            .count()
    }

    /// Identity of the oldest-arrived entry, or `None` when the buffer is empty.
    ///
    /// Ties on arrival order cannot happen, since `next_arrival` is strictly
    /// increasing; the identity comparison is a defensive tie-break for entries
    /// inserted directly into the public map, which carry no arrival record.
    fn oldest_entry(&self) -> Option<BlockIdentity> {
        self.buffered_blocks
            .keys()
            .min_by_key(|id| (self.arrival.get(*id).copied().unwrap_or(0), (*id).clone()))
            .cloned()
    }

    fn forget(&mut self, id: &BlockIdentity) {
        self.buffered_blocks.remove(id);
        self.arrival.remove(id);
        self.retry_passes.remove(id);
    }

    /// Add a block to the buffer that might be missing predecessors.
    ///
    /// Admission respects [`BufferPolicy`]: a creator over its quota is refused,
    /// and a full buffer evicts its oldest entry to make room. The return value
    /// says which happened; callers that do not care may ignore it.
    pub fn buffer_block_with_missing_predecessors(&mut self, block: Block) -> BufferOutcome {
        let id = block.identity.clone();

        // Re-arrival of something already held: refresh the block but neither
        // grow the buffer nor let a repeat consume extra quota.
        if let std::collections::hash_map::Entry::Occupied(mut held) =
            self.buffered_blocks.entry(id.clone())
        {
            held.insert(block);
            return BufferOutcome::AlreadyBuffered;
        }

        let creator = id.creator.clone();
        if self.entries_by_creator(&creator) >= self.policy.max_entries_per_creator {
            self.stats.rejected_creator_quota += 1;
            return BufferOutcome::RejectedCreatorQuota { creator };
        }

        let mut evicted = None;
        if self.buffered_blocks.len() >= self.policy.max_entries
            && let Some(victim) = self.oldest_entry()
        {
            self.forget(&victim);
            self.stats.evicted_for_capacity += 1;
            evicted = Some(victim);
        }

        self.arrival.insert(id.clone(), self.next_arrival);
        self.next_arrival += 1;
        self.retry_passes.insert(id.clone(), 0);
        self.buffered_blocks.insert(id, block);

        match evicted {
            Some(victim) => BufferOutcome::BufferedEvicting(victim),
            None => BufferOutcome::Buffered,
        }
    }

    /// Drop entries that have failed to resolve for more than
    /// `max_retry_passes`, and discard bookkeeping for entries that are no
    /// longer present.
    fn evict_stale(&mut self) {
        let stale: Vec<BlockIdentity> = self
            .buffered_blocks
            .keys()
            .filter(|id| {
                self.retry_passes.get(*id).copied().unwrap_or(0) > self.policy.max_retry_passes
            })
            .cloned()
            .collect();

        for id in stale {
            self.forget(&id);
            self.stats.evicted_stale += 1;
        }

        // Entries inserted or removed through the public map leave orphaned
        // bookkeeping; drop it so the maps cannot grow unbounded either.
        self.arrival
            .retain(|id, _| self.buffered_blocks.contains_key(id));
        self.retry_passes
            .retain(|id, _| self.buffered_blocks.contains_key(id));
    }

    /// Retry inserting buffered blocks into the given blocklace.
    ///
    /// Loops through buffered blocks and attempts to insert them if their
    /// predecessors are now available. Continues as long as progress is made
    /// (e.g., a block is inserted which then satisfies another block's dependencies).
    ///
    /// Buffered replay uses the same consensus validation path as normal
    /// ingestion via `validated_insert`, so dependency resolution does not
    /// weaken admission rules.
    ///
    /// Successfully inserted blocks, or blocks that are definitively rejected
    /// by validation, are removed from the buffer. Retryability is classified
    /// explicitly by validation error kind; with the current validation model,
    /// only `MissingPredecessors` is considered retryable for the supplied
    /// `bonds` and `config`.
    ///
    /// Returns every definitively rejected block together with its validation
    /// errors. A block that was buffered as `MissingPredecessors` can turn out
    /// to be an equivocation once its history arrives, and this replay is the
    /// only place that rejection happens — callers that retain equivocation
    /// proof (see `record_rejected_equivocation`) must be handed the rejected
    /// block *here*, before it is dropped, or the evidence is lost.
    pub fn retry_buffered_blocks(
        &mut self,
        blocklace: &mut Blocklace,
        bonds: &HashMap<NodeId, u64>,
        config: &ValidationConfig,
    ) -> Vec<(Block, Vec<InvalidBlock>)> {
        let mut rejected = Vec::new();
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
                    match validated_insert(block.clone(), blocklace, bonds, config) {
                        crate::consensus::validation::ValidationResult::Valid => {
                            resolved.push(id.clone());
                            progress = true;
                        }
                        crate::consensus::validation::ValidationResult::Invalid(errors) => {
                            if !should_keep_buffered_after_validation(&errors) {
                                resolved.push(id.clone());
                                rejected.push((block.clone(), errors));
                            }
                        }
                    }
                }
            }

            for id in resolved {
                self.forget(&id);
            }
        }

        // Everything still held has failed to resolve for one more pass. Count
        // it, then evict whatever has exceeded the policy — otherwise a block
        // whose predecessors never arrive occupies its slot forever.
        let remaining: Vec<BlockIdentity> = self.buffered_blocks.keys().cloned().collect();
        for id in remaining {
            *self.retry_passes.entry(id).or_insert(0) += 1;
        }
        self.evict_stale();
    }
}
