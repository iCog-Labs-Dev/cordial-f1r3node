use std::collections::HashMap;

use crate::Block;
use crate::blocklace::Blocklace;
use crate::consensus::OrderingError;
#[cfg(feature = "trace")]
use crate::consensus::round::{candidate_depth, compute_all_depths};
#[cfg(feature = "trace")]
use crate::consensus::wave::wave_of_round;
use crate::consensus::{
    CordialEquivocationEvidence, CordialEvidencePool, EvidencePool, InvalidBlock,
    PendingBlockBuffer, ProposalError, ValidationConfig, ValidationResult, build_block_candidate,
    latest_final_leader, latest_weighted_final_leader, record_rejected_equivocation, tau,
    validated_insert, weighted_tau,
};
#[cfg(feature = "trace")]
use crate::trace::{
    self, BlockLifecycleEvent, ResolveMissingParentEvent, TraceEvent, WaveTaskEvent,
};
use crate::types::{BlockContent, BlockIdentity, NodeId};

/// Outcome of delivering a block to a simulated node.
#[derive(Debug, Clone, PartialEq)]
pub enum DeliveryOutcome {
    Inserted,
    Buffered,
    Rejected(Vec<InvalidBlock>),
}

#[derive(Debug, Clone)]
struct PendingDelivery {
    recipient: NodeId,
    block: Block,
}

/// Minimal node model for dissemination simulations.
///
/// This keeps only the local state needed for early dissemination tests:
/// - a local blocklace view
/// - a pending buffer for out-of-order blocks
/// - validation inputs used when receiving or retrying blocks
pub struct SimNode {
    pub id: NodeId,
    pub blocklace: Blocklace,
    pub pending: PendingBlockBuffer,
    /// Equivocation proof captured at the moment of rejection.
    ///
    /// Held per node rather than globally: whether a node has the proof depends
    /// on whether *it* saw both branches, which is exactly what a partition
    /// changes.
    pub evidence: CordialEvidencePool,
    bonds: HashMap<NodeId, u64>,
    validation_config: ValidationConfig,
}

impl SimNode {
    #[cfg(feature = "trace")]
    fn emit_wave_task(&self, wavelength: u64, task: &str) {
        let Some(max_round) = compute_all_depths(&self.blocklace).values().copied().max() else {
            return;
        };
        let Some(wave) = wave_of_round(max_round, wavelength) else {
            return;
        };
        trace::emit(TraceEvent::RunWaveTask(WaveTaskEvent {
            node_id: trace::hex(&self.id.0),
            wave,
            task: task.into(),
        }));
    }

    pub fn new(
        id: NodeId,
        bonds: HashMap<NodeId, u64>,
        validation_config: ValidationConfig,
    ) -> Self {
        Self {
            id,
            blocklace: Blocklace::new(),
            pending: PendingBlockBuffer::new(),
            evidence: CordialEvidencePool::new(),
            bonds,
            validation_config,
        }
    }

    /// Deliver a block into the node's local view.
    ///
    /// Missing-predecessor blocks are buffered for later replay. Blocks that
    /// fail for any other reason are rejected immediately.
    pub fn receive_block(&mut self, block: Block) -> DeliveryOutcome {
        match validated_insert(
            block.clone(),
            &mut self.blocklace,
            &self.bonds,
            &self.validation_config,
        ) {
            ValidationResult::Valid => DeliveryOutcome::Inserted,
            ValidationResult::Invalid(errors) => {
                // Capture equivocation proof before the block is dropped. The
                // chain axiom keeps both branches from ever coexisting in the
                // blocklace, so rejection is the only moment the pair is in
                // hand.
                record_rejected_equivocation(&block, &errors, &self.blocklace, &mut self.evidence);

                if errors
                    .iter()
                    .all(|error| matches!(error, InvalidBlock::MissingPredecessors { .. }))
                {
                    let missing: std::collections::HashSet<_> = block
                        .content
                        .predecessors
                        .iter()
                        .filter(|parent| self.blocklace.content(parent).is_none())
                        .cloned()
                        .collect();
                    #[cfg(feature = "trace")]
                    {
                        trace::emit(TraceEvent::BufferBlock(BlockLifecycleEvent {
                            node_id: trace::hex(&self.id.0),
                            wave: None,
                            round: candidate_depth(&self.blocklace, &block.content),
                            block_hash: trace::hex(&block.identity.content_hash),
                            parent_hashes: trace::sorted_block_hashes(&block.content.predecessors),
                            missing_parent_hashes: trace::sorted_block_hashes(&missing),
                            creator: trace::hex(&block.identity.creator.0),
                            weight_table_hash: Some(trace::weight_table_hash(&self.bonds)),
                        }));
                    }
                    self.pending
                        .buffer_block_with_missing_predecessors_known(block, missing);
                    DeliveryOutcome::Buffered
                } else {
                    DeliveryOutcome::Rejected(errors)
                }
            }
        }
    }

    /// Retry any buffered blocks against the current local view.
    ///
    /// A block buffered as `MissingPredecessors` can turn out to be an
    /// equivocation once its history arrives, and this replay is where that
    /// rejection happens — so proof is captured here too, exactly as in
    /// `receive_block`, before the rejected block is dropped.
    pub fn retry_buffered_blocks(&mut self) {
        let resolved = self.pending.take_resolved_predecessors(&self.blocklace);
        #[cfg(not(feature = "trace"))]
        let _ = resolved;
        #[cfg(feature = "trace")]
        for (block_id, parent) in resolved {
            trace::emit(TraceEvent::ResolveMissingParent(
                ResolveMissingParentEvent {
                    node_id: trace::hex(&self.id.0),
                    block_hash: trace::hex(&block_id.content_hash),
                    resolved_parent_hash: trace::hex(&parent.content_hash),
                },
            ));
        }
        let rejected = self.pending.retry_buffered_blocks(
            &mut self.blocklace,
            &self.bonds,
            &self.validation_config,
        );
        for (block, errors) in rejected {
            record_rejected_equivocation(&block, &errors, &self.blocklace, &mut self.evidence);
        }
    }

    pub fn knows_block(&self, id: &BlockIdentity) -> bool {
        self.blocklace.content(id).is_some()
    }

    pub fn pending_len(&self) -> usize {
        self.pending.buffered_blocks.len()
    }

    /// Equivocation proof this node holds for `validator`.
    pub fn evidence_for(&self, validator: &NodeId) -> Vec<CordialEquivocationEvidence> {
        self.evidence.evidence_for(validator)
    }

    /// Whether this node holds proof that `validator` equivocated.
    pub fn has_evidence_against(&self, validator: &NodeId) -> bool {
        !self.evidence.evidence_for(validator).is_empty()
    }

    /// Build a local block candidate from the node's current view.
    pub fn build_block_candidate(&self, payload: Vec<u8>) -> Result<BlockContent, ProposalError> {
        build_block_candidate(&self.blocklace, &self.bonds, payload)
    }

    pub fn latest_final_leader<F>(
        &self,
        wavelength: u64,
        n: usize,
        f: usize,
        leader_selection: F,
    ) -> Option<BlockIdentity>
    where
        F: Fn(u64) -> Option<NodeId> + Copy,
    {
        #[cfg(feature = "trace")]
        self.emit_wave_task(wavelength, "finalize");
        latest_final_leader(&self.blocklace, wavelength, n, f, leader_selection)
    }

    pub fn ordered_output<F>(
        &self,
        wavelength: u64,
        n: usize,
        f: usize,
        leader_selection: F,
    ) -> Result<Vec<BlockIdentity>, OrderingError>
    where
        F: Fn(u64) -> Option<NodeId> + Copy,
    {
        tau(&self.blocklace, wavelength, n, f, leader_selection)
    }

    pub fn latest_weighted_final_leader<F>(
        &self,
        wavelength: u64,
        leader_selection: F,
    ) -> Option<BlockIdentity>
    where
        F: Fn(u64) -> Option<NodeId> + Copy,
    {
        #[cfg(feature = "trace")]
        self.emit_wave_task(wavelength, "finalize");
        latest_weighted_final_leader(&self.blocklace, wavelength, &self.bonds, leader_selection)
    }

    pub fn weighted_ordered_output<F>(
        &self,
        wavelength: u64,
        leader_selection: F,
    ) -> Result<Vec<BlockIdentity>, OrderingError>
    where
        F: Fn(u64) -> Option<NodeId> + Copy,
    {
        weighted_tau(&self.blocklace, wavelength, &self.bonds, leader_selection)
    }
}

/// Minimal network harness for dissemination simulations.
///
/// This keeps a set of simulated nodes plus an explicit delivery queue so tests
/// can control message order.
pub struct SimNetwork {
    pub nodes: HashMap<NodeId, SimNode>,
    pending_deliveries: Vec<PendingDelivery>,
}

impl SimNetwork {
    pub fn new(nodes: Vec<SimNode>) -> Self {
        let nodes = nodes
            .into_iter()
            .map(|node| (node.id.clone(), node))
            .collect();
        Self {
            nodes,
            pending_deliveries: Vec::new(),
        }
    }

    pub fn node(&self, id: &NodeId) -> Option<&SimNode> {
        self.nodes.get(id)
    }

    pub fn node_mut(&mut self, id: &NodeId) -> Option<&mut SimNode> {
        self.nodes.get_mut(id)
    }

    pub fn queue_delivery(&mut self, recipient: NodeId, block: Block) {
        self.pending_deliveries
            .push(PendingDelivery { recipient, block });
    }

    pub fn queued_delivery_count(&self) -> usize {
        self.pending_deliveries.len()
    }

    pub fn deliver_next_to(&mut self, recipient: &NodeId) -> Option<DeliveryOutcome> {
        let idx = self
            .pending_deliveries
            .iter()
            .position(|delivery| &delivery.recipient == recipient)?;
        let delivery = self.pending_deliveries.remove(idx);
        let node = self.nodes.get_mut(recipient)?;
        Some(node.receive_block(delivery.block))
    }

    pub fn retry_all_buffers(&mut self) {
        for node in self.nodes.values_mut() {
            node.retry_buffered_blocks();
        }
    }
}
