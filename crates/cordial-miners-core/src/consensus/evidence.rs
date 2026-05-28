//! Equivocation evidence retention for Cordial Miners.
//!
//! Consensus validation can detect equivocation from the blocklace, but later
//! slashing needs the original conflicting blocks as cryptographic proof. This
//! module keeps that proof in the pure core crate without depending on host
//! node or wire-serialization types.

use std::collections::BTreeMap;
use std::marker::PhantomData;

use crate::block::Block;
use crate::types::{BlockIdentity, NodeId};

/// A block-like value that can expose a stable identity for evidence
/// deduplication and deterministic ordering.
pub trait EvidenceBlock<Id> {
    fn evidence_id(&self) -> Id;
}

impl EvidenceBlock<BlockIdentity> for Block {
    fn evidence_id(&self) -> BlockIdentity {
        self.identity.clone()
    }
}

/// Raw proof that one validator created conflicting blocks in one round.
///
/// `P` is intentionally generic so the core can retain the host's native block
/// object without knowing how that host serializes it. In this crate, the
/// concrete Cordial block type is [`Block`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EquivocationEvidence<V, P, Id> {
    pub validator: V,
    pub round: u64,
    pub blocks: Vec<P>,
    _identity: PhantomData<fn() -> Id>,
}

impl<V, P, Id> EquivocationEvidence<V, P, Id> {
    pub fn new(validator: V, round: u64, blocks: Vec<P>) -> Self {
        Self {
            validator,
            round,
            blocks,
            _identity: PhantomData,
        }
    }
}

/// Storage interface for retaining equivocation proof.
pub trait EvidencePool<V, P, Id> {
    /// Record one conflicting block set for `(validator, round)`.
    ///
    /// Returns `true` when a new evidence record was inserted and `false` when
    /// the evidence was already present or fewer than two distinct blocks were
    /// supplied.
    fn record_equivocation<I>(&mut self, validator: V, round: u64, blocks: I) -> bool
    where
        I: IntoIterator<Item = P>;

    /// Return all evidence known for a validator in deterministic order.
    fn evidence_for(&self, validator: &V) -> Vec<EquivocationEvidence<V, P, Id>>;
}

/// In-memory evidence pool keyed first by `(validator, round)`.
///
/// Within each `(validator, round)` bucket, records are deduplicated by the
/// sorted identities of the conflicting blocks. This means recording the same
/// pair in the opposite order still produces one evidence record.
type EvidenceBucket<P, Id, V> = BTreeMap<Vec<Id>, EquivocationEvidence<V, P, Id>>;
type EvidenceRecords<V, P, Id> = BTreeMap<(V, u64), EvidenceBucket<P, Id, V>>;

#[derive(Debug, Clone)]
pub struct InMemoryEvidencePool<V, P, Id> {
    records: EvidenceRecords<V, P, Id>,
}

impl<V, P, Id> Default for InMemoryEvidencePool<V, P, Id> {
    fn default() -> Self {
        Self {
            records: BTreeMap::new(),
        }
    }
}

impl<V, P, Id> InMemoryEvidencePool<V, P, Id> {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.records.values().map(|bucket| bucket.len()).sum()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl<V, P, Id> EvidencePool<V, P, Id> for InMemoryEvidencePool<V, P, Id>
where
    V: Ord + Clone,
    P: EvidenceBlock<Id> + Clone,
    Id: Ord + Clone,
{
    fn record_equivocation<I>(&mut self, validator: V, round: u64, blocks: I) -> bool
    where
        I: IntoIterator<Item = P>,
    {
        let mut unique_blocks = BTreeMap::<Id, P>::new();
        for block in blocks {
            unique_blocks.entry(block.evidence_id()).or_insert(block);
        }

        if unique_blocks.len() < 2 {
            return false;
        }

        let evidence_key: Vec<Id> = unique_blocks.keys().cloned().collect();
        let evidence_blocks: Vec<P> = unique_blocks.into_values().collect();
        let bucket = self.records.entry((validator.clone(), round)).or_default();

        if bucket.contains_key(&evidence_key) {
            return false;
        }

        bucket.insert(
            evidence_key,
            EquivocationEvidence::new(validator, round, evidence_blocks),
        );
        true
    }

    fn evidence_for(&self, validator: &V) -> Vec<EquivocationEvidence<V, P, Id>> {
        self.records
            .iter()
            .filter(|((record_validator, _), _)| record_validator == validator)
            .flat_map(|(_, bucket)| bucket.values().cloned())
            .collect()
    }
}

pub type CordialEquivocationEvidence = EquivocationEvidence<NodeId, Block, BlockIdentity>;
pub type CordialEvidencePool = InMemoryEvidencePool<NodeId, Block, BlockIdentity>;
