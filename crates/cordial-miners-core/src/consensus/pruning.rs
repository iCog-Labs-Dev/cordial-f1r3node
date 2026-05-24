//! Checkpoint-based garbage collection for the in-memory blocklace.
//!
//! Once a leader has finalized and the corresponding tau prefix has been
//! materialized, the consensus engine can treat that leader as the new in-memory
//! genesis and delete older orphaned block contents.

use std::collections::{BTreeSet, HashMap};

use crate::blocklace::Blocklace;
use crate::consensus::finality::{latest_final_leader, latest_weighted_final_leader};
use crate::consensus::ordering::{tau, weighted_tau, xsort};
use crate::consensus::round::depth;
use crate::types::{BlockIdentity, NodeId};

/// Trait for DAG stores that can expose and advance a finalized checkpoint.
pub trait CheckpointGc<Id> {
    fn current_checkpoint(&self) -> Option<&Id>;
    fn prune_below_checkpoint(
        &mut self,
        checkpoint: &Id,
    ) -> Result<PruneReport<Id>, PruneError<Id>>;
}

/// Summary of a checkpoint prune operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PruneReport<Id> {
    pub checkpoint: Id,
    pub checkpoint_depth: u64,
    pub removed: BTreeSet<Id>,
    pub retained_blocks: usize,
    pub tau_prefix_len: usize,
    pub weighted_tau_prefix_len: usize,
}

/// Reasons checkpoint pruning can be rejected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PruneError<Id> {
    UnknownCheckpoint {
        checkpoint: Box<Id>,
    },
    CheckpointRegression {
        current: Box<Id>,
        requested: Box<Id>,
    },
    DisconnectedCheckpoint {
        current: Box<Id>,
        requested: Box<Id>,
    },
}

impl CheckpointGc<BlockIdentity> for Blocklace {
    fn current_checkpoint(&self) -> Option<&BlockIdentity> {
        self.checkpoint()
    }

    fn prune_below_checkpoint(
        &mut self,
        checkpoint: &BlockIdentity,
    ) -> Result<PruneReport<BlockIdentity>, PruneError<BlockIdentity>> {
        let checkpoint_order_prefix = checkpoint_order_prefix(self, checkpoint);
        self.prune_below_checkpoint_with_prefix(checkpoint, checkpoint_order_prefix, Vec::new())
    }
}

impl Blocklace {
    fn prune_below_checkpoint_with_prefix(
        &mut self,
        checkpoint: &BlockIdentity,
        checkpoint_order_prefix: Vec<BlockIdentity>,
        checkpoint_weighted_order_prefix: Vec<BlockIdentity>,
    ) -> Result<PruneReport<BlockIdentity>, PruneError<BlockIdentity>> {
        if !self.blocks.contains_key(checkpoint) {
            return Err(PruneError::UnknownCheckpoint {
                checkpoint: Box::new(checkpoint.clone()),
            });
        }

        let checkpoint_depth =
            depth(self, checkpoint).ok_or_else(|| PruneError::UnknownCheckpoint {
                checkpoint: Box::new(checkpoint.clone()),
            })?;

        if let Some(current) = self.checkpoint.clone() {
            if current == *checkpoint {
                return Ok(PruneReport {
                    checkpoint: checkpoint.clone(),
                    checkpoint_depth,
                    removed: BTreeSet::new(),
                    retained_blocks: self.blocks.len(),
                    tau_prefix_len: self.checkpoint_order_prefix.len(),
                    weighted_tau_prefix_len: self.checkpoint_weighted_order_prefix.len(),
                });
            }

            if self
                .checkpoint_depth
                .is_some_and(|current_depth| checkpoint_depth < current_depth)
            {
                return Err(PruneError::CheckpointRegression {
                    current: Box::new(current.clone()),
                    requested: Box::new(checkpoint.clone()),
                });
            }

            if !self.preceedes_or_equals(&current, checkpoint) {
                return Err(PruneError::DisconnectedCheckpoint {
                    current: Box::new(current),
                    requested: Box::new(checkpoint.clone()),
                });
            }
        }

        let mut candidates: BTreeSet<BlockIdentity> =
            self.observe(checkpoint).into_iter().collect();
        candidates.remove(checkpoint);

        let protected = protected_candidate_closure(self, checkpoint, &candidates);
        for id in &protected {
            candidates.remove(id);
        }

        for id in &candidates {
            self.blocks.remove(id);
        }

        self.checkpoint = Some(checkpoint.clone());
        self.checkpoint_depth = Some(checkpoint_depth);
        self.checkpoint_order_prefix = checkpoint_order_prefix;
        self.checkpoint_weighted_order_prefix = checkpoint_weighted_order_prefix;

        Ok(PruneReport {
            checkpoint: checkpoint.clone(),
            checkpoint_depth,
            removed: candidates,
            retained_blocks: self.blocks.len(),
            tau_prefix_len: self.checkpoint_order_prefix.len(),
            weighted_tau_prefix_len: self.checkpoint_weighted_order_prefix.len(),
        })
    }
}

/// Advance the checkpoint to the latest known final leader, if one exists.
pub fn checkpoint_after_finality<F>(
    blocklace: &mut Blocklace,
    wavelength: u64,
    n: usize,
    f: usize,
    leader_selection: F,
) -> Result<Option<PruneReport<BlockIdentity>>, PruneError<BlockIdentity>>
where
    F: Fn(u64) -> Option<NodeId> + Copy,
{
    let Some(final_leader) = latest_final_leader(blocklace, wavelength, n, f, leader_selection)
    else {
        return Ok(None);
    };

    if blocklace.checkpoint() == Some(&final_leader) {
        return Ok(None);
    }

    let checkpoint_order_prefix = tau(blocklace, wavelength, n, f, leader_selection)
        .unwrap_or_else(|_| checkpoint_order_prefix(blocklace, &final_leader));

    blocklace
        .prune_below_checkpoint_with_prefix(&final_leader, checkpoint_order_prefix, Vec::new())
        .map(Some)
}

/// Advance the checkpoint to the latest stake-weighted final leader, if one exists.
///
/// This is the f1r3node-oriented counterpart to [`checkpoint_after_finality`].
/// It stores the weighted tau prefix separately so `weighted_tau(...)` does not
/// replay a paper-native tau prefix after pruning.
pub fn checkpoint_after_weighted_finality<F>(
    blocklace: &mut Blocklace,
    wavelength: u64,
    bonds: &HashMap<NodeId, u64>,
    leader_selection: F,
) -> Result<Option<PruneReport<BlockIdentity>>, PruneError<BlockIdentity>>
where
    F: Fn(u64) -> Option<NodeId> + Copy,
{
    let Some(final_leader) =
        latest_weighted_final_leader(blocklace, wavelength, bonds, leader_selection)
    else {
        return Ok(None);
    };

    if blocklace.checkpoint() == Some(&final_leader) {
        return Ok(None);
    }

    let checkpoint_weighted_order_prefix =
        weighted_tau(blocklace, wavelength, bonds, leader_selection)
            .unwrap_or_else(|_| checkpoint_order_prefix(blocklace, &final_leader));

    blocklace
        .prune_below_checkpoint_with_prefix(
            &final_leader,
            Vec::new(),
            checkpoint_weighted_order_prefix,
        )
        .map(Some)
}

fn checkpoint_order_prefix(
    blocklace: &Blocklace,
    checkpoint: &BlockIdentity,
) -> Vec<BlockIdentity> {
    let observed_ids: std::collections::HashSet<BlockIdentity> =
        blocklace.observe(checkpoint).into_iter().collect();
    let observed_blocks = blocklace.get_set(&observed_ids);

    xsort(&observed_blocks).unwrap_or_else(|_| blocklace.observe(checkpoint).into_iter().collect())
}

fn protected_candidate_closure(
    blocklace: &Blocklace,
    checkpoint: &BlockIdentity,
    candidates: &BTreeSet<BlockIdentity>,
) -> BTreeSet<BlockIdentity> {
    let mut protected = BTreeSet::new();
    let mut stack = Vec::new();

    for (id, content) in &blocklace.blocks {
        if candidates.contains(id) || id == checkpoint {
            continue;
        }

        for pred_id in &content.predecessors {
            if candidates.contains(pred_id) && protected.insert(pred_id.clone()) {
                stack.push(pred_id.clone());
            }
        }
    }

    while let Some(id) = stack.pop() {
        let Some(content) = blocklace.content(&id) else {
            continue;
        };

        for pred_id in &content.predecessors {
            if candidates.contains(pred_id) && protected.insert(pred_id.clone()) {
                stack.push(pred_id.clone());
            }
        }
    }

    protected
}
