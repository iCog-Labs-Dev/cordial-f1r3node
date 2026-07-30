//! Adversarial network simulation for Cordial Miners.
//!
//! [`AdversarialNetwork`] extends the cooperative harness in
//! [`crate::simulation::dissemination`] with the fault behaviours a production
//! deployment has to survive:
//!
//! - delayed block delivery
//! - reverse / out-of-order delivery
//! - validator equivocation
//! - temporary network partitions and healing
//!
//! The scheduler is deterministic. Delivery is driven by an explicit step
//! counter and, for randomised schedules, a seeded generator — so a failing
//! run is reproduced exactly by re-running with the same seed.
//!
//! The model deliberately keeps the *adversary* outside the protocol: honest
//! nodes only ever call [`SimNode::receive_block`], and every fault is
//! expressed as a scheduling decision (hold, reorder, sever) or as a Byzantine
//! block-production choice (equivocate). Nothing here weakens validation.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use crate::consensus::round::depth;
use crate::consensus::wave::wave_of_round;
use crate::consensus::{OrderingError, ValidationConfig};
use crate::simulation::dissemination::{DeliveryOutcome, SimNode};
use crate::types::{BlockContent, BlockIdentity, NodeId};
use crate::{Block, Blocklace};

/// Maximum number of Byzantine validators tolerated in eventual-synchrony (ES)
/// mode, for a validator set of size `n`.
///
/// ES quorums in this implementation are the supermajorities of
/// [`crate::consensus::is_supermajority`]: a block set whose distinct creators
/// number strictly more than `(n + f) / 2`. Writing `q` for that quorum size:
///
/// - **Safety** needs any two quorums to share an honest validator. Two
///   quorums overlap in at least `2q - n > f` validators, so the overlap
///   always contains someone honest — this holds for every `f < n`.
/// - **Liveness** needs a quorum to be reachable from honest validators alone,
///   i.e. `q <= n - f`. With `q > (n + f) / 2` that requires
///   `(n + f) / 2 < n - f`, which simplifies to `3f < n`.
///
/// Liveness is therefore the binding constraint, giving the ES bound
/// `f < n / 3`, i.e. `f <= (n - 1) / 3`.
pub fn max_byzantine_faults(n: usize) -> usize {
    n.saturating_sub(1) / 3
}

/// Whether `f` Byzantine validators are within the ES-mode fault bound for a
/// validator set of size `n`.
///
/// See [`max_byzantine_faults`] for the derivation of `3f < n`.
pub fn within_es_fault_bound(n: usize, f: usize) -> bool {
    3 * f < n
}

/// The ES quorum size for `n` validators tolerating `f` faults: the smallest
/// number of distinct creators that forms a supermajority.
///
/// This mirrors [`crate::consensus::is_supermajority`], which tests
/// `distinct_creators > (n + f) / 2` using integer division.
pub fn es_quorum_size(n: usize, f: usize) -> usize {
    (n + f) / 2 + 1
}

/// Deterministic xorshift64\* generator.
///
/// Used only to permute delivery order. A local generator keeps adversarial
/// schedules reproducible without adding a dependency on `rand`'s
/// distributions, whose sampling is not guaranteed stable across versions.
#[derive(Debug, Clone)]
pub struct Rng {
    state: u64,
}

impl Rng {
    pub fn new(seed: u64) -> Self {
        // Spread low-entropy seeds (0, 1, 2, ...) across the state space, and
        // never allow the all-zero state, which is a fixed point of xorshift.
        let state = seed.wrapping_mul(0x2545_F491_4F6C_DD1D) | 1;
        Self { state }
    }

    fn next_u64(&mut self) -> u64 {
        let mut x = self.state;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.state = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// Uniform-ish index in `0..n`. Returns 0 when `n == 0`.
    fn below(&mut self, n: usize) -> usize {
        if n == 0 {
            0
        } else {
            (self.next_u64() % n as u64) as usize
        }
    }
}

/// Builds blocks for simulations, including equivocating pairs.
///
/// Content hashes are synthetic: simulations run with `check_content_hash` and
/// `check_signature` disabled, so the only requirement is that distinct blocks
/// get distinct identities.
#[derive(Debug, Default)]
pub struct BlockFactory {
    next_tag: u32,
}

impl BlockFactory {
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a block by `creator` over `predecessors`.
    pub fn block(&mut self, creator: &NodeId, predecessors: HashSet<BlockIdentity>) -> Block {
        let tag = self.next_tag;
        self.next_tag += 1;

        let mut content_hash = [0u8; 32];
        content_hash[..4].copy_from_slice(&tag.to_be_bytes());
        content_hash[4] = creator.0.first().copied().unwrap_or(0);

        Block {
            identity: BlockIdentity {
                content_hash,
                creator: creator.clone(),
                signature: tag.to_be_bytes().to_vec(),
            },
            content: BlockContent {
                payload: tag.to_be_bytes().to_vec(),
                predecessors,
            },
        }
    }

    /// Create two distinct blocks by the same creator over the same
    /// predecessors — a same-round equivocation.
    ///
    /// The two blocks land at the same depth and are mutually incomparable, so
    /// a blocklace holding both violates the chain axiom for `creator`.
    pub fn equivocating_pair(
        &mut self,
        creator: &NodeId,
        predecessors: HashSet<BlockIdentity>,
    ) -> (Block, Block) {
        let left = self.block(creator, predecessors.clone());
        let right = self.block(creator, predecessors);
        (left, right)
    }
}

/// How the scheduler orders the messages that are eligible for delivery.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeliveryOrder {
    /// Deliver in the order the adversary queued them.
    Fifo,
    /// Deliver most-recently-queued first — the worst case for causal order,
    /// since children consistently arrive before their parents.
    Reverse,
    /// Deliver in a seeded pseudo-random permutation.
    Shuffled,
}

#[derive(Debug, Clone)]
struct InFlight {
    recipient: NodeId,
    block: Block,
    /// Step at which this message becomes eligible for delivery.
    release_step: u64,
    /// Monotonic queue position, for stable tie-breaking.
    seq: u64,
}

/// Outcome of a safety check.
///
/// The violation is boxed: it carries block identities, so an unboxed
/// `Result` would make the success path pay for the failure path — which
/// matters here because these checks run after every adversarial step.
pub type SafetyResult = Result<(), Box<SafetyViolation>>;

/// A safety invariant that the simulated run broke.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SafetyViolation {
    /// Two nodes committed different blocks at the same position in the total
    /// order. This is the property the protocol may never violate, however
    /// adversarial the schedule.
    Divergence {
        left: NodeId,
        right: NodeId,
        index: usize,
    },
    /// A node emitted the same block twice in its ordered output.
    DuplicateCommit { node: NodeId, block: BlockIdentity },
    /// A node ordered a block before one of its causal predecessors.
    CausalityViolation {
        node: NodeId,
        block: BlockIdentity,
        predecessor: BlockIdentity,
    },
    /// Two nodes finalised different leader blocks for the same wave.
    ConflictingFinalLeaders {
        left: NodeId,
        right: NodeId,
        left_leader: BlockIdentity,
        right_leader: BlockIdentity,
    },
    /// A node's ordered output could not be computed at all.
    Ordering { node: NodeId, error: OrderingError },
}

impl std::fmt::Display for SafetyViolation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Divergence { left, right, index } => write!(
                f,
                "ordered outputs of {left:?} and {right:?} diverge at index {index}"
            ),
            Self::DuplicateCommit { node, block } => {
                write!(f, "{node:?} ordered {block:?} more than once")
            }
            Self::CausalityViolation {
                node,
                block,
                predecessor,
            } => write!(
                f,
                "{node:?} ordered {block:?} before its predecessor {predecessor:?}"
            ),
            Self::ConflictingFinalLeaders {
                left,
                right,
                left_leader,
                right_leader,
            } => write!(
                f,
                "{left:?} finalised leader {left_leader:?} but {right:?} finalised {right_leader:?}"
            ),
            Self::Ordering { node, error } => {
                write!(f, "{node:?} failed to compute an ordered output: {error:?}")
            }
        }
    }
}

/// Consensus parameters shared by every observation made of a run.
#[derive(Debug, Clone, Copy)]
pub struct ConsensusParams {
    pub wavelength: u64,
    pub n: usize,
    pub f: usize,
}

impl ConsensusParams {
    /// Parameters for `n` validators at the maximum ES-tolerable fault count.
    pub fn at_es_bound(n: usize, wavelength: u64) -> Self {
        Self {
            wavelength,
            n,
            f: max_byzantine_faults(n),
        }
    }

    /// Whether these parameters sit inside the ES-mode fault bound.
    pub fn within_es_bound(&self) -> bool {
        within_es_fault_bound(self.n, self.f)
    }
}

/// A network of simulated validators under adversarial scheduling.
///
/// Every validator is both a block creator and an observer, so partitions and
/// delays act on the same identities that appear in quorum arithmetic.
pub struct AdversarialNetwork {
    nodes: BTreeMap<NodeId, SimNode>,
    validators: Vec<NodeId>,
    bonds: HashMap<NodeId, u64>,
    /// Messages that will be delivered, once their release step arrives.
    inflight: Vec<InFlight>,
    /// Messages held back by an active partition. Released by [`Self::heal`].
    severed: Vec<InFlight>,
    partition: Option<Vec<BTreeSet<NodeId>>>,
    delays: BTreeMap<NodeId, u64>,
    order: DeliveryOrder,
    step: u64,
    seq: u64,
    rng: Rng,
    seed: u64,
}

impl AdversarialNetwork {
    /// Build a network where each validator observes the blocklace directly.
    pub fn new(
        validators: Vec<NodeId>,
        bonds: HashMap<NodeId, u64>,
        config: ValidationConfig,
    ) -> Self {
        let nodes: BTreeMap<NodeId, SimNode> = validators
            .iter()
            .map(|id| {
                (
                    id.clone(),
                    SimNode::new(id.clone(), bonds.clone(), config.clone()),
                )
            })
            .collect();

        Self {
            nodes,
            validators,
            bonds,
            inflight: Vec::new(),
            severed: Vec::new(),
            partition: None,
            delays: BTreeMap::new(),
            order: DeliveryOrder::Fifo,
            step: 0,
            seq: 0,
            rng: Rng::new(0),
            seed: 0,
        }
    }

    /// Equal-stake network over validators `1..=n`, with the validation config
    /// simulations use (synthetic hashes and signatures are not checked).
    pub fn equal_stake(n: usize) -> Self {
        let validators: Vec<NodeId> = (1..=n).map(|i| NodeId(vec![i as u8])).collect();
        let bonds: HashMap<NodeId, u64> = validators.iter().map(|id| (id.clone(), 100)).collect();
        Self::new(validators, bonds, simulation_validation_config())
    }

    /// Network with explicit per-validator stake.
    ///
    /// The equal-stake constructor cannot exercise the weighted quorum path in
    /// any meaningful way: when every validator has the same stake, a stake
    /// supermajority and a count supermajority are the same set, so a bug that
    /// counted validators instead of weighing them would pass unnoticed. Skewed
    /// stake is what separates the two.
    pub fn weighted(weights: &[(NodeId, u64)]) -> Self {
        let validators: Vec<NodeId> = weights.iter().map(|(id, _)| id.clone()).collect();
        let bonds: HashMap<NodeId, u64> = weights.iter().cloned().collect();
        Self::new(validators, bonds, simulation_validation_config())
    }

    /// Bonded stake per validator.
    pub fn bonds(&self) -> &HashMap<NodeId, u64> {
        &self.bonds
    }

    /// Total bonded stake.
    pub fn total_stake(&self) -> u64 {
        self.bonds.values().copied().sum()
    }

    /// Stake held by `id`, or zero if it is not bonded.
    pub fn stake_of(&self, id: &NodeId) -> u64 {
        self.bonds.get(id).copied().unwrap_or(0)
    }

    /// Whether `validators` together hold a strict two-thirds stake majority.
    ///
    /// Mirrors [`crate::consensus::is_weighted_supermajority`] so tests can
    /// state the stake arithmetic they rely on instead of assuming it.
    pub fn is_stake_supermajority(&self, validators: &[NodeId]) -> bool {
        let support: u64 = validators.iter().map(|id| self.stake_of(id)).sum();
        (support as u128) * 3 > (self.total_stake() as u128) * 2
    }

    pub fn with_seed(mut self, seed: u64) -> Self {
        self.seed = seed;
        self.rng = Rng::new(seed);
        self
    }

    pub fn with_delivery_order(mut self, order: DeliveryOrder) -> Self {
        self.order = order;
        self
    }

    pub fn seed(&self) -> u64 {
        self.seed
    }

    pub fn step(&self) -> u64 {
        self.step
    }

    pub fn validators(&self) -> &[NodeId] {
        &self.validators
    }

    pub fn node(&self, id: &NodeId) -> Option<&SimNode> {
        self.nodes.get(id)
    }

    pub fn node_mut(&mut self, id: &NodeId) -> Option<&mut SimNode> {
        self.nodes.get_mut(id)
    }

    pub fn blocklace(&self, id: &NodeId) -> Option<&Blocklace> {
        self.nodes.get(id).map(|node| &node.blocklace)
    }

    /// Number of messages still waiting to be delivered, including those held
    /// back by a partition.
    pub fn inflight_len(&self) -> usize {
        self.inflight.len() + self.severed.len()
    }

    // ---- adversary controls -------------------------------------------------

    /// Sever the network into `groups`. Blocks only flow between validators in
    /// the same group; everything else is held until [`Self::heal`].
    ///
    /// A validator absent from every group is treated as unpartitioned, i.e.
    /// still reachable from all others.
    pub fn partition(&mut self, groups: Vec<Vec<NodeId>>) {
        self.partition = Some(
            groups
                .into_iter()
                .map(|group| group.into_iter().collect())
                .collect(),
        );
    }

    /// Heal the partition and release every message it held back.
    ///
    /// Held messages become eligible immediately: this models the catch-up
    /// that follows a partition, not a second delay.
    pub fn heal(&mut self) {
        self.partition = None;
        let step = self.step;
        for mut msg in self.severed.drain(..) {
            msg.release_step = step;
            self.inflight.push(msg);
        }
    }

    pub fn is_partitioned(&self) -> bool {
        self.partition.is_some()
    }

    /// Delay everything subsequently sent to `id` by `steps`.
    pub fn delay_node(&mut self, id: &NodeId, steps: u64) {
        self.delays.insert(id.clone(), steps);
    }

    pub fn clear_delays(&mut self) {
        self.delays.clear();
    }

    fn connected(&self, a: &NodeId, b: &NodeId) -> bool {
        let Some(groups) = &self.partition else {
            return true;
        };

        let group_of = |id: &NodeId| groups.iter().position(|group| group.contains(id));
        match (group_of(a), group_of(b)) {
            (Some(x), Some(y)) => x == y,
            _ => true,
        }
    }

    // ---- traffic ------------------------------------------------------------

    fn next_seq(&mut self) -> u64 {
        let seq = self.seq;
        self.seq += 1;
        seq
    }

    /// Send `block` from its creator to every validator, subject to the active
    /// partition and per-recipient delays.
    ///
    /// The creator always receives its own block: a validator that produced a
    /// block has it locally regardless of the network.
    pub fn broadcast(&mut self, block: &Block) {
        let creator = block.identity.creator.clone();
        for recipient in self.validators.clone() {
            let delay = self.delays.get(&recipient).copied().unwrap_or(0);
            let seq = self.next_seq();
            let msg = InFlight {
                release_step: self.step + delay,
                recipient: recipient.clone(),
                block: block.clone(),
                seq,
            };

            if recipient == creator || self.connected(&creator, &recipient) {
                self.inflight.push(msg);
            } else {
                self.severed.push(msg);
            }
        }
    }

    /// Send `block` only to `recipients`, ignoring partitions and delays.
    ///
    /// This is the Byzantine primitive: it lets an equivocating validator show
    /// one branch to one part of the network and a conflicting branch to
    /// another, which no honest broadcast would ever do.
    pub fn send_to(&mut self, block: &Block, recipients: &[NodeId]) {
        for recipient in recipients {
            let seq = self.next_seq();
            self.inflight.push(InFlight {
                recipient: recipient.clone(),
                block: block.clone(),
                release_step: self.step,
                seq,
            });
        }
    }

    /// Deliver every message whose release step has arrived, in the configured
    /// order, and return what each recipient did with it.
    pub fn deliver_ready(&mut self) -> Vec<(NodeId, DeliveryOutcome)> {
        let step = self.step;
        let mut eligible = Vec::new();
        let mut held = Vec::new();
        for msg in self.inflight.drain(..) {
            if msg.release_step <= step {
                eligible.push(msg);
            } else {
                held.push(msg);
            }
        }
        self.inflight = held;

        eligible.sort_by_key(|msg| msg.seq);
        match self.order {
            DeliveryOrder::Fifo => {}
            DeliveryOrder::Reverse => eligible.reverse(),
            DeliveryOrder::Shuffled => {
                for i in (1..eligible.len()).rev() {
                    let j = self.rng.below(i + 1);
                    eligible.swap(i, j);
                }
            }
        }

        let mut outcomes = Vec::with_capacity(eligible.len());
        for msg in eligible {
            if let Some(node) = self.nodes.get_mut(&msg.recipient) {
                let outcome = node.receive_block(msg.block);
                outcomes.push((msg.recipient, outcome));
            }
        }
        outcomes
    }

    /// Retry every node's buffered out-of-order blocks against its current view.
    pub fn retry_all_buffers(&mut self) {
        for node in self.nodes.values_mut() {
            node.retry_buffered_blocks();
        }
    }

    pub fn advance(&mut self, steps: u64) {
        self.step += steps;
    }

    /// Deliver everything currently eligible and resolve buffers, repeating
    /// until nothing more can be delivered at the current step.
    ///
    /// Messages still held by a delay or a partition are left alone.
    pub fn settle(&mut self) -> usize {
        let mut delivered = 0;
        loop {
            let batch = self.deliver_ready().len();
            self.retry_all_buffers();
            if batch == 0 {
                return delivered;
            }
            delivered += batch;
        }
    }

    /// Resume delivery in full: jump the clock past every outstanding delay and
    /// deliver all in-flight messages.
    ///
    /// Messages held by an *active* partition stay held — call [`Self::heal`]
    /// first to model a partition that actually recovers.
    pub fn deliver_everything(&mut self) -> usize {
        let latest = self
            .inflight
            .iter()
            .map(|msg| msg.release_step)
            .max()
            .unwrap_or(self.step);
        self.step = self.step.max(latest);
        self.settle()
    }

    // ---- observation --------------------------------------------------------

    /// The ordered output (τ) of a single node.
    pub fn ordered_output<F>(
        &self,
        id: &NodeId,
        params: ConsensusParams,
        leader_selection: F,
    ) -> Result<Vec<BlockIdentity>, OrderingError>
    where
        F: Fn(u64) -> Option<NodeId> + Copy,
    {
        match self.nodes.get(id) {
            Some(node) => {
                node.ordered_output(params.wavelength, params.n, params.f, leader_selection)
            }
            None => Ok(Vec::new()),
        }
    }

    /// Ordered outputs for every validator, keyed by node.
    pub fn all_ordered_outputs<F>(
        &self,
        params: ConsensusParams,
        leader_selection: F,
    ) -> BTreeMap<NodeId, Vec<BlockIdentity>>
    where
        F: Fn(u64) -> Option<NodeId> + Copy,
    {
        self.nodes
            .iter()
            .map(|(id, node)| {
                let output = node
                    .ordered_output(params.wavelength, params.n, params.f, leader_selection)
                    .unwrap_or_default();
                (id.clone(), output)
            })
            .collect()
    }

    /// Latest final leader as seen by every validator.
    pub fn all_final_leaders<F>(
        &self,
        params: ConsensusParams,
        leader_selection: F,
    ) -> BTreeMap<NodeId, Option<BlockIdentity>>
    where
        F: Fn(u64) -> Option<NodeId> + Copy,
    {
        self.nodes
            .iter()
            .map(|(id, node)| {
                (
                    id.clone(),
                    node.latest_final_leader(
                        params.wavelength,
                        params.n,
                        params.f,
                        leader_selection,
                    ),
                )
            })
            .collect()
    }

    /// Latest final leader per validator, paired with the wave it leads.
    ///
    /// The wave is resolved against the observing node's own blocklace, which
    /// is the only view that node has.
    pub fn all_final_leader_waves<F>(
        &self,
        params: ConsensusParams,
        leader_selection: F,
    ) -> BTreeMap<NodeId, Option<(u64, BlockIdentity)>>
    where
        F: Fn(u64) -> Option<NodeId> + Copy,
    {
        self.nodes
            .iter()
            .map(|(id, node)| {
                let decided = node
                    .latest_final_leader(params.wavelength, params.n, params.f, leader_selection)
                    .and_then(|leader| {
                        let round = depth(&node.blocklace, &leader)?;
                        let wave = wave_of_round(round, params.wavelength)?;
                        Some((wave, leader))
                    });
                (id.clone(), decided)
            })
            .collect()
    }

    /// Weighted ordered output (τ) for every validator.
    pub fn all_weighted_ordered_outputs<F>(
        &self,
        wavelength: u64,
        leader_selection: F,
    ) -> BTreeMap<NodeId, Vec<BlockIdentity>>
    where
        F: Fn(u64) -> Option<NodeId> + Copy,
    {
        self.nodes
            .iter()
            .map(|(id, node)| {
                let output = node
                    .weighted_ordered_output(wavelength, leader_selection)
                    .unwrap_or_default();
                (id.clone(), output)
            })
            .collect()
    }

    /// Latest weighted final leader per validator, paired with its wave.
    pub fn all_weighted_final_leader_waves<F>(
        &self,
        wavelength: u64,
        leader_selection: F,
    ) -> BTreeMap<NodeId, Option<(u64, BlockIdentity)>>
    where
        F: Fn(u64) -> Option<NodeId> + Copy,
    {
        self.nodes
            .iter()
            .map(|(id, node)| {
                let decided = node
                    .latest_weighted_final_leader(wavelength, leader_selection)
                    .and_then(|leader| {
                        let round = depth(&node.blocklace, &leader)?;
                        let wave = wave_of_round(round, wavelength)?;
                        Some((wave, leader))
                    });
                (id.clone(), decided)
            })
            .collect()
    }

    /// Safety invariants over the weighted ordering path.
    ///
    /// Same four properties as [`Self::check_safety`], measured against
    /// `weighted_tau` and `latest_weighted_final_leader` instead of their
    /// count-based counterparts.
    pub fn check_weighted_safety<F>(&self, wavelength: u64, leader_selection: F) -> SafetyResult
    where
        F: Fn(u64) -> Option<NodeId> + Copy,
    {
        let mut outputs = BTreeMap::new();
        for (id, node) in &self.nodes {
            let output = node
                .weighted_ordered_output(wavelength, leader_selection)
                .map_err(|error| {
                    Box::new(SafetyViolation::Ordering {
                        node: id.clone(),
                        error,
                    })
                })?;
            check_no_duplicates(id, &output)?;
            check_causal_closure(id, &node.blocklace, &output)?;
            outputs.insert(id.clone(), output);
        }

        let leaders = self.all_weighted_final_leader_waves(wavelength, leader_selection);
        check_leader_agreement(&leaders)?;
        check_prefix_consistency(&outputs)
    }

    /// Whether every validator produced the same non-empty weighted output.
    pub fn has_converged_weighted<F>(&self, wavelength: u64, leader_selection: F) -> bool
    where
        F: Fn(u64) -> Option<NodeId> + Copy,
    {
        let outputs = self.all_weighted_ordered_outputs(wavelength, leader_selection);
        let mut iter = outputs.values();
        let Some(first) = iter.next() else {
            return false;
        };
        !first.is_empty() && iter.all(|output| output == first)
    }

    /// Run every safety invariant against the current state.
    ///
    /// Safety is what must hold at *all* times, under any schedule, so this is
    /// meant to be called after every adversarial step rather than only at the
    /// end of a run:
    ///
    /// 1. no node orders a block twice
    /// 2. no node orders a block before one of its causal predecessors
    /// 3. no two nodes finalise conflicting leaders
    /// 4. no two nodes' ordered outputs diverge — the shorter is always a
    ///    prefix of the longer
    pub fn check_safety<F>(&self, params: ConsensusParams, leader_selection: F) -> SafetyResult
    where
        F: Fn(u64) -> Option<NodeId> + Copy,
    {
        let mut outputs = BTreeMap::new();
        for (id, node) in &self.nodes {
            let output = node
                .ordered_output(params.wavelength, params.n, params.f, leader_selection)
                .map_err(|error| {
                    Box::new(SafetyViolation::Ordering {
                        node: id.clone(),
                        error,
                    })
                })?;
            check_no_duplicates(id, &output)?;
            check_causal_closure(id, &node.blocklace, &output)?;
            outputs.insert(id.clone(), output);
        }

        let leaders = self.all_final_leader_waves(params, leader_selection);
        check_leader_agreement(&leaders)?;
        check_prefix_consistency(&outputs)
    }

    /// Whether every validator has produced the same non-empty ordered output.
    pub fn has_converged<F>(&self, params: ConsensusParams, leader_selection: F) -> bool
    where
        F: Fn(u64) -> Option<NodeId> + Copy,
    {
        let outputs = self.all_ordered_outputs(params, leader_selection);
        let mut iter = outputs.values();
        let Some(first) = iter.next() else {
            return false;
        };
        !first.is_empty() && iter.all(|output| output == first)
    }
}

/// The validation config simulations run with: synthetic content hashes and
/// signatures are not verified, everything structural still is.
pub fn simulation_validation_config() -> ValidationConfig {
    ValidationConfig {
        check_content_hash: false,
        check_signature: false,
        ..ValidationConfig::default()
    }
}

/// Length of the longest common prefix of two ordered outputs.
pub fn common_prefix_len(left: &[BlockIdentity], right: &[BlockIdentity]) -> usize {
    left.iter()
        .zip(right.iter())
        .take_while(|(a, b)| a == b)
        .count()
}

/// Check that the shorter of every pair of outputs is a prefix of the longer.
pub fn check_prefix_consistency(outputs: &BTreeMap<NodeId, Vec<BlockIdentity>>) -> SafetyResult {
    let entries: Vec<(&NodeId, &Vec<BlockIdentity>)> = outputs.iter().collect();

    for i in 0..entries.len() {
        for j in (i + 1)..entries.len() {
            let (left_id, left) = entries[i];
            let (right_id, right) = entries[j];

            let shared = common_prefix_len(left, right);
            if shared < left.len().min(right.len()) {
                return Err(Box::new(SafetyViolation::Divergence {
                    left: left_id.clone(),
                    right: right_id.clone(),
                    index: shared,
                }));
            }
        }
    }

    Ok(())
}

/// Check that a node ordered no block more than once.
pub fn check_no_duplicates(node: &NodeId, output: &[BlockIdentity]) -> SafetyResult {
    let mut seen = HashSet::with_capacity(output.len());
    for id in output {
        if !seen.insert(id.clone()) {
            return Err(Box::new(SafetyViolation::DuplicateCommit {
                node: node.clone(),
                block: id.clone(),
            }));
        }
    }
    Ok(())
}

/// Check that every ordered block appears after all of its predecessors that
/// are themselves ordered.
///
/// A predecessor outside the output is not a violation: τ orders the causal
/// history of final leaders, and a block can legitimately reference something
/// no final leader has approved yet.
pub fn check_causal_closure(
    node: &NodeId,
    blocklace: &Blocklace,
    output: &[BlockIdentity],
) -> SafetyResult {
    let positions: HashMap<&BlockIdentity, usize> = output
        .iter()
        .enumerate()
        .map(|(index, id)| (id, index))
        .collect();

    for (index, id) in output.iter().enumerate() {
        let Some(content) = blocklace.content(id) else {
            continue;
        };

        for predecessor in &content.predecessors {
            if let Some(&predecessor_index) = positions.get(predecessor)
                && predecessor_index > index
            {
                return Err(Box::new(SafetyViolation::CausalityViolation {
                    node: node.clone(),
                    block: id.clone(),
                    predecessor: predecessor.clone(),
                }));
            }
        }
    }

    Ok(())
}

/// Check that no two nodes finalised different leader blocks for the same wave.
///
/// Each entry is a node's latest final leader together with the wave it belongs
/// to. Nodes lagging behind may sit on an *earlier* wave's leader — that is a
/// liveness difference, not a safety one. The violation is two nodes finalising
/// different blocks for the *same* wave, which is exactly what a successful
/// equivocation attack would produce.
pub fn check_leader_agreement(
    leaders: &BTreeMap<NodeId, Option<(u64, BlockIdentity)>>,
) -> SafetyResult {
    let mut by_wave: BTreeMap<u64, (&NodeId, &BlockIdentity)> = BTreeMap::new();

    for (id, decided) in leaders {
        let Some((wave, leader)) = decided else {
            continue;
        };

        match by_wave.get(wave) {
            Some((other_id, other_leader)) if *other_leader != leader => {
                return Err(Box::new(SafetyViolation::ConflictingFinalLeaders {
                    left: (*other_id).clone(),
                    right: id.clone(),
                    left_leader: (*other_leader).clone(),
                    right_leader: leader.clone(),
                }));
            }
            Some(_) => {}
            None => {
                by_wave.insert(*wave, (id, leader));
            }
        }
    }

    Ok(())
}
