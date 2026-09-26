//! Weighted admission of authenticated reputation-block publications.
//!
//! This layer turns individually authenticated publications into a local quorum
//! certificate. A publication earns weight only after deterministic transition
//! replay against the receiver's completed rating round and current state.
//! Admission itself is read-only; durable state owns the eventual commit.

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

/// Default strict publication quorum: signed weight must be greater than 2/3.
pub const DEFAULT_REPUTATION_BLOCK_QUORUM_NUMERATOR: u64 = 2;

/// Denominator paired with DEFAULT_REPUTATION_BLOCK_QUORUM_NUMERATOR.
pub const DEFAULT_REPUTATION_BLOCK_QUORUM_DENOMINATOR: u64 = 3;

/// A strict rational threshold over the snapshotted eligible publisher weight.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PorReputationBlockQuorumPolicy {
    numerator: u64,
    denominator: u64,
}

impl PorReputationBlockQuorumPolicy {
    pub fn new(numerator: u64, denominator: u64) -> Result<Self, PorReputationBlockAdmissionError> {
        if numerator == 0 || denominator == 0 || numerator >= denominator {
            return Err(PorReputationBlockAdmissionError::InvalidThreshold {
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

impl Default for PorReputationBlockQuorumPolicy {
    fn default() -> Self {
        Self {
            numerator: DEFAULT_REPUTATION_BLOCK_QUORUM_NUMERATOR,
            denominator: DEFAULT_REPUTATION_BLOCK_QUORUM_DENOMINATOR,
        }
    }
}

/// Observable weighted publication progress for one audited block candidate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PorReputationBlockAdmissionProgress {
    pub eligible_publishers: usize,
    pub publishers: Vec<NodeId>,
    pub candidate_hash: Option<[u8; 32]>,
    pub total_eligible_weight: u128,
    pub signed_weight: u128,
    pub required_weight: u128,
}

impl PorReputationBlockAdmissionProgress {
    pub fn is_reached(&self) -> bool {
        self.signed_weight >= self.required_weight
    }
}

/// Whether an authenticated publication changed admission weight.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PorReputationBlockObservation {
    Counted,
    Duplicate,
}

/// A replay-audited block accompanied by a weighted signed quorum certificate.
///
/// Fields are private so this value can only be produced by a successful
/// PorReputationBlockAdmissionCoordinator. Durable application still replays
/// the transition against current state to reject stale certificates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdmittedPorReputationBlock {
    block: ReputationBlock,
    block_hash: [u8; 32],
    publications: Vec<ReputationBlockPublicationV1>,
    progress: PorReputationBlockAdmissionProgress,
}

impl AdmittedPorReputationBlock {
    pub fn block(&self) -> &ReputationBlock {
        &self.block
    }

    pub fn block_hash(&self) -> [u8; 32] {
        self.block_hash
    }

    pub fn publications(&self) -> &[ReputationBlockPublicationV1] {
        &self.publications
    }

    pub fn progress(&self) -> &PorReputationBlockAdmissionProgress {
        &self.progress
    }

    pub fn publishers(&self) -> impl Iterator<Item = &NodeId> {
        self.publications
            .iter()
            .map(ReputationBlockPublicationV1::publisher)
    }
}

/// Two different authenticated block publications from the same publisher.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PorReputationBlockConflictEvidence {
    publisher: NodeId,
    first_hash: [u8; 32],
    conflicting_hash: [u8; 32],
    first_publication: ReputationBlockPublicationV1,
    conflicting_publication: ReputationBlockPublicationV1,
}

impl PorReputationBlockConflictEvidence {
    pub fn publisher(&self) -> &NodeId {
        &self.publisher
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

impl fmt::Display for PorReputationBlockConflictEvidence {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "PoR reputation-block publisher {:?} signed conflicting blocks {:?} and {:?}",
            self.publisher, self.first_hash, self.conflicting_hash
        )
    }
}

/// Failures while configuring, collecting, or finalizing block admission.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum PorReputationBlockAdmissionError {
    #[error(
        "PoR reputation-block quorum must satisfy 0 < numerator < denominator, got {numerator}/{denominator}"
    )]
    InvalidThreshold { numerator: u64, denominator: u64 },

    #[error("PoR reputation-block admission requires at least one eligible publisher")]
    EmptyEligiblePublisherSet,

    #[error("unknown eligible PoR reputation-block publisher {0:?}")]
    UnknownEligiblePublisher(NodeId),

    #[error("excluded eligible PoR reputation-block publisher {0:?}")]
    ExcludedEligiblePublisher(NodeId),

    #[error("PoR reputation-block eligible publishers have zero total reputation weight")]
    ZeroEligibleWeight,

    #[error("PoR reputation-block quorum weight arithmetic overflowed")]
    WeightOverflow,

    #[error("unexpected PoR reputation-block publisher {0:?}")]
    UnexpectedPublisher(NodeId),

    #[error("{0}")]
    ConflictingPublication(Box<PorReputationBlockConflictEvidence>),

    #[error(
        "deterministic PoR audit accepted competing block candidates {first_hash:?} and {competing_hash:?}"
    )]
    CompetingAuditedBlock {
        first_hash: [u8; 32],
        competing_hash: [u8; 32],
    },

    #[error("PoR reputation-block transition audit failed: {0}")]
    Audit(#[from] PorError),

    #[error("PoR reputation-block publication quorum was already reached")]
    QuorumAlreadyReached,

    #[error(
        "PoR reputation-block publication quorum has weight {signed_weight}, but requires {required_weight}"
    )]
    QuorumNotReached {
        signed_weight: u128,
        required_weight: u128,
    },
}

/// Collects distinct replay-valid publications for one completed rating round.
pub struct PorReputationBlockAdmissionCoordinator<'a> {
    state: &'a ReputationState,
    completed: &'a CompletedPorRatingRound,
    config: &'a PorConfig,
    shard_id: &'a [u8],
    eligible_weights: BTreeMap<NodeId, u64>,
    total_eligible_weight: u128,
    required_weight: u128,
    signed_weight: u128,
    candidate_hash: Option<[u8; 32]>,
    candidate_block: Option<ReputationBlock>,
    publications: BTreeMap<NodeId, ReputationBlockPublicationV1>,
}

impl<'a> PorReputationBlockAdmissionCoordinator<'a> {
    /// Snapshot an explicit publisher set against the current reputation state.
    ///
    /// Passing the eligible set explicitly keeps committee selection outside
    /// this module. Until a committee selector is wired, callers may pass all
    /// active validators; later they can pass the selected consensus group
    /// without changing admission semantics.
    pub fn new(
        state: &'a ReputationState,
        completed: &'a CompletedPorRatingRound,
        config: &'a PorConfig,
        shard_id: &'a [u8],
        eligible_publishers: impl IntoIterator<Item = NodeId>,
        policy: PorReputationBlockQuorumPolicy,
    ) -> Result<Self, PorReputationBlockAdmissionError> {
        let mut eligible_weights = BTreeMap::new();
        let mut total_eligible_weight = 0u128;

        for publisher in eligible_publishers {
            let index = state
                .reputation_list()
                .entries
                .binary_search_by(|entry| entry.node_id.cmp(&publisher))
                .map_err(|_| {
                    PorReputationBlockAdmissionError::UnknownEligiblePublisher(publisher.clone())
                })?;
            let entry = &state.reputation_list().entries[index];
            if entry.is_excluded || state.is_ejected(&publisher) {
                return Err(PorReputationBlockAdmissionError::ExcludedEligiblePublisher(
                    publisher,
                ));
            }

            if eligible_weights.contains_key(&publisher) {
                continue;
            }
            total_eligible_weight = total_eligible_weight
                .checked_add(u128::from(entry.reputation))
                .ok_or(PorReputationBlockAdmissionError::WeightOverflow)?;
            eligible_weights.insert(publisher, entry.reputation);
        }

        if eligible_weights.is_empty() {
            return Err(PorReputationBlockAdmissionError::EmptyEligiblePublisherSet);
        }
        if total_eligible_weight == 0 {
            return Err(PorReputationBlockAdmissionError::ZeroEligibleWeight);
        }

        let required_weight =
            strict_required_weight(total_eligible_weight, policy.numerator, policy.denominator)
                .ok_or(PorReputationBlockAdmissionError::WeightOverflow)?;

        Ok(Self {
            state,
            completed,
            config,
            shard_id,
            eligible_weights,
            total_eligible_weight,
            required_weight,
            signed_weight: 0,
            candidate_hash: None,
            candidate_block: None,
            publications: BTreeMap::new(),
        })
    }

    pub fn progress(&self) -> PorReputationBlockAdmissionProgress {
        PorReputationBlockAdmissionProgress {
            eligible_publishers: self.eligible_weights.len(),
            publishers: self.publications.keys().cloned().collect(),
            candidate_hash: self.candidate_hash,
            total_eligible_weight: self.total_eligible_weight,
            signed_weight: self.signed_weight,
            required_weight: self.required_weight,
        }
    }

    /// Audit and count one authenticated publication atomically.
    pub fn observe(
        &mut self,
        publication: ReputationBlockPublicationV1,
    ) -> Result<PorReputationBlockObservation, PorReputationBlockAdmissionError> {
        if self.progress().is_reached() {
            return Err(PorReputationBlockAdmissionError::QuorumAlreadyReached);
        }

        let publisher = publication.publisher().clone();
        let Some(weight) = self.eligible_weights.get(&publisher).copied() else {
            return Err(PorReputationBlockAdmissionError::UnexpectedPublisher(
                publisher,
            ));
        };
        let block_hash = reputation_block_hash(publication.block())?;

        if let Some(previous) = self.publications.get(&publisher) {
            let previous_hash = reputation_block_hash(previous.block())?;
            if previous_hash == block_hash {
                return Ok(PorReputationBlockObservation::Duplicate);
            }
            return Err(PorReputationBlockAdmissionError::ConflictingPublication(
                Box::new(PorReputationBlockConflictEvidence {
                    publisher,
                    first_hash: previous_hash,
                    conflicting_hash: block_hash,
                    first_publication: previous.clone(),
                    conflicting_publication: publication,
                }),
            ));
        }

        self.audit(publication.block())?;
        if let Some(first_hash) = self.candidate_hash
            && first_hash != block_hash
        {
            return Err(PorReputationBlockAdmissionError::CompetingAuditedBlock {
                first_hash,
                competing_hash: block_hash,
            });
        }

        let signed_weight = self
            .signed_weight
            .checked_add(u128::from(weight))
            .ok_or(PorReputationBlockAdmissionError::WeightOverflow)?;

        if self.candidate_hash.is_none() {
            self.candidate_hash = Some(block_hash);
            self.candidate_block = Some(publication.block().clone());
        }
        self.publications.insert(publisher, publication);
        self.signed_weight = signed_weight;

        Ok(PorReputationBlockObservation::Counted)
    }

    /// Consume a coordinator after its strict weighted quorum has been reached.
    pub fn into_admitted(
        self,
    ) -> Result<AdmittedPorReputationBlock, PorReputationBlockAdmissionError> {
        let progress = self.progress();
        if !progress.is_reached() {
            return Err(PorReputationBlockAdmissionError::QuorumNotReached {
                signed_weight: progress.signed_weight,
                required_weight: progress.required_weight,
            });
        }

        Ok(AdmittedPorReputationBlock {
            block: self
                .candidate_block
                .expect("reached publication quorum always has a candidate block"),
            block_hash: self
                .candidate_hash
                .expect("reached publication quorum always has a candidate hash"),
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
