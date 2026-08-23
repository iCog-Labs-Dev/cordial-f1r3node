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

use cordial_miners_core::Block;
use cordial_miners_core::blocklace::Blocklace;
use cordial_miners_core::consensus::OrderingCache;
use cordial_miners_core::crypto::CryptoVerifier;
use cordial_miners_core::types::BlockIdentity;
use cordial_miners_core::types::{BlockContent, NodeId};

use crate::block_translation::BlockMessage;
use crate::deploy_trace::DeployTracer;
use crate::grpc_ingest::{BlocklaceAdapter, GrpcBlockMapper};
use crate::ordered_output::OrderedFinalizedOutput;
use crate::repository::{BlocklaceRepository, RSpaceBlocklaceRepository, RepoError};
use crate::shard_conf::CasperShardConf;
use crate::shared_ordered_output::{ReadOrderedOutput, SharedOrderedOutput};
use crate::snapshot::{
    CasperSnapshot, SnapshotError, build_snapshot, latest_finalized_block_id,
    ordered_block_identities_with_cache, ordered_finalized_block_hashes_with_cache,
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

    pub fn ingest(&mut self, block: Block) -> Result<MirrorUpdate, String> {
        let block = self.canonicalize_block(block);

        if self.blocklace.get(&block.identity).is_some() {
            return Ok(MirrorUpdate {
                disposition: MirrorDisposition::Duplicate,
                released_from_buffer: 0,
            });
        }

        if self.pending.contains_key(&block.identity) {
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
        let mut block = self.canonicalize_block(block);

        if self.blocklace.get(&block.identity).is_some() {
            return Ok(MirrorUpdate {
                disposition: MirrorDisposition::Duplicate,
                released_from_buffer: 0,
            });
        }

        if self.pending.contains_key(&block.identity) {
            return Ok(MirrorUpdate {
                disposition: MirrorDisposition::Duplicate,
                released_from_buffer: 0,
            });
        }

        block
            .content
            .predecessors
            .retain(|pred_id| self.resolve_known_identity(pred_id).is_some());

        self.blocklace.insert(block, &self.verifier)?;
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
            .all(|pred_id| self.resolve_known_identity(pred_id).is_some())
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
                let block = self.canonicalize_block(block);
                self.blocklace.insert(block, &self.verifier)?;
                released += 1;
            }
        }

        Ok(released)
    }

    fn canonicalize_block(&self, mut block: Block) -> Block {
        block.content.predecessors = block
            .content
            .predecessors
            .iter()
            .map(|pred_id| {
                self.resolve_known_identity(pred_id)
                    .unwrap_or_else(|| pred_id.clone())
            })
            .collect();
        block
    }

    fn resolve_known_identity(&self, pred_id: &BlockIdentity) -> Option<BlockIdentity> {
        let exact = self
            .blocklace
            .dom()
            .into_iter()
            .find(|known| {
                known.content_hash == pred_id.content_hash && known.creator == pred_id.creator
            })
            .cloned();

        if exact.is_some() {
            return exact;
        }

        let mut same_hash = self
            .blocklace
            .dom()
            .into_iter()
            .filter(|known| known.content_hash == pred_id.content_hash)
            .cloned();
        let first = same_hash.next()?;
        if same_hash.next().is_none() {
            Some(first)
        } else {
            None
        }
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
    shared_ordered_output: SharedOrderedOutput,
    bonds: HashMap<NodeId, u64>,
    shard_conf: CasperShardConf,
    shard_id: String,
    /// Optional deploy tracer. When set, block ingestion and finalized-output
    /// computation automatically advance the deploy lifecycle state machine.
    deploy_tracer: Option<DeployTracer>,
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

    /// Replace the bonded validator set used for finality and ordering views.
    pub fn set_bonds(&mut self, bonds: HashMap<NodeId, u64>) {
        self.bonds = bonds;
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
    /// `ordered_output` export seam: the finalized-prefix blocks (full
    /// [`BlockIdentity`] entries, not bare hashes), linearized via weighted
    /// tau ordering, together with the anchor and consensus metadata needed
    /// to interpret them.
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
        let (blocks, anchor) = {
            let blocklace = self.mirror.blocklace();
            let bonds = &self.bonds;
            let cache = &mut self.ordering_cache;
            ordered_block_identities_with_cache(blocklace, bonds, cache)
        };

        let total_mirrored_blocks = self.mirror.blocklace().dom().len();

        let output = OrderedFinalizedOutput::new(
            blocks,
            anchor,
            wave_length,
            self.bonds.len(),
            total_mirrored_blocks,
        );

        // Advance deploy traces to FinalizedOrdered for blocks in this output.
        if let Some(tracer) = &self.deploy_tracer {
            let finalized_hashes: Vec<String> =
                output.block_hashes().into_iter().map(hex::encode).collect();
            if let Some(anchor_id) = &output.anchor {
                tracer.correlate_finalized_output(&finalized_hashes, &anchor_id.content_hash);
            }
        }

        self.shared_ordered_output
            .update(output.clone())
            .map_err(|_| SnapshotError::OrderedOutputPrefixViolation)?;

        Ok(output)
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
        let update = self
            .mirror
            .ingest(block.clone())
            .map_err(LiveIngressError::Mirror)?;
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
        let update = self
            .mirror
            .ingest_with_trusted_boundary(block.clone())
            .map_err(LiveIngressError::Mirror)?;
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

        self.mirror
            .ingest(block.clone())
            .map_err(|err| LiveIngressError::Adapter(anyhow::anyhow!(err)))?;

        // Correlate traced deploy signatures found in the block body.
        if let Some(tracer) = &self.deploy_tracer {
            let block_hash = &block_msg.block_hash;
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
