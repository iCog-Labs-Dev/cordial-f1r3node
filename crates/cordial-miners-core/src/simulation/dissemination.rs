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
