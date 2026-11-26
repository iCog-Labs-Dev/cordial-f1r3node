//! Live ingress boundary for Cordial Miners integration with f1r3node.
//!
//! This module is the adapter-side home for the "live interception" path.
//! Native blocks enter through `ingest_native_block_message`, which verifies
//! their wire signature and resolves native parent hashes against accepted
//! Cordial identities. Adapter-owned blocks use `ingest_block_message`.
//! Persistent ingress restores the native identity index at startup. No host
//! transport calls this native entry point yet.
//!
use std::collections::HashMap;
use std::time::Instant;

use cordial_miners_core::Block;
use cordial_miners_core::blocklace::Blocklace;
use cordial_miners_core::consensus::{
    OrderingCache, WeightSnapshot, WeightedEpochBoundary, WeightedEpochError, depth,
    last_round_of_wave, wave_of_round, weighted_final_leader_for_wave,
    weighted_tau_after_epoch_boundary,
};
use cordial_miners_core::crypto::CryptoVerifier;
use cordial_miners_core::types::BlockIdentity;
use cordial_miners_core::types::{BlockContent, NodeId};
use cordial_por::{PorError, ReputationState, authorized_validator_weights};
use models::rust::casper::protocol::casper_message::BlockMessage as NativeBlockMessage;

use crate::block_translation::BlockMessage;
use crate::deploy_trace::DeployTracer;
use crate::grpc_ingest::{BlocklaceAdapter, GrpcBlockMapper, NativeBlockIdentityIndex};
use crate::ordered_output::OrderedFinalizedOutput;
use crate::por::finality::finalized_prefix_checkpoint;
use crate::por::{
    FinalizedPrefixCheckpoint, PorWeightEpochPolicy, PorWeightEpochPolicyError,
    PorWeightEpochRecord,
};
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
    MissingReputationCheckpoint {
        round: u64,
    },
    MissingFinalizedOutput {
        source_wave: u64,
    },
    FinalizedCheckpointUnavailable {
        round: u64,
    },
    FinalizedCheckpointMismatch {
        round: u64,
    },
    InvalidWavelength,
    UnknownFinalizedAnchor,
    FinalizedWaveBehind {
        required: u64,
        available: u64,
    },
    ActivatedValidatorSetMismatch {
        configured: usize,
        recorded: usize,
    },
    WouldRewriteFinalizedOutput,
    OrderingFailed(cordial_miners_core::consensus::OrderingError),
    WouldRewriteFinalizedLeader,
    MissingEpochBoundary,
    EffectiveWaveAlreadyPublished {
        effective_wave: u64,
        published_wave: u64,
    },
    FutureEpoch(WeightedEpochError),
    FutureEpochPolicy(PorWeightEpochPolicyError),
    EpochBoundaryWavePending {
        required: u64,
        published: u64,
    },
    FutureEpochAlreadyActive,
}

impl std::fmt::Display for PorWeightActivationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::FutureEpochAlreadyActive => {
                write!(f, "a future-only PoR epoch is already active")
            }
            Self::Projection(error) => write!(f, "cannot project PoR weights: {error}"),
            Self::MissingReputationCheckpoint { round } => {
                write!(f, "PoR reputation round {round} has no audited checkpoint")
            }
            Self::MissingFinalizedOutput { source_wave } => write!(
                f,
                "PoR weights require finalized Cordial source wave {source_wave}, but no ordered output is published"
            ),
            Self::FinalizedCheckpointUnavailable { round } => write!(
                f,
                "PoR weights require finalized Cordial checkpoint round {round}, but its prefix is not yet published"
            ),
            Self::FinalizedCheckpointMismatch { round } => write!(
                f,
                "PoR checkpoint round {round} does not match the published Cordial finalized prefix"
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
            Self::WouldRewriteFinalizedLeader => write!(
                f,
                "PoR weight activation would change the published Cordial final leader"
            ),
            Self::MissingEpochBoundary => write!(
                f,
                "future PoR weight epoch requires a published Cordial final leader"
            ),
            Self::EffectiveWaveAlreadyPublished {
                effective_wave,
                published_wave,
            } => write!(
                f,
                "PoR effective wave {effective_wave} is not after published wave {published_wave}"
            ),
            Self::FutureEpoch(error) => write!(f, "future PoR weight epoch is invalid: {error:?}"),
            Self::FutureEpochPolicy(error) => {
                write!(f, "future PoR weight epoch policy is invalid: {error}")
            }
            Self::EpochBoundaryWavePending {
                required,
                published,
            } => write!(
                f,
                "PoR epoch boundary requires finalized wave {required}, but published wave is {published}"
            ),
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
    native_index: NativeBlockIdentityIndex,
    mirror: LiveBlocklaceMirror,
    ordering_cache: OrderingCache,
    last_finalized_output_fingerprint: Option<FinalizedOutputFingerprint>,
    finalized_prefix_reuse: Option<FinalizedPrefixReuse>,
    ordering_diagnostics: OrderingDiagnostics,
    shared_ordered_output: SharedOrderedOutput,
    bonds: HashMap<NodeId, u64>,
    future_epoch: Option<PorWeightEpochRecord>,
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

type FinalizedPrefixExtension = (Vec<BlockIdentity>, BlockIdentity);
type FinalizedPrefixExtensionAttempt = Result<
    (Option<FinalizedPrefixExtension>, Option<&'static str>),
    cordial_miners_core::consensus::OrderingError,
>;

impl<A> LiveIngress<A> {
    /// Create a new live-ingress wrapper around an adapter-side component.
    pub fn new(adapter: A) -> Self {
        Self {
            phase: LiveIngressPhase::Defined,
            adapter,
            mapper: GrpcBlockMapper::new(),
            native_index: NativeBlockIdentityIndex::new(),
            mirror: LiveBlocklaceMirror::default(),
            ordering_cache: OrderingCache::default(),
            last_finalized_output_fingerprint: None,
            finalized_prefix_reuse: None,
            ordering_diagnostics: OrderingDiagnostics::default(),
            shared_ordered_output: SharedOrderedOutput::new(),
            bonds: HashMap::new(),
            future_epoch: None,
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
            native_index: NativeBlockIdentityIndex::new(),
            mirror: LiveBlocklaceMirror::default(),
            ordering_cache: OrderingCache::default(),
            last_finalized_output_fingerprint: None,
            finalized_prefix_reuse: None,
            ordering_diagnostics: OrderingDiagnostics::default(),
            shared_ordered_output: SharedOrderedOutput::new(),
            bonds,
            future_epoch: None,
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
        let mut native_index = NativeBlockIdentityIndex::new();
        for (wire_hash, identity) in repo.native_block_mappings()? {
            let block = mirror.blocklace.get(&identity).ok_or_else(|| {
                RepoError::NativeMapping(format!(
                    "native wire hash references an unrecovered Cordial block: {identity:?}"
                ))
            })?;
            native_index
                .record_recovered(wire_hash, &block)
                .map_err(|error| RepoError::NativeMapping(error.to_string()))?;
        }

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
            native_index,
            mirror,
            ordering_cache: OrderingCache::default(),
            last_finalized_output_fingerprint: None,
            finalized_prefix_reuse: None,
            ordering_diagnostics: OrderingDiagnostics::default(),
            shared_ordered_output: SharedOrderedOutput::new(),
            bonds,
            future_epoch: None,
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

    pub fn native_identity_for_wire_hash(&self, wire_hash: &[u8; 32]) -> Option<&BlockIdentity> {
        self.native_index.get(wire_hash)
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

    pub fn future_epoch(&self) -> Option<&PorWeightEpochRecord> {
        self.future_epoch.as_ref()
    }

    /// Verify the committed cutover against the retained DAG before exposing
    /// its weights or finalized output. Repeating the same installation is safe.
    pub fn install_future_epoch(
        &mut self,
        record: PorWeightEpochRecord,
        committed_checkpoint_hash: [u8; 32],
    ) -> Result<(), SnapshotError> {
        if record.reputation_checkpoint_hash() != committed_checkpoint_hash {
            return Err(SnapshotError::EpochReplay(
                "committed checkpoint changed".into(),
            ));
        }
        if let Some(active) = &self.future_epoch {
            return if active == &record {
                Ok(())
            } else {
                Err(SnapshotError::EpochReplay(
                    "a different future epoch is active".into(),
                ))
            };
        }
        if self.bonds != record.old_weights() {
            return Err(SnapshotError::EpochReplay(
                "old epoch weights changed".into(),
            ));
        }
        let mut authorized: Vec<_> = self.bonds.keys().cloned().collect();
        authorized.sort();
        let output = record
            .verify_replay(self.blocklace(), &authorized, committed_checkpoint_hash)
            .map_err(|error| SnapshotError::EpochReplay(error.to_string()))?;
        if let Some(previous) = self.shared_ordered_output.latest()
            && !output.blocks.starts_with(&previous.blocks)
        {
            return Err(SnapshotError::OrderedOutputPrefixViolation);
        }
        self.shared_ordered_output
            .update(output)
            .map_err(|_| SnapshotError::OrderedOutputPrefixViolation)?;
        self.bonds = record.new_weights();
        self.ordering_cache = OrderingCache::default();
        self.future_epoch = Some(record);
        Ok(())
    }

    fn replay_future_epoch(&self) -> Result<OrderedFinalizedOutput, SnapshotError> {
        let record = self
            .future_epoch
            .as_ref()
            .expect("future epoch checked by caller");
        if self.bonds != record.new_weights() {
            return Err(SnapshotError::EpochReplay(
                "active epoch weights changed".into(),
            ));
        }
        let mut authorized: Vec<_> = self.bonds.keys().cloned().collect();
        authorized.sort();
        record
            .verify_replay(
                self.blocklace(),
                &authorized,
                record.reputation_checkpoint_hash(),
            )
            .map_err(|error| SnapshotError::EpochReplay(error.to_string()))
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
        if self.future_epoch.is_some() {
            return Err(PorWeightActivationError::FutureEpochAlreadyActive);
        }
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
    /// that Cordial has already published the source wave or checkpoint. A
    /// prospective weighted-order calculation must preserve the published
    /// finalized prefix and final leader, so a weight change cannot rewrite
    /// prior finality.
    /// Projection and safety checks finish before the live weights change.
    pub fn apply_por_weights(
        &mut self,
        state: &ReputationState,
    ) -> Result<(), PorWeightActivationError> {
        if self.future_epoch.is_some() {
            return Err(PorWeightActivationError::FutureEpochAlreadyActive);
        }
        let next_weights = self.checked_por_weights(state)?;
        self.bonds = next_weights;
        self.ordering_cache = OrderingCache::default();
        self.invalidate_finalized_prefix_reuse();
        Ok(())
    }

    /// Check a prospective PoR projection without changing the live weights.
    ///
    /// A host can call this before durably committing the reputation state to
    /// reject a known unsafe activation while its previous round is intact.
    pub fn validate_por_weights(
        &self,
        state: &ReputationState,
    ) -> Result<(), PorWeightActivationError> {
        self.checked_por_weights(state).map(|_| ())
    }

    /// Preview an epoch cutover without changing weights or published output.
    ///
    /// The caller supplies an audited projection and a network-agreed effective
    /// wave. The complete current publication becomes the sealed old-weight
    /// prefix. A cutover into a wave that was already published fails closed.
    /// This method does not persist or activate an epoch.
    pub fn preview_future_weight_epoch(
        &self,
        projected: &HashMap<NodeId, u64>,
        effective_wave: u64,
    ) -> Result<OrderedFinalizedOutput, PorWeightActivationError> {
        if self.bonds.len() != projected.len()
            || !self
                .bonds
                .keys()
                .all(|validator| projected.contains_key(validator))
        {
            return Err(PorWeightActivationError::ActivatedValidatorSetMismatch {
                configured: self.bonds.len(),
                recorded: projected.len(),
            });
        }
        if self.bonds.is_empty() {
            return Err(PorWeightActivationError::MissingEpochBoundary);
        }
        let published = self
            .shared_ordered_output
            .latest()
            .ok_or(PorWeightActivationError::MissingEpochBoundary)?;
        let leader = published
            .anchor
            .as_ref()
            .ok_or(PorWeightActivationError::MissingEpochBoundary)?;
        let leader_round = depth(self.blocklace(), leader)
            .ok_or(PorWeightActivationError::UnknownFinalizedAnchor)?;
        let published_wave = wave_of_round(leader_round, published.wavelength)
            .ok_or(PorWeightActivationError::InvalidWavelength)?;
        if effective_wave <= published_wave {
            return Err(PorWeightActivationError::EffectiveWaveAlreadyPublished {
                effective_wave,
                published_wave,
            });
        }

        let mut validators: Vec<_> = self.bonds.keys().cloned().collect();
        validators.sort();
        let leader_of_wave = |wave: u64| {
            let index = usize::try_from(wave).ok()? % validators.len();
            Some(validators[index].clone())
        };
        let (blocks, anchor) = weighted_tau_after_epoch_boundary(
            self.blocklace(),
            published.wavelength,
            &self.bonds,
            projected,
            WeightedEpochBoundary {
                final_leader: leader,
                ordered_prefix: &published.blocks,
                effective_wave,
            },
            leader_of_wave,
        )
        .map_err(PorWeightActivationError::FutureEpoch)?;
        Ok(OrderedFinalizedOutput::new(
            blocks,
            Some(anchor),
            published.wavelength,
            projected.len(),
            self.blocklace().dom().len(),
        ))
    }

    /// Derive a checkpoint's fixed effective wave, then preview its projection.
    /// A late cutover fails instead of moving the wave to this observer's head.
    pub fn preview_checkpoint_weight_epoch(
        &self,
        projected: &HashMap<NodeId, u64>,
        source: &FinalizedPrefixCheckpoint,
        policy: PorWeightEpochPolicy,
    ) -> Result<(u64, OrderedFinalizedOutput), PorWeightActivationError> {
        let published = self
            .shared_ordered_output
            .latest()
            .ok_or(PorWeightActivationError::MissingEpochBoundary)?;
        let effective_wave = policy
            .effective_wave(self.blocklace(), published, source)
            .map_err(PorWeightActivationError::FutureEpochPolicy)?;
        let leader = published
            .anchor
            .as_ref()
            .ok_or(PorWeightActivationError::MissingEpochBoundary)?;
        let leader_round = depth(self.blocklace(), leader)
            .ok_or(PorWeightActivationError::UnknownFinalizedAnchor)?;
        let published_wave = wave_of_round(leader_round, published.wavelength)
            .ok_or(PorWeightActivationError::InvalidWavelength)?;
        let required_boundary_wave = effective_wave - 1;
        if published_wave < required_boundary_wave {
            return Err(PorWeightActivationError::EpochBoundaryWavePending {
                required: required_boundary_wave,
                published: published_wave,
            });
        }
        let preview = self.preview_future_weight_epoch(projected, effective_wave)?;
        Ok((effective_wave, preview))
    }

    fn checked_por_weights(
        &self,
        state: &ReputationState,
    ) -> Result<HashMap<NodeId, u64>, PorWeightActivationError> {
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
            if let Some(source) = checkpoint.header.source_checkpoint {
                let local = finalized_prefix_checkpoint(output, source.round).map_err(|_| {
                    PorWeightActivationError::FinalizedCheckpointMismatch {
                        round: source.round,
                    }
                })?;
                let Some(local) = local else {
                    return Err(PorWeightActivationError::FinalizedCheckpointUnavailable {
                        round: source.round,
                    });
                };
                if local.rating_source() != source {
                    return Err(PorWeightActivationError::FinalizedCheckpointMismatch {
                        round: source.round,
                    });
                }
            } else {
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
        }

        if let Some(previous) = self.shared_ordered_output.latest() {
            if let Some(previous_leader) = &previous.anchor
                && next_weights.len() > 1
            {
                let leader_round = depth(self.blocklace(), previous_leader)
                    .ok_or(PorWeightActivationError::UnknownFinalizedAnchor)?;
                let wave = wave_of_round(leader_round, previous.wavelength)
                    .ok_or(PorWeightActivationError::InvalidWavelength)?;
                let prospective_leader = weighted_final_leader_for_wave(
                    self.blocklace(),
                    wave,
                    previous.wavelength,
                    &next_weights,
                    |selected_wave| {
                        let index =
                            usize::try_from(selected_wave).ok()? % authorized_validators.len();
                        Some(authorized_validators[index].clone())
                    },
                );
                if prospective_leader.as_ref() != Some(previous_leader) {
                    return Err(PorWeightActivationError::WouldRewriteFinalizedLeader);
                }
            }
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

        Ok(next_weights)
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
        let mut snapshot = build_snapshot(
            self.blocklace(),
            &self.bonds,
            self.shard_conf.to_snapshot_conf(),
            &self.shard_id,
        )?;
        if self.future_epoch.is_some() {
            let output = self.replay_future_epoch()?;
            let hashes = output.block_hashes();
            let anchor = output
                .anchor_hash()
                .map(|hash| hash.to_vec())
                .unwrap_or_default();
            snapshot.last_finalized_block = anchor.clone();
            snapshot.ordered_finalized_blocks = hashes.clone();
            snapshot.dag.last_finalized_block_hash = anchor;
            snapshot.dag.finalized_blocks_set = hashes.into_iter().collect();
        }
        Ok(snapshot)
    }

    /// Return the latest finalized block hash if the mirrored state has one.
    pub fn last_finalized_block_hash(&self) -> Result<Option<Vec<u8>>, SnapshotError> {
        if self.future_epoch.is_some() {
            return Ok(self
                .replay_future_epoch()?
                .anchor_hash()
                .map(|hash| hash.to_vec()));
        }
        Ok(latest_finalized_block_id(self.blocklace(), &self.bonds)
            .map(|id| id.content_hash.to_vec()))
    }

    /// Return the current weighted tau output as block hashes.
    pub fn ordered_finalized_blocks(&mut self) -> Result<Vec<Vec<u8>>, SnapshotError> {
        if self.future_epoch.is_some() {
            return Ok(self.replay_future_epoch()?.block_hashes());
        }
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
        // Future epochs have their own replay-verified ordering path. Do not
        // reuse the ordinary fingerprint/prefix cache across this boundary.
        if self.future_epoch.is_some() {
            let output = self.replay_future_epoch()?;
            if output.wavelength != wave_length {
                return Err(SnapshotError::EpochReplay(
                    "epoch wavelength changed".into(),
                ));
            }
            self.shared_ordered_output
                .update(output.clone())
                .map_err(|_| SnapshotError::OrderedOutputPrefixViolation)?;
            self.last_finalized_output_fingerprint = None;
            self.finalized_prefix_reuse = None;
            if let Some(tracer) = &self.deploy_tracer {
                let finalized_hashes: Vec<String> =
                    output.block_hashes().into_iter().map(hex::encode).collect();
                if let Some(anchor) = &output.anchor {
                    tracer.correlate_finalized_output(&finalized_hashes, &anchor.content_hash);
                }
            }
            return Ok(output);
        }

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
    ) -> FinalizedPrefixExtensionAttempt {
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
    /// Accept a native f1r3node block after checking its wire hash, signature,
    /// and complete native-to-Cordial predecessor mapping.
    ///
    /// The native index is updated only after both the adapter and mirror have
    /// accepted the translated block. This in-memory entry point does not
    /// persist blocks; use `ingest_native_block_message_and_persist` when the
    /// mapping must survive a restart.
    pub fn ingest_native_block_message(
        &mut self,
        block_msg: &NativeBlockMessage,
    ) -> Result<MirrorUpdate, LiveIngressError> {
        self.ingest_native_block_message_inner(block_msg, None, false)
    }

    /// Accept and durably store a verified native block and its wire-hash
    /// mapping in one LMDB transaction before publishing it in the mirror.
    pub fn ingest_native_block_message_and_persist(
        &mut self,
        block_msg: &NativeBlockMessage,
        repo: &RSpaceBlocklaceRepository,
    ) -> Result<MirrorUpdate, LiveIngressError> {
        self.ingest_native_block_message_inner(block_msg, Some(repo), false)
    }

    /// Admit the host's unsigned genesis from a trusted local block source.
    /// This exception requires the exact unsigned root shape and wire hash.
    pub fn ingest_trusted_unsigned_native_genesis_and_persist(
        &mut self,
        block_msg: &NativeBlockMessage,
        repo: &RSpaceBlocklaceRepository,
    ) -> Result<MirrorUpdate, LiveIngressError> {
        self.ingest_native_block_message_inner(block_msg, Some(repo), true)
    }

    fn ingest_native_block_message_inner(
        &mut self,
        block_msg: &NativeBlockMessage,
        repo: Option<&RSpaceBlocklaceRepository>,
        trusted_unsigned_genesis: bool,
    ) -> Result<MirrorUpdate, LiveIngressError> {
        let validated = if trusted_unsigned_genesis {
            self.mapper.from_trusted_unsigned_genesis(block_msg)
        } else {
            self.mapper
                .from_protobuf_with_parents(block_msg, &self.native_index)
        }
        .map_err(LiveIngressError::Mapping)?;
        self.native_index
            .check_candidate(&validated)
            .map_err(LiveIngressError::Mapping)?;

        if trusted_unsigned_genesis
            && self.native_index.get(&validated.wire_hash).is_none()
            && !self.mirror.blocklace().dom().is_empty()
        {
            return Err(LiveIngressError::Mirror(
                "unsigned genesis cannot be added after other Cordial blocks".into(),
            ));
        }

        let block = &validated.block;
        if let Some(known_identity) = self.native_index.get(&validated.wire_hash) {
            if self.mirror.blocklace().get(known_identity).is_none() {
                return Err(LiveIngressError::Mirror(
                    "indexed native block is absent from the Cordial mirror".to_string(),
                ));
            }
            if let Some(repo) = repo {
                repo.put_native_block(block, &validated.wire_hash)
                    .map_err(LiveIngressError::Persistence)?;
            }
            return Ok(MirrorUpdate {
                disposition: MirrorDisposition::Duplicate,
                released_from_buffer: 0,
            });
        }

        if self.mirror.blocklace().get(&block.identity).is_some()
            || self.mirror.pending().contains_key(&block.identity)
        {
            return Err(LiveIngressError::Mapping(anyhow::anyhow!(
                "Cordial block is present without its native wire-hash mapping"
            )));
        }
        if block
            .content
            .predecessors
            .iter()
            .any(|parent| self.mirror.blocklace().get(parent).is_none())
        {
            return Err(LiveIngressError::Mirror(
                "resolved native parent is absent from the Cordial mirror".to_string(),
            ));
        }

        self.adapter
            .on_block(block.clone())
            .map_err(LiveIngressError::Adapter)?;
        if let Some(repo) = repo {
            repo.put_native_block(block, &validated.wire_hash)
                .map_err(LiveIngressError::Persistence)?;
        }
        let update = self
            .mirror
            .ingest(block.clone())
            .map_err(LiveIngressError::Mirror)?;
        if update.disposition != MirrorDisposition::Applied {
            return Err(LiveIngressError::Mirror(
                "native block was not applied after predecessor preflight".to_string(),
            ));
        }
        self.native_index
            .record_accepted(&validated)
            .map_err(LiveIngressError::Mapping)?;
        if let Some(tracer) = &self.deploy_tracer {
            correlate_block_payload(tracer, block);
        }
        self.phase = LiveIngressPhase::Connected;
        Ok(update)
    }

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

    let payload = match CordialBlockPayload::from_bytes(&block.content.payload) {
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
