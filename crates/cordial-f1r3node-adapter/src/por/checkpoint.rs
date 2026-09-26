//! Optional attestation of deterministic reputation checkpoints.
//!
//! This layer can collect authenticated confirmations that multiple Cordial
//! validators calculated the same reputation block. It never decides Cordial
//! finality, validator membership, or weight activation. Every publication is
//! replayed against the receiver's completed rating round and current state,
//! and durable application repeats that audit before committing the checkpoint.

use std::{collections::BTreeMap, fmt};

use cordial_miners_core::NodeId;
use cordial_por::{
    PorConfig, PorError, ReputationBlock, ReputationState, ReputationVector, reputation_block_hash,
    verify_reputation_transition,
};
use thiserror::Error;

use super::{
    lifecycle::CompletedPorRatingRound, transport::reputation_block::ReputationBlockPublicationV1,
};

/// Default attestation threshold: signed weight must be greater than 2/3.
pub const DEFAULT_POR_CHECKPOINT_THRESHOLD_NUMERATOR: u64 = 2;

/// Denominator paired with DEFAULT_POR_CHECKPOINT_THRESHOLD_NUMERATOR.
pub const DEFAULT_POR_CHECKPOINT_THRESHOLD_DENOMINATOR: u64 = 3;

/// A strict rational threshold over snapshotted authorized-attester weight.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PorCheckpointPolicy {
    numerator: u64,
    denominator: u64,
}

impl PorCheckpointPolicy {
    pub fn new(numerator: u64, denominator: u64) -> Result<Self, PorCheckpointError> {
        if numerator == 0 || denominator == 0 || numerator >= denominator {
            return Err(PorCheckpointError::InvalidThreshold {
                numerator,
                denominator,
            });
        }

        Ok(Self {
            numerator,
            denominator,
        })
    }

    pub fn numerator(&self) -> u64 {
        self.numerator
    }

    pub fn denominator(&self) -> u64 {
        self.denominator
    }
}

impl Default for PorCheckpointPolicy {
    fn default() -> Self {
        Self {
            numerator: DEFAULT_POR_CHECKPOINT_THRESHOLD_NUMERATOR,
            denominator: DEFAULT_POR_CHECKPOINT_THRESHOLD_DENOMINATOR,
        }
    }
}

/// Observable weighted publication progress for one audited block candidate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PorCheckpointProgress {
    pub authorized_attesters: usize,
    pub attesters: Vec<NodeId>,
    pub candidate_hash: Option<[u8; 32]>,
    pub total_attester_weight: u128,
    pub signed_weight: u128,
    pub required_weight: u128,
}

impl PorCheckpointProgress {
    pub fn is_reached(&self) -> bool {
        self.signed_weight >= self.required_weight
    }
}

/// Whether an authenticated publication changed attestation weight.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PorCheckpointObservation {
    Counted,
    Duplicate,
}

/// A replay-audited checkpoint accompanied by weighted signed attestations.
///
/// Fields are private so this value can only be produced by a successful
/// PorCheckpointCollector. Durable application still replays
/// the transition against current state to reject stale certificates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttestedPorCheckpoint {
    block: ReputationBlock,
    block_hash: [u8; 32],
    publications: Vec<ReputationBlockPublicationV1>,
    progress: PorCheckpointProgress,
}

impl AttestedPorCheckpoint {
    pub fn block(&self) -> &ReputationBlock {
        &self.block
    }

    pub fn block_hash(&self) -> [u8; 32] {
        self.block_hash
    }

    pub fn publications(&self) -> &[ReputationBlockPublicationV1] {
        &self.publications
    }

    pub fn progress(&self) -> &PorCheckpointProgress {
        &self.progress
    }

    pub fn attesters(&self) -> impl Iterator<Item = &NodeId> {
        self.publications
            .iter()
            .map(ReputationBlockPublicationV1::publisher)
    }
}

/// Two different authenticated block publications from the same attester.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PorCheckpointConflictEvidence {
    attester: NodeId,
    first_hash: [u8; 32],
    conflicting_hash: [u8; 32],
    first_publication: ReputationBlockPublicationV1,
    conflicting_publication: ReputationBlockPublicationV1,
}

impl PorCheckpointConflictEvidence {
    pub fn attester(&self) -> &NodeId {
        &self.attester
    }

    pub fn first_hash(&self) -> [u8; 32] {
        self.first_hash
    }

    pub fn conflicting_hash(&self) -> [u8; 32] {
        self.conflicting_hash
    }

    pub fn first_publication(&self) -> &ReputationBlockPublicationV1 {
        &self.first_publication
    }

    pub fn conflicting_publication(&self) -> &ReputationBlockPublicationV1 {
        &self.conflicting_publication
    }
}

impl fmt::Display for PorCheckpointConflictEvidence {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "PoR reputation-block attester {:?} signed conflicting blocks {:?} and {:?}",
            self.attester, self.first_hash, self.conflicting_hash
        )
    }
}

/// Failures while configuring, collecting, or finalizing checkpoint attestations.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum PorCheckpointError {
    #[error(
        "PoR checkpoint threshold must satisfy 0 < numerator < denominator, got {numerator}/{denominator}"
    )]
    InvalidThreshold { numerator: u64, denominator: u64 },

    #[error("PoR checkpoint attestation requires at least one authorized attester")]
    EmptyAttesterSet,

    #[error("unknown authorized PoR checkpoint attester {0:?}")]
    UnknownAttester(NodeId),

    #[error("excluded authorized PoR checkpoint attester {0:?}")]
    ExcludedAttester(NodeId),

    #[error("PoR checkpoint attesters have zero total reputation weight")]
    ZeroAttesterWeight,

    #[error("PoR checkpoint attestation weight arithmetic overflowed")]
    WeightOverflow,

    #[error("unauthorized PoR checkpoint attester {0:?}")]
    UnauthorizedAttester(NodeId),

    #[error("{0}")]
    ConflictingPublication(Box<PorCheckpointConflictEvidence>),

    #[error(
        "deterministic PoR audit accepted competing block candidates {first_hash:?} and {competing_hash:?}"
    )]
    CompetingAuditedBlock {
        first_hash: [u8; 32],
        competing_hash: [u8; 32],
    },

    #[error("PoR reputation-block transition audit failed: {0}")]
    Audit(#[from] PorError),

    #[error("PoR checkpoint attestation threshold was already reached")]
    ThresholdAlreadyReached,

    #[error(
        "PoR checkpoint attestation has weight {signed_weight}, but requires {required_weight}"
    )]
    ThresholdNotReached {
        signed_weight: u128,
        required_weight: u128,
    },
}

/// Collects distinct replay-valid publications for one completed rating round.
pub struct PorCheckpointCollector<'a> {
    state: &'a ReputationState,
    completed: &'a CompletedPorRatingRound,
    config: &'a PorConfig,
    shard_id: &'a [u8],
    attester_weights: BTreeMap<NodeId, u64>,
    total_attester_weight: u128,
    required_weight: u128,
    signed_weight: u128,
    candidate_hash: Option<[u8; 32]>,
    candidate_block: Option<ReputationBlock>,
    publications: BTreeMap<NodeId, ReputationBlockPublicationV1>,
}

impl<'a> PorCheckpointCollector<'a> {
    /// Snapshot an explicit attester set against the current reputation state.
    ///
    /// Cordial supplies this set from its existing authorized validators. PoR
    /// does not select a committee or alter validator membership. The weights
    /// are snapshotted from the preceding committed reputation state and are
    /// used only to summarize checkpoint confirmations.
    pub fn new(
        state: &'a ReputationState,
        completed: &'a CompletedPorRatingRound,
        config: &'a PorConfig,
        shard_id: &'a [u8],
        authorized_attesters: impl IntoIterator<Item = NodeId>,
        policy: PorCheckpointPolicy,
    ) -> Result<Self, PorCheckpointError> {
        let mut attester_weights = BTreeMap::new();
        let mut total_attester_weight = 0u128;

        for attester in authorized_attesters {
            let index = state
                .reputation_list()
                .entries
                .binary_search_by(|entry| entry.node_id.cmp(&attester))
                .map_err(|_| PorCheckpointError::UnknownAttester(attester.clone()))?;
            let entry = &state.reputation_list().entries[index];
            if entry.is_excluded || state.is_ejected(&attester) {
                return Err(PorCheckpointError::ExcludedAttester(attester));
            }

            if attester_weights.contains_key(&attester) {
                continue;
            }
            total_attester_weight = total_attester_weight
                .checked_add(u128::from(entry.reputation))
                .ok_or(PorCheckpointError::WeightOverflow)?;
            attester_weights.insert(attester, entry.reputation);
        }

        if attester_weights.is_empty() {
            return Err(PorCheckpointError::EmptyAttesterSet);
        }
        if total_attester_weight == 0 {
            return Err(PorCheckpointError::ZeroAttesterWeight);
        }

        let required_weight =
            strict_required_weight(total_attester_weight, policy.numerator, policy.denominator)
                .ok_or(PorCheckpointError::WeightOverflow)?;

        Ok(Self {
            state,
            completed,
            config,
            shard_id,
            attester_weights,
            total_attester_weight,
            required_weight,
            signed_weight: 0,
            candidate_hash: None,
            candidate_block: None,
            publications: BTreeMap::new(),
        })
    }

    pub fn progress(&self) -> PorCheckpointProgress {
        PorCheckpointProgress {
            authorized_attesters: self.attester_weights.len(),
            attesters: self.publications.keys().cloned().collect(),
            candidate_hash: self.candidate_hash,
            total_attester_weight: self.total_attester_weight,
            signed_weight: self.signed_weight,
            required_weight: self.required_weight,
        }
    }

    /// Audit and count one authenticated publication atomically.
    pub fn observe(
        &mut self,
        publication: ReputationBlockPublicationV1,
    ) -> Result<PorCheckpointObservation, PorCheckpointError> {
        if self.progress().is_reached() {
            return Err(PorCheckpointError::ThresholdAlreadyReached);
        }

        let attester = publication.publisher().clone();
        let Some(weight) = self.attester_weights.get(&attester).copied() else {
            return Err(PorCheckpointError::UnauthorizedAttester(attester));
        };
        let block_hash = reputation_block_hash(publication.block())?;

        if let Some(previous) = self.publications.get(&attester) {
            let previous_hash = reputation_block_hash(previous.block())?;
            if previous_hash == block_hash {
                return Ok(PorCheckpointObservation::Duplicate);
            }
            return Err(PorCheckpointError::ConflictingPublication(Box::new(
                PorCheckpointConflictEvidence {
                    attester,
                    first_hash: previous_hash,
                    conflicting_hash: block_hash,
                    first_publication: previous.clone(),
                    conflicting_publication: publication,
                },
            )));
        }

        self.audit(publication.block())?;
        if let Some(first_hash) = self.candidate_hash
            && first_hash != block_hash
        {
            return Err(PorCheckpointError::CompetingAuditedBlock {
                first_hash,
                competing_hash: block_hash,
            });
        }

        let signed_weight = self
            .signed_weight
            .checked_add(u128::from(weight))
            .ok_or(PorCheckpointError::WeightOverflow)?;

        if self.candidate_hash.is_none() {
            self.candidate_hash = Some(block_hash);
            self.candidate_block = Some(publication.block().clone());
        }
        self.publications.insert(attester, publication);
        self.signed_weight = signed_weight;

        Ok(PorCheckpointObservation::Counted)
    }

    /// Consume the collector after its strict attestation threshold is reached.
    pub fn into_attested(self) -> Result<AttestedPorCheckpoint, PorCheckpointError> {
        let progress = self.progress();
        if !progress.is_reached() {
            return Err(PorCheckpointError::ThresholdNotReached {
                signed_weight: progress.signed_weight,
                required_weight: progress.required_weight,
            });
        }

        Ok(AttestedPorCheckpoint {
            block: self
                .candidate_block
                .expect("reached attestation threshold always has a candidate block"),
            block_hash: self
                .candidate_hash
                .expect("reached attestation threshold always has a candidate hash"),
            publications: self.publications.into_values().collect(),
            progress,
        })
    }

    fn audit(&self, block: &ReputationBlock) -> Result<(), PorError> {
        let previous = ReputationVector {
            round: self.state.round(),
            values: self.state.reputation_list().entries.clone(),
        };
        verify_reputation_transition(
            &previous,
            &self.completed.batch().ratings,
            block,
            cordial_por::ReputationBlockContext {
                shard_id: self.shard_id,
                source_finalized_wave: self.completed.opened().finalized_wave,
                previous_block: self.state.latest_block(),
            },
            self.config,
        )
    }
}

fn strict_required_weight(total_weight: u128, numerator: u64, denominator: u64) -> Option<u128> {
    let numerator = u128::from(numerator);
    let denominator = u128::from(denominator);
    let whole = (total_weight / denominator).checked_mul(numerator)?;
    let remainder = (total_weight % denominator).checked_mul(numerator)? / denominator;

    whole.checked_add(remainder)?.checked_add(1)
}
