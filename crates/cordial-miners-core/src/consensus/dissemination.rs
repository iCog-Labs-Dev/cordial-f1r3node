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
//!
//! ## Predecessor-Selection Modes
//!
//! This module exposes **two predecessor-selection behaviours** that must not be confused with
//! each other or with finality/ordering exclusion:
//!
//! | Concept | Where it happens | What it does |
//! |---|---|---|
//! | Finality / ordering exclusion | `finality.rs`, `ordering.rs` | Equivocating validators' blocks are never elected leader and never appear in the committed output. This is always active. |
//! | **Compatibility predecessor selection** | `select_predecessors` / [`PredecessorSelectionMode::Compatibility`] | An honest node re-adds any equivocator branch that is **not yet transitively visible** through the current honest tips, so the network never loses track of the equivocation evidence. This is the default and preserves inter-node compatibility. |
//! | **Strict predecessor selection** | `select_predecessors_strict` / [`PredecessorSelectionMode::Strict`] | An honest node never directly points to an equivocator branch. It relies solely on transitive acknowledgement through other honest tips. This is the paper-native excommunication behaviour (§6.1): correct miners "ignore Byzantine miners" by not including direct pointers to their blocks after detecting an equivocation. Before proposing, [`next_block_predecessors_with_mode`] verifies that the honest-tip closure already covers every known equivocation branch; if not, the proposal is rejected with [`ProposalError::InsufficientEquivocationAcknowledgement`]. |
//!
//! Choose **`Strict`** for paper-faithful proposer behaviour. Use **`Compatibility`** when the
//! adapter or snapshot path must remain interoperable with observers that may not yet have
//! received the equivocation evidence.

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

/// Controls how predecessor selection handles known equivocator branches.
///
/// This enum separates two distinct behaviours that are sometimes conflated:
///
/// * **Finality / ordering exclusion** — equivocating validators are *always* excluded from
///   leader election and the committed output, regardless of which mode is selected here.
///   That policy lives in `finality.rs` and `ordering.rs` and is independent of this enum.
///
/// * **Predecessor-selection excommunication** — whether a newly proposed block should
///   ever carry a *direct pointer* to an equivocator's branch. This is what the two
///   variants below control.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PredecessorSelectionMode {
    /// **Compatibility mode** (default).
    ///
    /// When a known equivocation branch is not yet transitively visible through the current
    /// honest tip set, it is added as a *direct predecessor* so that the network never loses
    /// track of the evidence. This preserves interoperability with observers that may not
    /// yet have received the equivocation, and is required by adapter / snapshot paths that
    /// need full DAG coverage.
    ///
    /// This is the behaviour that has always been present in `select_predecessors`.
    #[default]
    Compatibility,

    /// **Strict mode** (paper-native excommunication).
    ///
    /// From Cordial Miners §6.1: *"correct miners ignore Byzantine miners by not including
    /// direct pointers to their blocks after detecting an equivocation."*
    ///
    /// In strict mode no equivocator branch ever appears as a direct predecessor.
    /// The proposer relies solely on transitive acknowledgement through other honest tips.
    /// This is the preferred mode for proposer-side use when the network is operating in
    /// fully protocol-faithful (paper-native) configuration.
    ///
    /// **Safety invariant**: when building a proposal via [`next_block_predecessors_with_mode`]
    /// the system additionally verifies (via
    /// [`predecessors_acknowledge_all_equivocation_branches`]) that every known equivocation
    /// branch is already transitively reachable through the selected honest tips. If any branch
    /// is missing the proposal is rejected with
    /// [`ProposalError::InsufficientEquivocationAcknowledgement`], preventing the node from
    /// producing a block that would fail strict cordial validation downstream.
    Strict,
}

/// Select predecessors for a newly created block, using the specified selection mode.
///
/// This is the single entry-point for all predecessor-selection logic. The two modes
/// differ only in whether known equivocator branches are ever carried as direct
/// predecessors — see [`PredecessorSelectionMode`] for the full distinction.
///
/// **Cordiality invariant**: In either mode the returned set satisfies the cordiality
/// condition (Def. A.12, Fig. 2) when the local view contains tips from at least 2f+1
/// honest validators. In `Strict` mode this relies on those honest tips already
/// transitively observing the equivocation evidence.
///
/// **Guarantees** (both modes):
/// - All returned predecessors exist in the blocklace (closure axiom satisfied)
/// - No equivocating validator appears in the honest-tip map
/// - Deterministic: same blocklace view and mode → same predecessor set
/// - Non-empty when the blocklace has at least one honest validator with a block
/// - Empty only when the blocklace is empty or contains only equivocators
///
/// # Arguments
/// * `blocklace` - The local blocklace DAG view
/// * `bonds` - The bonded validator set and their stake weights
/// * `mode` - Whether to re-add unseen equivocator branches ([`Compatibility`]) or omit
///   them entirely ([`Strict`]).
///
/// # Returns
/// A set of block identities to be used as the block's predecessors.
pub fn select_predecessors_with_mode(
    blocklace: &Blocklace,
    bonds: &HashMap<NodeId, u64>,
    mode: PredecessorSelectionMode,
) -> HashSet<BlockIdentity> {
    // Step 1: collect only honest (non-equivocating) validator tips.
    let predecessors: HashSet<BlockIdentity> = validator_visible_tips(blocklace, bonds)
        .into_values()
        .collect();

    if predecessors.is_empty() || mode == PredecessorSelectionMode::Strict {
        // Strict mode: never reference equivocator branches directly. Return the
        // honest-tip set as-is. Transitive observation through those tips is sufficient.
        return predecessors;
    }

    // Compatibility mode: add any equivocation branch that is not yet transitively
    // reachable from the honest tips, so the network retains a direct reference to
    // the full equivocation evidence.
    let mut predecessors = predecessors;
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

/// Select predecessors for a newly created block (compatibility mode).
///
/// This is the default predecessor-selection function and preserves the original
/// behaviour: all honest validator tips are selected, and any known equivocation
/// branch not yet transitively visible is re-added as a direct predecessor.
///
/// For paper-native strict excommunication behaviour, use [`select_predecessors_strict`]
/// or call [`select_predecessors_with_mode`] with [`PredecessorSelectionMode::Strict`].
///
/// **Typical usage** (adapter / snapshot paths):
/// ```ignore
/// let predecessors = select_predecessors(&local_blocklace, &bonds);
/// let block_content = BlockContent { payload: my_operations, predecessors };
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
    select_predecessors_with_mode(blocklace, bonds, PredecessorSelectionMode::Compatibility)
}

/// Select predecessors for a newly created block (strict excommunication mode).
///
/// In strict mode no equivocator branch ever appears as a direct predecessor.
/// The proposer relies solely on transitive acknowledgement through other honest tips,
/// which is the paper-native behaviour described in Cordial Miners §6.1.
///
/// For full compatibility with observers that may not yet hold the equivocation
/// evidence, use [`select_predecessors`] (compatibility mode) instead.
///
/// # Arguments
/// * `blocklace` - The local blocklace DAG view
/// * `bonds` - The bonded validator set and their stake weights
///
/// # Returns
/// A set of honest validator tip block identities to be used as predecessors.
/// Returns an empty set if the blocklace is empty or contains only equivocators.
pub fn select_predecessors_strict(
    blocklace: &Blocklace,
    bonds: &HashMap<NodeId, u64>,
) -> HashSet<BlockIdentity> {
    select_predecessors_with_mode(blocklace, bonds, PredecessorSelectionMode::Strict)
}

/// Select predecessors and return them as a sorted vector for deterministic ordering.
///
/// This is a convenience wrapper around [`select_predecessors`] (compatibility mode)
/// that returns results in a deterministic order, useful for logging, comparison, or
/// network transmission.
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
    select_predecessors_sorted_with_mode(blocklace, bonds, PredecessorSelectionMode::Compatibility)
}

/// Select predecessors and return them as a sorted vector, using the specified selection mode.
///
/// Sorting is by the full natural ordering of `BlockIdentity`, so ties on
/// `content_hash` are broken consistently by creator and signature as needed.
///
/// # Arguments
/// * `blocklace` - The local blocklace DAG view
/// * `bonds` - The bonded validator set and their stake weights
/// * `mode` - Predecessor selection mode ([`PredecessorSelectionMode`])
///
/// # Returns
/// A sorted vector of block identities.
pub fn select_predecessors_sorted_with_mode(
    blocklace: &Blocklace,
    bonds: &HashMap<NodeId, u64>,
    mode: PredecessorSelectionMode,
) -> Vec<BlockIdentity> {
    let mut result: Vec<BlockIdentity> = select_predecessors_with_mode(blocklace, bonds, mode)
        .into_iter()
        .collect();
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

    /// Strict mode only: the honest-tip closure does not transitively cover all
    /// known equivocation branches.
    ///
    /// In strict mode the proposer never adds equivocator branches as direct
    /// predecessors (§6.1). Before accepting the proposal the system checks that
    /// all known branches are already reachable through the selected honest tips.
    /// If any branch is missing, proposing would produce a block that hides
    /// equivocation evidence and would fail strict cordial validation downstream.
    ///
    /// The node should wait until the missing branches arrive via gossip and are
    /// transitively acknowledged by at least one honest tip before proposing again.
    InsufficientEquivocationAcknowledgement {
        /// Identities of the equivocation branches not yet covered by the tip closure.
        missing: Vec<BlockIdentity>,
    },
}

/// Return the equivocation branches that are **not** transitively reachable through `predecessors`.
///
/// Computes the union of all blocks reachable from the given predecessor set (via
/// [`Blocklace::observe`]) and then collects any equivocation branch identity not present in
/// that set. An empty result means all known equivocation evidence is already covered.
///
/// This is the core check used by [`next_block_predecessors_with_mode`] in strict mode.
fn missing_equivocation_branches(
    blocklace: &Blocklace,
    predecessors: &HashSet<BlockIdentity>,
) -> Vec<BlockIdentity> {
    // Build the transitive closure of everything already observable through the tip set.
    let observed: HashSet<BlockIdentity> = predecessors
        .iter()
        .flat_map(|pred_id| blocklace.observe(pred_id).into_iter())
        .collect();

    // Collect any equivocation branch not covered by that closure.
    let mut missing = Vec::new();
    for equivocation in all_equivocations(blocklace) {
        for branch in equivocation.blocks {
            if !observed.contains(&branch) {
                missing.push(branch);
            }
        }
    }
    missing.sort();
    missing
}

/// Check whether a candidate predecessor set transitively covers all known equivocation branches.
///
/// Returns `true` when every equivocation branch currently present in the blocklace is
/// reachable through the transitive closure of `predecessors`. Returns `false` if any branch
/// would be hidden, in which case a strict-mode proposal should be deferred.
///
/// This is exposed as a public API so callers can inspect coverage independently of
/// [`next_block_predecessors_with_mode`], e.g. to decide whether to switch modes or log a warning.
///
/// # Arguments
/// * `blocklace` - The local blocklace DAG view
/// * `predecessors` - The candidate predecessor set to check
///
/// # Returns
/// `true` if all known equivocation branches are transitively observable; `false` otherwise.
pub fn predecessors_acknowledge_all_equivocation_branches(
    blocklace: &Blocklace,
    predecessors: &HashSet<BlockIdentity>,
) -> bool {
    missing_equivocation_branches(blocklace, predecessors).is_empty()
}

/// Select the predecessor set for the next locally proposed block (compatibility mode).
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
///
/// To use strict (paper-native) excommunication behaviour, call
/// [`next_block_predecessors_with_mode`] with [`PredecessorSelectionMode::Strict`].
pub fn next_block_predecessors(
    blocklace: &Blocklace,
    bonds: &HashMap<NodeId, u64>,
) -> Result<HashSet<BlockIdentity>, ProposalError> {
    next_block_predecessors_with_mode(blocklace, bonds, PredecessorSelectionMode::Compatibility)
}

/// Select the predecessor set for the next locally proposed block, using the specified mode.
///
/// Identical to [`next_block_predecessors`] except the caller controls whether known
/// equivocator branches are ever carried as direct predecessors.
///
/// # Arguments
/// * `blocklace` - The local blocklace DAG view
/// * `bonds` - The bonded validator set and their stake weights
/// * `mode` - Predecessor selection mode ([`PredecessorSelectionMode`])
///
/// # Returns
/// The predecessor set on success, or a [`ProposalError`] when the local view is
/// insufficient to satisfy the cordiality threshold.
pub fn next_block_predecessors_with_mode(
    blocklace: &Blocklace,
    bonds: &HashMap<NodeId, u64>,
    mode: PredecessorSelectionMode,
) -> Result<HashSet<BlockIdentity>, ProposalError> {
    if blocklace.dom().is_empty() {
        return Ok(HashSet::new());
    }

    let predecessors = select_predecessors_with_mode(blocklace, bonds, mode);
    if predecessors.is_empty() {
        return Err(ProposalError::NoPredecessorsAvailable);
    }

    // Strict-mode safety check: verify that every known equivocation branch is
    // already transitively reachable through the selected honest tips. Without
    // this, a strict-mode proposer could silently produce a block that hides
    // equivocation evidence, causing it to fail strict cordial validation
    // downstream (see `hidden_equivocations`). This check runs before the
    // acknowledgement threshold so the caller gets a precise, actionable error.
    if mode == PredecessorSelectionMode::Strict {
        let missing = missing_equivocation_branches(blocklace, &predecessors);
        if !missing.is_empty() {
            return Err(ProposalError::InsufficientEquivocationAcknowledgement { missing });
        }
    }

    let observed = validator_visible_tips(blocklace, bonds).len();
    let required = required_acknowledgements(bonds);

    if observed < required {
        return Err(ProposalError::InsufficientAcknowledgements { observed, required });
    }

    Ok(predecessors)
}

/// Build a deterministic block-content candidate from the current local view (compatibility mode).
///
/// This helper does not decide *when* a node should propose; it only answers
/// *what* payload and predecessor set should be used once an external scheduler
/// requests a proposal.
///
/// Uses [`PredecessorSelectionMode::Compatibility`] — known equivocator branches that are
/// not yet transitively visible are re-added as direct predecessors. For strict
/// paper-native behaviour, use [`build_block_candidate_with_mode`] with
/// [`PredecessorSelectionMode::Strict`].
pub fn build_block_candidate(
    blocklace: &Blocklace,
    bonds: &HashMap<NodeId, u64>,
    payload: Vec<u8>,
) -> Result<BlockContent, ProposalError> {
    build_block_candidate_with_mode(
        blocklace,
        bonds,
        payload,
        PredecessorSelectionMode::Compatibility,
    )
}

/// Build a deterministic block-content candidate from the current local view.
///
/// Identical to [`build_block_candidate`] except the caller controls whether known
/// equivocator branches are ever carried as direct predecessors.
///
/// # Arguments
/// * `blocklace` - The local blocklace DAG view
/// * `bonds` - The bonded validator set and their stake weights
/// * `payload` - The raw payload bytes for the new block
/// * `mode` - Predecessor selection mode ([`PredecessorSelectionMode`])
///
/// # Returns
/// A [`BlockContent`] ready for signing, or a [`ProposalError`] if the local view is
/// insufficient.
pub fn build_block_candidate_with_mode(
    blocklace: &Blocklace,
    bonds: &HashMap<NodeId, u64>,
    payload: Vec<u8>,
    mode: PredecessorSelectionMode,
) -> Result<BlockContent, ProposalError> {
    let predecessors = next_block_predecessors_with_mode(blocklace, bonds, mode)?;

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
    /// Refused: the policy has `max_entries == 0`, so the buffer admits
    /// nothing. Without this outcome the eviction path would admit the first
    /// block anyway, because a full-but-empty buffer has no victim to evict.
    RejectedZeroCapacity,
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
        if self.policy.max_entries == 0 {
            return BufferOutcome::RejectedZeroCapacity;
        }

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
    /// Replay is deterministic: each pass visits buffered blocks in
    /// `BlockIdentity` order rather than `HashMap` iteration order, so every
    /// node that buffered the same conflicting blocks resolves them the same
    /// way. Local state stays a deterministic function of messages received.
    ///
    /// Identity order is used rather than the buffer's arrival order on
    /// purpose: arrival order is a property of the local delivery schedule,
    /// not of the message set, so two nodes receiving the same conflicting
    /// blocks in different orders would admit different branches. Arrival
    /// order is therefore reserved for eviction, where cross-node agreement
    /// does not matter.
    ///
    /// Blocks whose identity is already present in the blocklace are dropped
    /// from the buffer without revalidation.
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

            let mut pending: Vec<BlockIdentity> = self.buffered_blocks.keys().cloned().collect();
            pending.sort();

            for id in pending {
                // Already known: remove the entry without revalidating.
                if blocklace.content(&id).is_some() {
                    resolved.push(id);
                    continue;
                }

                let block = &self.buffered_blocks[&id];

                // Check if all predecessors are in the blocklace
                let ready = block
                    .content
                    .predecessors
                    .iter()
                    .all(|p| blocklace.content(p).is_some());

                if ready {
                    match validated_insert(block.clone(), blocklace, bonds, config) {
                        crate::consensus::validation::ValidationResult::Valid => {
                            resolved.push(id);
                            progress = true;
                        }
                        crate::consensus::validation::ValidationResult::Invalid(errors) => {
                            if !should_keep_buffered_after_validation(&errors) {
                                rejected.push((block.clone(), errors));
                                resolved.push(id);
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

        rejected
    }
}
