use std::collections::HashMap;

use crate::Block;
use crate::blocklace::Blocklace;
use crate::consensus::{
    InvalidBlock, PendingBlockBuffer, ValidationConfig, ValidationResult, validated_insert,
};
use crate::types::{BlockIdentity, NodeId};

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
    bonds: HashMap<NodeId, u64>,
    validation_config: ValidationConfig,
}

impl SimNode {
    pub fn new(
        id: NodeId,
        bonds: HashMap<NodeId, u64>,
        validation_config: ValidationConfig,
    ) -> Self {
        Self {
            id,
            blocklace: Blocklace::new(),
            pending: PendingBlockBuffer::new(),
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
                if errors
                    .iter()
                    .all(|error| matches!(error, InvalidBlock::MissingPredecessors { .. }))
                {
                    self.pending.buffer_block_with_missing_predecessors(block);
                    DeliveryOutcome::Buffered
                } else {
                    DeliveryOutcome::Rejected(errors)
                }
            }
        }
    }

    /// Retry any buffered blocks against the current local view.
    pub fn retry_buffered_blocks(&mut self) {
        self.pending.retry_buffered_blocks(
            &mut self.blocklace,
            &self.bonds,
            &self.validation_config,
        );
    }

    pub fn knows_block(&self, id: &BlockIdentity) -> bool {
        self.blocklace.content(id).is_some()
    }

    pub fn pending_len(&self) -> usize {
        self.pending.buffered_blocks.len()
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
        let nodes = nodes.into_iter().map(|node| (node.id.clone(), node)).collect();
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
