//! Live ingress scaffolding for Cordial Miners integration with f1r3node.
//!
//! This module is the adapter-side home for the "live interception" path.
//! It is intentionally narrow in scope for now: the first step is to make
//! the responsibility boundary explicit before wiring real transport traffic.
//!
//! ## Responsibility
//!
//! `live_ingress` is where runtime-facing integration should live once we
//! connect the adapter crate to real `f1r3node` message flow.
//!
//! In future steps this module will be responsible for:
//!
//! - receiving live block-bearing messages from a host node
//! - translating them through existing adapter logic
//! - feeding translated blocks into a local Cordial blocklace mirror
//! - exposing enough state for snapshot, finality, and ordering work
//!
//! ## Non-goals in this first step
//!
//! This module does **not** yet:
//!
//! - attach to a real gRPC or transport server
//! - maintain a full blocklace mirror
//! - run finality or tau ordering
//! - compare against HTTP-visible node state
//!
//! Those come in follow-up increments.

use std::collections::HashMap;
use std::time::Instant;

use cordial_miners_core::Block;
use cordial_miners_core::blocklace::Blocklace;
use cordial_miners_core::consensus::{
    OrderingCache, WeightSnapshot, depth, last_round_of_wave, wave_of_round,
};
use cordial_miners_core::crypto::CryptoVerifier;
use cordial_miners_core::types::BlockIdentity;
use cordial_miners_core::types::{BlockContent, NodeId};
use cordial_por::{PorError, ReputationState, authorized_validator_weights};

use crate::block_translation::BlockMessage;
use crate::deploy_trace::DeployTracer;
use crate::grpc_ingest::{BlocklaceAdapter, GrpcBlockMapper};
use crate::ordered_output::OrderedFinalizedOutput;
use crate::repository::{BlocklaceRepository, RSpaceBlocklaceRepository, RepoError};
use crate::shard_conf::CasperShardConf;
use crate::shared_ordered_output::{ReadOrderedOutput, SharedOrderedOutput};
use crate::snapshot::{
    CasperSnapshot, SnapshotError, build_snapshot, latest_finalized_block_id,
    ordered_block_identities_with_verified_prefix, ordered_finalized_block_hashes_with_cache,
    try_ordered_block_identities_with_cache,
};
/// High-level runtime phase for the live ingress adapter.
///
/// Keeping this explicit helps later commits avoid mixing "discovery only"
/// state with fully wired live ingestion state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LiveIngressPhase {
    /// Module exists, but no live runtime path is attached yet.
    #[default]
    Defined,
    /// A host-side ingress seam has been identified and documented.
    Traced,
    /// Live block messages are being accepted by the adapter boundary.
    Connected,
}

/// Minimal adapter-side entry point for future live interception work.
///
/// The generic `A` is the stateful adapter/runtime component that later
/// commits will use to store mirrored state, update snapshots, or expose
/// ordered output. This first increment keeps the shape intentionally small.
#[derive(Debug)]
pub enum LiveIngressError {
    Mapping(anyhow::Error),
    Adapter(anyhow::Error),
    Mirror(String),
    /// Persisting a block or the finalized cursor to the LMDB store failed.
    Persistence(RepoError),
}

impl std::fmt::Display for LiveIngressError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Mapping(err) => write!(f, "failed to map live BlockMessage: {err}"),
            Self::Adapter(err) => write!(f, "adapter rejected live block: {err}"),
            Self::Mirror(err) => write!(f, "failed to mirror live block: {err}"),
            Self::Persistence(err) => write!(f, "failed to persist live block: {err}"),
        }
    }
}

impl std::error::Error for LiveIngressError {}

/// Failure to activate PoR output as the weights for Cordial's existing
/// authorized validator set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PorWeightActivationError {
    Projection(PorError),
    MissingReputationCheckpoint { round: u64 },
    MissingFinalizedOutput { source_wave: u64 },
    InvalidWavelength,
    UnknownFinalizedAnchor,
    FinalizedWaveBehind { required: u64, available: u64 },
    ActivatedValidatorSetMismatch { configured: usize, recorded: usize },
    WouldRewriteFinalizedOutput,
    OrderingFailed(cordial_miners_core::consensus::OrderingError),
}

impl std::fmt::Display for PorWeightActivationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Projection(error) => write!(f, "cannot project PoR weights: {error}"),
            Self::MissingReputationCheckpoint { round } => {
                write!(f, "PoR reputation round {round} has no audited checkpoint")
            }
            Self::MissingFinalizedOutput { source_wave } => write!(
                f,
                "PoR weights require finalized Cordial source wave {source_wave}, but no ordered output is published"
            ),
            Self::InvalidWavelength => write!(f, "Cordial ordered output has zero wavelength"),
            Self::UnknownFinalizedAnchor => {
                write!(
                    f,
                    "Cordial ordered-output anchor is absent from the local blocklace"
                )
            }
            Self::FinalizedWaveBehind {
                required,
                available,
            } => write!(
                f,
                "PoR weights require finalized Cordial source wave {required}, but latest published wave is {available}"
            ),
            Self::WouldRewriteFinalizedOutput => write!(
                f,
                "PoR weight activation would rewrite the published Cordial finalized prefix"
            ),
            Self::OrderingFailed(error) => {
                write!(
                    f,
                    "cannot compute prospective finalized ordering: {error:?}"
                )
            }
            Self::ActivatedValidatorSetMismatch {
                configured,
                recorded,
            } => write!(
                f,
                "durably activated PoR weights contain {recorded} validators, but Cordial configured {configured}"
            ),
        }
    }
}

impl std::error::Error for PorWeightActivationError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Projection(error) => Some(error),
            _ => None,
        }
    }
}

impl From<PorError> for PorWeightActivationError {
    fn from(error: PorError) -> Self {
        Self::Projection(error)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MirrorDisposition {
    Applied,
    Buffered,
    Duplicate,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MirrorUpdate {
    pub disposition: MirrorDisposition,
    pub released_from_buffer: usize,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct AlreadyValidatedVerifier;

impl CryptoVerifier for AlreadyValidatedVerifier {
    type Error = &'static str;

    fn verify_block(
        &self,
        _content: &BlockContent,
        _signature: &[u8],
        _creator: &NodeId,
    ) -> Result<(), Self::Error> {
        Ok(())
    }
}

pub struct LiveBlocklaceMirror<V = AlreadyValidatedVerifier> {
    blocklace: Blocklace,
    pending: HashMap<BlockIdentity, Block>,
    verifier: V,
}

impl<V> LiveBlocklaceMirror<V>
where
    V: CryptoVerifier,
{
    pub fn new(verifier: V) -> Self {
        Self {
            blocklace: Blocklace::new(),
            pending: HashMap::new(),
            verifier,
        }
    }

    pub fn blocklace(&self) -> &Blocklace {
        &self.blocklace
    }

    pub fn pending(&self) -> &HashMap<BlockIdentity, Block> {
        &self.pending
    }

    fn duplicate_or_identity_conflict(&self, id: &BlockIdentity) -> Result<bool, String> {
        if self.blocklace.get(id).is_some() || self.pending.contains_key(id) {
            return Ok(true);
        }

        let conflicting = self.blocklace.conflicting_identity_variant(id).or_else(|| {
            self.pending
                .keys()
                .find(|candidate| candidate.same_consensus_identity(id))
        });
        if let Some(conflicting) = conflicting {
            return Err(format!(
                "Identity conflict: content hash and creator already belong to {conflicting:?}"
            ));
        }

        Ok(false)
    }

    pub fn ingest(&mut self, block: Block) -> Result<MirrorUpdate, String> {
        if self.duplicate_or_identity_conflict(&block.identity)? {
            return Ok(MirrorUpdate {
                disposition: MirrorDisposition::Duplicate,
                released_from_buffer: 0,
            });
        }

        if !self.predecessors_available(&block) {
            self.pending.insert(block.identity.clone(), block);
            return Ok(MirrorUpdate {
                disposition: MirrorDisposition::Buffered,
                released_from_buffer: 0,
            });
        }

        self.blocklace.insert(block, &self.verifier)?;
        let released_from_buffer = self.release_pending()?;

        Ok(MirrorUpdate {
            disposition: MirrorDisposition::Applied,
            released_from_buffer,
        })
    }

    pub fn ingest_with_trusted_boundary(&mut self, block: Block) -> Result<MirrorUpdate, String> {
        if self.duplicate_or_identity_conflict(&block.identity)? {
            return Ok(MirrorUpdate {
                disposition: MirrorDisposition::Duplicate,
                released_from_buffer: 0,
            });
        }

        self.blocklace
            .insert_with_trusted_boundary(block, &self.verifier)?;
        let released_from_buffer = self.release_pending()?;

        Ok(MirrorUpdate {
            disposition: MirrorDisposition::Applied,
            released_from_buffer,
        })
    }

    fn predecessors_available(&self, block: &Block) -> bool {
        block
            .content
            .predecessors
            .iter()
            .all(|pred_id| self.blocklace.content(pred_id).is_some())
    }

    fn release_pending(&mut self) -> Result<usize, String> {
        let mut released = 0usize;

        loop {
            let ready_ids: Vec<BlockIdentity> = self
                .pending
                .iter()
                .filter(|(_, block)| self.predecessors_available(block))
                .map(|(id, _)| id.clone())
                .collect();

            if ready_ids.is_empty() {
                break;
            }

            for id in ready_ids {
                let block = self
                    .pending
                    .remove(&id)
                    .expect("ready pending block should still exist");
                self.blocklace.insert(block, &self.verifier)?;
                released += 1;
            }
        }

        Ok(released)
    }
}

impl Default for LiveBlocklaceMirror<AlreadyValidatedVerifier> {
    fn default() -> Self {
        Self::new(AlreadyValidatedVerifier)
    }
}

pub struct LiveIngress<A> {
    phase: LiveIngressPhase,
    adapter: A,
    mapper: GrpcBlockMapper,
    mirror: LiveBlocklaceMirror,
    ordering_cache: OrderingCache,
    last_finalized_output_fingerprint: Option<FinalizedOutputFingerprint>,
    finalized_prefix_reuse: Option<FinalizedPrefixReuse>,
    ordering_diagnostics: OrderingDiagnostics,
    shared_ordered_output: SharedOrderedOutput,
    bonds: HashMap<NodeId, u64>,
    shard_conf: CasperShardConf,
    shard_id: String,
    /// Optional deploy tracer. When set, block ingestion and finalized-output
    /// computation automatically advance the deploy lifecycle state machine.
    deploy_tracer: Option<DeployTracer>,
}

/// Ordering path and cache statistics for live inspection and benchmarks.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OrderingDiagnostics {
    pub last_path: Option<&'static str>,
    pub last_fallback_reason: Option<&'static str>,
    pub cache_entries: usize,
    pub unchanged_polls: u64,
    pub verified_prefix_extensions: u64,
    pub full_recomputations: u64,
    pub full_fallbacks: u64,
    pub fallbacks_by_reason: HashMap<&'static str, u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FinalizedOutputFingerprint {
    generation: u64,
    mirrored_blocks: usize,
    wavelength: u64,
    weights: WeightSnapshot,
}

struct FinalizedPrefixReuse {
    fingerprint: FinalizedOutputFingerprint,
    anchor: BlockIdentity,
    anchor_wave_end: u64,
    observed_generation: u64,
    observed_mirrored_blocks: usize,
    /// Strict closed-DAG insertions after this wave cannot change its witness
    /// set or add ancestors to the already-computed leader fragments.
    safe_append: bool,
}

impl<A> LiveIngress<A> {
    /// Create a new live-ingress wrapper around an adapter-side component.
    pub fn new(adapter: A) -> Self {
        Self {
            phase: LiveIngressPhase::Defined,
            adapter,
            mapper: GrpcBlockMapper::new(),
            mirror: LiveBlocklaceMirror::default(),
            ordering_cache: OrderingCache::default(),
            last_finalized_output_fingerprint: None,
            finalized_prefix_reuse: None,
            ordering_diagnostics: OrderingDiagnostics::default(),
            shared_ordered_output: SharedOrderedOutput::new(),
            bonds: HashMap::new(),
            shard_conf: CasperShardConf::default(),
            shard_id: String::from("root"),
            deploy_tracer: None,
        }
    }

    /// Create a live-ingress wrapper with explicit consensus-view settings.
    pub fn with_consensus_view(
        adapter: A,
        bonds: HashMap<NodeId, u64>,
        shard_conf: CasperShardConf,
        shard_id: impl Into<String>,
    ) -> Self {
        Self {
            phase: LiveIngressPhase::Defined,
            adapter,
            mapper: GrpcBlockMapper::new(),
            mirror: LiveBlocklaceMirror::default(),
            ordering_cache: OrderingCache::default(),
            last_finalized_output_fingerprint: None,
            finalized_prefix_reuse: None,
            ordering_diagnostics: OrderingDiagnostics::default(),
            shared_ordered_output: SharedOrderedOutput::new(),
            bonds,
            shard_conf,
            shard_id: shard_id.into(),
            deploy_tracer: None,
        }
    }

    /// Create a live ingress instance pre-hydrated from a persistent block store.
    ///
    /// Called once at startup. Replays all blocks from `repo` into the in-memory
    /// mirror before accepting any new blocks from the live gRPC stream.
    ///
    /// Returns the finalized cursor read from the store (if any) so callers can
    /// log the resume point or skip already-finalized ranges.
    pub fn with_persistent_store<V>(
        adapter: A,
        bonds: HashMap<NodeId, u64>,
        shard_conf: CasperShardConf,
        shard_id: impl Into<String>,
        repo: &RSpaceBlocklaceRepository,
        verifier: &V,
    ) -> Result<(Self, Option<BlockIdentity>), RepoError>
    where
        V: CryptoVerifier,
    {
        let mut mirror = LiveBlocklaceMirror::default();
        let cursor = repo.recover_into_engine(&mut mirror.blocklace, verifier)?;

        // A store that already held blocks means this process is resuming
        // live traffic, not starting fresh — reflect that in the phase.
        let phase = if mirror.blocklace.dom().is_empty() {
            LiveIngressPhase::Defined
        } else {
            LiveIngressPhase::Connected
        };

        let ingress = Self {
            phase,
            adapter,
            mapper: GrpcBlockMapper::new(),
            mirror,
            ordering_cache: OrderingCache::default(),
            last_finalized_output_fingerprint: None,
            finalized_prefix_reuse: None,
            ordering_diagnostics: OrderingDiagnostics::default(),
            shared_ordered_output: SharedOrderedOutput::new(),
            bonds,
            shard_conf,
            shard_id: shard_id.into(),
            deploy_tracer: None,
        };

        Ok((ingress, cursor))
    }

    /// Attach a [`DeployTracer`] so that block ingestion and finalized-output
    /// computation automatically advance the deploy lifecycle state machine.
    ///
    /// Typically the same `DeployTracer` instance is shared (cloned) between
    /// the live ingress and the `LiveDeployIngress` observer so that all four
    /// lifecycle states are updated from a single object.
    pub fn with_deploy_tracer(mut self, tracer: DeployTracer) -> Self {
        self.deploy_tracer = Some(tracer);
        self
    }

    /// Return the attached deploy tracer, if any.
    pub fn deploy_tracer(&self) -> Option<&DeployTracer> {
        self.deploy_tracer.as_ref()
    }

    /// Return the current runtime phase of the live ingress component.
    pub fn phase(&self) -> LiveIngressPhase {
        self.phase
    }

    /// Mark that the host ingress seam has been traced and identified.
    pub fn mark_traced(&mut self) {
        self.phase = LiveIngressPhase::Traced;
    }

    /// Mark that live block-bearing traffic is attached to this adapter.
    pub fn mark_connected(&mut self) {
        self.phase = LiveIngressPhase::Connected;
    }

    /// Borrow the wrapped adapter/runtime component.
    pub fn adapter(&self) -> &A {
        &self.adapter
    }

    /// Mutably borrow the wrapped adapter/runtime component.
    pub fn adapter_mut(&mut self) -> &mut A {
        &mut self.adapter
    }

    /// Consume the wrapper and return the inner adapter/runtime component.
    pub fn into_inner(self) -> A {
        self.adapter
    }

    /// Return the current local blocklace mirror.
    pub fn blocklace(&self) -> &Blocklace {
        self.mirror.blocklace()
    }

    /// Return blocks that are waiting on missing predecessors.
    pub fn pending_blocks(&self) -> &HashMap<BlockIdentity, Block> {
        self.mirror.pending()
    }

    /// Return the shared read-only ordered-output container owned by this
    /// live ingress instance.
    pub fn ordered_output_reader(&self) -> &SharedOrderedOutput {
        &self.shared_ordered_output
    }

    /// Return finalized-ordering path and cache statistics.
    pub fn ordering_diagnostics(&self) -> &OrderingDiagnostics {
        &self.ordering_diagnostics
    }

    /// Persist the current finalized cursor to LMDB.
    ///
    /// Should be called after `latest_finalized_ordered_output` returns a
    /// non-empty anchor.
    pub fn persist_finalized_cursor(
        &self,
        repo: &RSpaceBlocklaceRepository,
    ) -> Result<(), RepoError> {
        if let Some(anchor) = self
            .shared_ordered_output
            .latest()
            .and_then(|output| output.anchor.as_ref())
        {
            repo.put_finalized_cursor(anchor)?;
        }
        Ok(())
    }

    /// Restore a projection that a durable activation record proves was already active.
    ///
    /// This intentionally bypasses prospective-finality checks: the projection
    /// was checked before its activation record was committed. Membership must
    /// still match exactly because PoR never owns validator selection.
    pub(crate) fn restore_activated_por_weights(
        &mut self,
        weights: &HashMap<NodeId, u64>,
    ) -> Result<(), PorWeightActivationError> {
        let same_membership = self.bonds.len() == weights.len()
            && self
                .bonds
                .keys()
                .all(|validator| weights.contains_key(validator));
        if !same_membership {
            return Err(PorWeightActivationError::ActivatedValidatorSetMismatch {
                configured: self.bonds.len(),
                recorded: weights.len(),
            });
        }

        self.bonds = weights.clone();
        self.ordering_cache = OrderingCache::default();
        self.invalidate_finalized_prefix_reuse();
        Ok(())
    }

    /// Replace only the weights of Cordial's existing authorized validators.
    ///
    /// Validator identities come from `self.bonds`; PoR cannot add or remove
    /// membership. For non-genesis reputation state, activation also proves
    /// that Cordial has already published the source finalized wave. A
    /// prospective weighted-order calculation must preserve the published
    /// finalized prefix, so a weight change cannot rewrite prior finality.
    /// Projection and safety checks finish before the live weights change.
    pub fn apply_por_weights(
        &mut self,
        state: &ReputationState,
    ) -> Result<(), PorWeightActivationError> {
        let mut authorized_validators: Vec<_> = self.bonds.keys().cloned().collect();
        authorized_validators.sort();
        let next_weights = authorized_validator_weights(state, &authorized_validators)?;

        if state.round() != 0 && state.latest_block().is_none() {
            return Err(PorWeightActivationError::MissingReputationCheckpoint {
                round: state.round(),
            });
        }

        if let Some(checkpoint) = state.latest_block() {
            let source_wave = checkpoint.header.source_finalized_wave;
            let output = self
                .shared_ordered_output
                .latest()
                .ok_or(PorWeightActivationError::MissingFinalizedOutput { source_wave })?;
            if output.wavelength == 0 {
                return Err(PorWeightActivationError::InvalidWavelength);
            }
            let anchor = output
                .anchor
                .as_ref()
                .ok_or(PorWeightActivationError::MissingFinalizedOutput { source_wave })?;
            let anchor_round = depth(self.blocklace(), anchor)
                .ok_or(PorWeightActivationError::UnknownFinalizedAnchor)?;
            let available = wave_of_round(anchor_round, output.wavelength)
                .ok_or(PorWeightActivationError::InvalidWavelength)?;
            if available < source_wave {
                return Err(PorWeightActivationError::FinalizedWaveBehind {
                    required: source_wave,
                    available,
                });
            }
        }

        if let Some(previous) = self.shared_ordered_output.latest() {
            let mut prospective_cache = OrderingCache::default();
            let (prospective, _) = try_ordered_block_identities_with_cache(
                self.blocklace(),
                &next_weights,
                previous.wavelength,
                &mut prospective_cache,
            )
            .map_err(PorWeightActivationError::OrderingFailed)?;
            let prospective: Vec<_> = prospective
                .into_iter()
                .map(|identity| identity.consensus_identity())
                .collect();
            if !prospective.starts_with(&previous.blocks) {
                return Err(PorWeightActivationError::WouldRewriteFinalizedOutput);
            }
        }

        self.bonds = next_weights;
        self.ordering_cache = OrderingCache::default();
        self.invalidate_finalized_prefix_reuse();
        Ok(())
    }

    /// Replace the bonded validator set used for finality and ordering views.
    ///
    /// This is the Cordial membership/configuration boundary. PoR integration
    /// must use [`Self::apply_por_weights`] instead.
    pub fn set_bonds(&mut self, bonds: HashMap<NodeId, u64>) {
        if self.bonds != bonds {
            self.bonds = bonds;
            self.ordering_cache = OrderingCache::default();
            self.invalidate_finalized_prefix_reuse();
        }
    }

    pub fn bonds(&self) -> &HashMap<NodeId, u64> {
        &self.bonds
    }

    /// Replace the shard config projected into live snapshots.
    pub fn set_shard_conf(&mut self, shard_conf: CasperShardConf) {
        self.shard_conf = shard_conf;
    }

    pub fn shard_conf(&self) -> &CasperShardConf {
        &self.shard_conf
    }

    /// Replace the shard identifier exposed through snapshot views.
    pub fn set_shard_id(&mut self, shard_id: impl Into<String>) {
        self.shard_id = shard_id.into();
    }

    pub fn shard_id(&self) -> &str {
        &self.shard_id
    }

    /// Build a snapshot over the current mirrored blocklace state.
    pub fn snapshot(&self) -> Result<CasperSnapshot, SnapshotError> {
        build_snapshot(
            self.blocklace(),
            &self.bonds,
            self.shard_conf.to_snapshot_conf(),
            &self.shard_id,
        )
    }

    /// Return the latest finalized block hash if the mirrored state has one.
    pub fn last_finalized_block_hash(&self) -> Result<Option<Vec<u8>>, SnapshotError> {
        Ok(latest_finalized_block_id(self.blocklace(), &self.bonds)
            .map(|id| id.content_hash.to_vec()))
    }

    /// Return the current weighted tau output as block hashes.
    pub fn ordered_finalized_blocks(&mut self) -> Result<Vec<Vec<u8>>, SnapshotError> {
        let blocklace = self.mirror.blocklace();
        let bonds = &self.bonds;
        let cache = &mut self.ordering_cache;
        Ok(ordered_finalized_block_hashes_with_cache(
            blocklace, bonds, cache,
        ))
    }

    /// Return the latest finalized ordered output through the stable
    /// `ordered_output` export seam: the finalized-prefix blocks
    /// (signature-independent [`BlockIdentity`] references, not bare hashes),
    /// linearized via weighted tau ordering, together with the anchor and
    /// consensus metadata needed to interpret them.
    ///
    /// `anchor` is `None` and `blocks` is empty when the mirrored state does
    /// not yet have a finalized leader.
    ///
    /// This is the boundary inspection tooling and downstream consumers
    /// should use instead of recomputing ordering against the mirrored
    /// blocklace directly.
    pub fn latest_finalized_ordered_output(
        &mut self,
        wave_length: u64,
    ) -> Result<OrderedFinalizedOutput, SnapshotError> {
        let poll_started = Instant::now();
        let blocklace = self.mirror.blocklace();
        let fingerprint = FinalizedOutputFingerprint {
            generation: blocklace.generation(),
            mirrored_blocks: blocklace.dom().len(),
            wavelength: wave_length,
            weights: WeightSnapshot::from_bonds(&self.bonds),
        };

        // Generation detects all structural mutations (including equal-size
        // prune/insert); the complete weight snapshot avoids hash-only reuse.
        if self.last_finalized_output_fingerprint.as_ref() == Some(&fingerprint)
            && let Some(output) = self.shared_ordered_output.latest().cloned()
        {
            self.advance_deploy_trace(&output);
            self.ordering_diagnostics.last_path = Some("unchanged");
            self.ordering_diagnostics.last_fallback_reason = None;
            self.ordering_diagnostics.cache_entries = self.ordering_cache.cached_entries();
            self.ordering_diagnostics.unchanged_polls += 1;
            tracing::debug!(
                generation = fingerprint.generation,
                mirrored_blocks = fingerprint.mirrored_blocks,
                wavelength = fingerprint.wavelength,
                weight_snapshot = %fingerprint.weights.id(),
                ordering_duration_us = 0u128,
                poll_duration_us = poll_started.elapsed().as_micros(),
                cache_entries = self.ordering_cache.cached_entries(),
                ordering_skipped = true,
                output_changed = false,
                "PoR finalized output unchanged; reusing published result"
            );
            return Ok(output);
        }

        let ordering_started = Instant::now();
        let (incremental, fallback_reason) = self
            .try_extend_finalized_prefix(&fingerprint)
            .map_err(SnapshotError::OrderingFailed)?;
        let (blocks, anchor, ordering_path) = match incremental {
            Some((blocks, anchor)) => (blocks, Some(anchor), "verified_prefix"),
            None => {
                let (blocks, anchor) = {
                    let blocklace = self.mirror.blocklace();
                    let bonds = &self.bonds;
                    let cache = &mut self.ordering_cache;
                    try_ordered_block_identities_with_cache(blocklace, bonds, wave_length, cache)
                        .map_err(SnapshotError::OrderingFailed)?
                };
                (blocks, anchor, "full")
            }
        };
        let ordering_duration_us = ordering_started.elapsed().as_micros();

        let output = OrderedFinalizedOutput::new(
            blocks,
            anchor,
            wave_length,
            self.bonds.len(),
            fingerprint.mirrored_blocks,
        );

        let output_changed = self.shared_ordered_output.latest().is_none_or(|previous| {
            previous.blocks != output.blocks
                || previous.anchor != output.anchor
                || previous.wavelength != output.wavelength
                || previous.bond_count != output.bond_count
                || previous.total_mirrored_blocks != output.total_mirrored_blocks
        });

        self.shared_ordered_output
            .update(output.clone())
            .map_err(|_| SnapshotError::OrderedOutputPrefixViolation)?;
        self.last_finalized_output_fingerprint = Some(fingerprint.clone());
        self.update_finalized_prefix_reuse(fingerprint.clone(), &output);
        self.advance_deploy_trace(&output);
        self.ordering_diagnostics.cache_entries = self.ordering_cache.cached_entries();
        self.ordering_diagnostics.last_path = Some(ordering_path);
        self.ordering_diagnostics.last_fallback_reason = fallback_reason;
        if ordering_path == "verified_prefix" {
            self.ordering_diagnostics.verified_prefix_extensions += 1;
        } else {
            self.ordering_diagnostics.full_recomputations += 1;
            if let Some(reason) = fallback_reason.filter(|reason| *reason != "no_saved_prefix") {
                self.ordering_diagnostics.full_fallbacks += 1;
                *self
                    .ordering_diagnostics
                    .fallbacks_by_reason
                    .entry(reason)
                    .or_default() += 1;
            }
        }

        tracing::debug!(
            generation = fingerprint.generation,
            mirrored_blocks = fingerprint.mirrored_blocks,
            wavelength = fingerprint.wavelength,
            weight_snapshot = %fingerprint.weights.id(),
            ordering_duration_us,
            poll_duration_us = poll_started.elapsed().as_micros(),
            cache_entries = self.ordering_cache.cached_entries(),
            ordering_skipped = false,
            ordering_path,
            output_changed,
            "PoR finalized output computed"
        );

        Ok(output)
    }

    fn try_extend_finalized_prefix(
        &mut self,
        fingerprint: &FinalizedOutputFingerprint,
    ) -> Result<
        (
            Option<(Vec<BlockIdentity>, BlockIdentity)>,
            Option<&'static str>,
        ),
        cordial_miners_core::consensus::OrderingError,
    > {
        let Some(reuse) = self.finalized_prefix_reuse.as_ref() else {
            return Ok((None, Some("no_saved_prefix")));
        };
        let fallback_reason = if !reuse.safe_append {
            Some("unsafe_mirror_mutation")
        } else if reuse.observed_generation != fingerprint.generation {
            Some("generation_mismatch")
        } else if reuse.observed_mirrored_blocks != fingerprint.mirrored_blocks {
            Some("mirror_size_mismatch")
        } else if reuse.fingerprint.generation == fingerprint.generation {
            Some("unchanged_generation")
        } else if reuse.fingerprint.mirrored_blocks >= fingerprint.mirrored_blocks {
            Some("no_mirror_growth")
        } else if reuse.fingerprint.wavelength != fingerprint.wavelength {
            Some("wavelength_changed")
        } else if reuse.fingerprint.weights != fingerprint.weights {
            Some("weights_changed")
        } else if self.mirror.blocklace().has_trusted_boundaries() {
            Some("trusted_boundary")
        } else {
            None
        };
        if let Some(reason) = fallback_reason {
            return Ok((None, Some(reason)));
        }

        let Some(previous) = self
            .shared_ordered_output
            .latest()
            .map(|output| output.blocks.clone())
        else {
            return Ok((None, Some("missing_published_output")));
        };
        let Some((blocks, anchor)) = ordered_block_identities_with_verified_prefix(
            self.mirror.blocklace(),
            &self.bonds,
            fingerprint.wavelength,
            &mut self.ordering_cache,
            &previous,
            &reuse.anchor,
        )?
        else {
            return Ok((None, Some("anchor_not_verified")));
        };
        if !blocks.starts_with(&previous) {
            return Ok((None, Some("suffix_prefix_mismatch")));
        }
        Ok((Some((blocks, anchor)), None))
    }

    fn update_finalized_prefix_reuse(
        &mut self,
        fingerprint: FinalizedOutputFingerprint,
        output: &OrderedFinalizedOutput,
    ) {
        let Some(anchor) = output.anchor.clone() else {
            self.finalized_prefix_reuse = None;
            return;
        };
        let Some(anchor_round) = depth(self.mirror.blocklace(), &anchor) else {
            self.finalized_prefix_reuse = None;
            return;
        };
        let Some(anchor_wave) = wave_of_round(anchor_round, fingerprint.wavelength) else {
            self.finalized_prefix_reuse = None;
            return;
        };
        let Some(anchor_wave_end) = last_round_of_wave(anchor_wave, fingerprint.wavelength) else {
            self.finalized_prefix_reuse = None;
            return;
        };

        self.finalized_prefix_reuse = Some(FinalizedPrefixReuse {
            observed_generation: fingerprint.generation,
            observed_mirrored_blocks: fingerprint.mirrored_blocks,
            safe_append: !self.mirror.blocklace().has_trusted_boundaries(),
            fingerprint,
            anchor,
            anchor_wave_end,
        });
    }

    fn invalidate_finalized_prefix_reuse(&mut self) {
        if let Some(reuse) = &mut self.finalized_prefix_reuse {
            reuse.safe_append = false;
        }
    }

    fn record_strict_mirror_update(
        &mut self,
        generation_before: u64,
        mirrored_blocks_before: usize,
        update: &MirrorUpdate,
        inserted_block: &BlockIdentity,
    ) {
        let Some(reuse) = &mut self.finalized_prefix_reuse else {
            return;
        };

        if update.disposition == MirrorDisposition::Duplicate {
            return;
        }

        if update.disposition != MirrorDisposition::Applied
            || reuse.observed_generation != generation_before
            || reuse.observed_mirrored_blocks != mirrored_blocks_before
        {
            reuse.safe_append = false;
            return;
        }

        let blocklace = self.mirror.blocklace();
        let generation_delta = blocklace.generation().checked_sub(generation_before);
        let mirrored_blocks_delta = blocklace.dom().len().checked_sub(mirrored_blocks_before);
        if update.released_from_buffer == 0
            && generation_delta == Some(1)
            && mirrored_blocks_delta == Some(1)
            && depth(blocklace, inserted_block).is_some_and(|round| round > reuse.anchor_wave_end)
        {
            reuse.observed_generation = blocklace.generation();
            reuse.observed_mirrored_blocks = blocklace.dom().len();
        } else {
            reuse.safe_append = false;
        }
    }

    fn advance_deploy_trace(&self, output: &OrderedFinalizedOutput) {
        if let Some(tracer) = &self.deploy_tracer {
            let finalized_hashes: Vec<String> =
                output.block_hashes().into_iter().map(hex::encode).collect();
            if let Some(anchor_id) = &output.anchor {
                tracer.correlate_finalized_output(&finalized_hashes, &anchor_id.content_hash);
            }
        }
    }

    /// Ingest and persist a trusted block.
    ///
    /// Writes to the LMDB store before inserting into the in-memory mirror.
    /// Crash-safe: a crash between the two leaves the block durable and
    /// recoverable on next boot.
    pub fn ingest_and_persist(
        &mut self,
        block: Block,
        repo: &RSpaceBlocklaceRepository,
    ) -> Result<MirrorUpdate, LiveIngressError>
    where
        A: BlocklaceAdapter<BlockIdentity>,
    {
        repo.put_block(&block)
            .map_err(LiveIngressError::Persistence)?;
        self.ingest_trusted_block(block)
    }

    /// Ingest a trusted block that was reconstructed from a live node-facing
    /// API rather than validated through the raw `BlockMessage` mapper.
    pub fn ingest_trusted_block(&mut self, block: Block) -> Result<MirrorUpdate, LiveIngressError>
    where
        A: BlocklaceAdapter<BlockIdentity>,
    {
        self.adapter
            .on_block(block.clone())
            .map_err(LiveIngressError::Adapter)?;
        let generation_before = self.mirror.blocklace().generation();
        let mirrored_blocks_before = self.mirror.blocklace().dom().len();
        let update = match self.mirror.ingest(block.clone()) {
            Ok(update) => update,
            Err(error) => {
                self.invalidate_finalized_prefix_reuse();
                return Err(LiveIngressError::Mirror(error));
            }
        };
        self.record_strict_mirror_update(
            generation_before,
            mirrored_blocks_before,
            &update,
            &block.identity,
        );
        // Correlate any traced deploys found in this block's payload.
        if let Some(tracer) = &self.deploy_tracer {
            correlate_block_payload(tracer, &block);
        }
        self.phase = LiveIngressPhase::Connected;
        Ok(update)
    }

    /// Ingest a trusted live block while treating missing predecessors as a
    /// bounded-window boundary.
    ///
    /// This is intended for live inspection against a running node when the
    /// verifier intentionally mirrors only recent heights. The normal
    /// `ingest_trusted_block` path remains strict and buffers blocks until the
    /// complete predecessor closure is present.
    pub fn ingest_trusted_window_block(
        &mut self,
        block: Block,
    ) -> Result<MirrorUpdate, LiveIngressError>
    where
        A: BlocklaceAdapter<BlockIdentity>,
    {
        self.adapter
            .on_block(block.clone())
            .map_err(LiveIngressError::Adapter)?;
        let update = match self.mirror.ingest_with_trusted_boundary(block.clone()) {
            Ok(update) => update,
            Err(error) => {
                self.invalidate_finalized_prefix_reuse();
                return Err(LiveIngressError::Mirror(error));
            }
        };
        if update.disposition != MirrorDisposition::Duplicate {
            self.invalidate_finalized_prefix_reuse();
        }
        // Correlate any traced deploys found in this block's payload.
        if let Some(tracer) = &self.deploy_tracer {
            correlate_block_payload(tracer, &block);
        }
        self.phase = LiveIngressPhase::Connected;
        Ok(update)
    }
}

impl<A> LiveIngress<A>
where
    A: BlocklaceAdapter<BlockIdentity>,
{
    /// Accept an adapter-owned block message in Cordial's hash domain.
    ///
    /// Real f1r3node protobuf messages must first pass through
    /// `GrpcBlockMapper::from_protobuf`, which keeps their wire-domain
    /// attestation separate from the translated Cordial block.
    pub fn ingest_block_message(
        &mut self,
        block_msg: &BlockMessage,
    ) -> Result<Block, LiveIngressError> {
        let block = self
            .mapper
            .from_adapter_message(block_msg)
            .map_err(LiveIngressError::Mapping)?;

        self.adapter
            .on_block(block.clone())
            .map_err(LiveIngressError::Adapter)?;

        let generation_before = self.mirror.blocklace().generation();
        let mirrored_blocks_before = self.mirror.blocklace().dom().len();
        let update = match self.mirror.ingest(block.clone()) {
            Ok(update) => update,
            Err(error) => {
                self.invalidate_finalized_prefix_reuse();
                return Err(LiveIngressError::Adapter(anyhow::anyhow!(error)));
            }
        };
        self.record_strict_mirror_update(
            generation_before,
            mirrored_blocks_before,
            &update,
            &block.identity,
        );

        // Correlate traced deploy signatures found in the block body.
        if let Some(tracer) = &self.deploy_tracer {
            let block_hash = block.identity.content_hash.as_slice();
            let height = block_msg.body.state.block_number;
            let sigs: Vec<&[u8]> = block_msg
                .body
                .deploys
                .iter()
                .map(|pd| pd.deploy.sig.as_slice())
                .collect();
            tracer.correlate_block(sigs.into_iter(), block_hash, height);
        }

        self.phase = LiveIngressPhase::Connected;

        Ok(block)
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Private helpers
// ─────────────────────────────────────────────────────────────────────────────

/// Scan a core [`Block`]'s payload for deploy signatures and advance any
/// matching traces to `BlockIncluded`.
///
/// The block payload is decoded as [`CordialBlockPayload`]. If decoding fails
/// (e.g. the block has no payload) the function silently no-ops.
fn correlate_block_payload(tracer: &crate::deploy_trace::DeployTracer, block: &Block) {
    use cordial_miners_core::execution::CordialBlockPayload;

    let payload: CordialBlockPayload = match bincode::deserialize(&block.content.payload) {
        Ok(p) => p,
        Err(_) => return,
    };

    let block_hash = block.identity.content_hash.as_slice();
    // height is not directly available in the core Block type; use 0 as a
    // sentinel — callers using ingest_block_message get the real height.
    let height = 0i64;

    let sigs: Vec<&[u8]> = payload
        .deploys
        .iter()
        .map(|pd| pd.deploy.signature.as_slice())
        .collect();
    tracer.correlate_block(sigs.into_iter(), block_hash, height);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failed_publication_does_not_advance_fingerprint_or_prefix_state() {
        let mut ingress = LiveIngress::new(());
        let bonds = HashMap::new();
        let saved_anchor = BlockIdentity {
            content_hash: [1; 32],
            creator: NodeId(vec![1]),
            signature: Vec::new(),
        };
        let saved_fingerprint = FinalizedOutputFingerprint {
            generation: 0,
            mirrored_blocks: 0,
            wavelength: 3,
            weights: WeightSnapshot::from_bonds(&bonds),
        };
        ingress.finalized_prefix_reuse = Some(FinalizedPrefixReuse {
            fingerprint: saved_fingerprint.clone(),
            anchor: saved_anchor.clone(),
            anchor_wave_end: 2,
            observed_generation: 0,
            observed_mirrored_blocks: 0,
            safe_append: true,
        });
        let previously_published =
            OrderedFinalizedOutput::new(vec![saved_anchor.clone()], Some(saved_anchor), 3, 1, 1);
        ingress
            .shared_ordered_output
            .update(previously_published.clone())
            .expect("test output should be publishable");

        assert_eq!(
            ingress.latest_finalized_ordered_output(3),
            Err(SnapshotError::OrderedOutputPrefixViolation)
        );
        assert_eq!(
            ingress.shared_ordered_output.latest(),
            Some(&previously_published)
        );
        assert!(
            ingress.last_finalized_output_fingerprint.is_none(),
            "a failed publication must not mark its mirror fingerprint as published"
        );
        let reuse = ingress
            .finalized_prefix_reuse
            .as_ref()
            .expect("failed publication must not replace existing prefix state");
        assert_eq!(reuse.fingerprint, saved_fingerprint);
        assert!(reuse.safe_append);
    }

    #[test]
    fn releasing_buffered_blocks_disables_verified_prefix_reuse() {
        let mut ingress = LiveIngress::new(());
        let bonds = HashMap::new();
        let anchor = BlockIdentity {
            content_hash: [1; 32],
            creator: NodeId(vec![1]),
            signature: Vec::new(),
        };
        ingress.finalized_prefix_reuse = Some(FinalizedPrefixReuse {
            fingerprint: FinalizedOutputFingerprint {
                generation: 0,
                mirrored_blocks: 0,
                wavelength: 3,
                weights: WeightSnapshot::from_bonds(&bonds),
            },
            anchor: anchor.clone(),
            anchor_wave_end: 2,
            observed_generation: 0,
            observed_mirrored_blocks: 0,
            safe_append: true,
        });

        ingress.record_strict_mirror_update(
            0,
            0,
            &MirrorUpdate {
                disposition: MirrorDisposition::Applied,
                released_from_buffer: 1,
            },
            &anchor,
        );

        assert!(
            !ingress
                .finalized_prefix_reuse
                .as_ref()
                .expect("test setup should retain reuse state")
                .safe_append
        );
    }
}
