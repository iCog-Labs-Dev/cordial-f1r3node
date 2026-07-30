//! Equivocation evidence retention for Cordial Miners.
//!
//! Consensus validation can detect equivocation from the blocklace, but later
//! slashing needs the original conflicting blocks as cryptographic proof. This
//! module keeps that proof in the pure core crate without depending on host
//! node or wire-serialization types.

use std::collections::BTreeMap;
use std::marker::PhantomData;

use crate::block::Block;
use crate::blocklace::Blocklace;
use crate::consensus::round::depth;
use crate::consensus::validation::InvalidBlock;
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

/// In-memory evidence pool keyed first by validator, then by round.
///
/// Within each validator/round bucket, records are deduplicated by the
/// sorted identities of the conflicting blocks. This means recording the same
/// pair in the opposite order still produces one evidence record.
type EvidenceBucket<P, Id, V> = BTreeMap<Vec<Id>, EquivocationEvidence<V, P, Id>>;
type EvidenceRounds<V, P, Id> = BTreeMap<u64, EvidenceBucket<P, Id, V>>;
type EvidenceRecords<V, P, Id> = BTreeMap<V, EvidenceRounds<V, P, Id>>;

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
        self.records
            .values()
            .flat_map(|rounds| rounds.values())
            .map(|bucket| bucket.len())
            .sum()
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
        let bucket = self
            .records
            .entry(validator.clone())
            .or_default()
            .entry(round)
            .or_default();

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
            .get(validator)
            .into_iter()
            .flat_map(|rounds| rounds.values())
            .flat_map(|bucket| bucket.values().cloned())
            .collect()
    }
}

pub type CordialEquivocationEvidence = EquivocationEvidence<NodeId, Block, BlockIdentity>;
pub type CordialEvidencePool = InMemoryEvidencePool<NodeId, Block, BlockIdentity>;

/// Capture equivocation proof from a rejected block, at the moment of detection.
///
/// Equivocation is detected when a block fails validation with
/// [`InvalidBlock::Equivocation`], which names the conflicting block already
/// held locally. That instant is the *only* opportunity to retain the proof:
/// the chain axiom stops both branches from ever coexisting in the blocklace,
/// so once the incoming branch is rejected and dropped, the pair cannot be
/// reconstructed from local state afterwards — `all_equivocations` scans the
/// blocklace, which by construction holds at most one branch.
///
/// Callers should therefore invoke this with the rejected block *before*
/// discarding it:
///
/// ```ignore
/// let result = validated_insert(block.clone(), &mut blocklace, &bonds, &config);
/// if let ValidationResult::Invalid(errors) = &result {
///     record_rejected_equivocation(&block, errors, &blocklace, &mut pool);
/// }
/// ```
///
/// Returns `true` when new evidence was stored. Returns `false` when the errors
/// contain no equivocation, when the conflicting block is not in the blocklace,
/// when its round cannot be determined, or when the same proof was already held.
pub fn record_rejected_equivocation<P>(
    rejected: &Block,
    errors: &[InvalidBlock],
    blocklace: &Blocklace,
    pool: &mut P,
) -> bool
where
    P: EvidencePool<NodeId, Block, BlockIdentity>,
{
    let mut recorded = false;

    for error in errors {
        let InvalidBlock::Equivocation { conflicting } = error else {
            continue;
        };

        // The counterpart must be present locally — it is the block that caused
        // the rejection — but be defensive rather than panic on a caller that
        // passes a stale error set.
        let Some(held) = blocklace.get(conflicting) else {
            continue;
        };
        let Some(round) = depth(blocklace, conflicting) else {
            continue;
        };

        if pool.record_equivocation(
            rejected.identity.creator.clone(),
            round,
            vec![rejected.clone(), held],
        ) {
            recorded = true;
        }
    }

    recorded
}
