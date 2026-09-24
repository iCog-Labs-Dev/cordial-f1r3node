//! Lifecycle coordination for one finalized PoR block-production rating round.
//!
//! The coordinator connects local rating production, resumable outbound
//! delivery, inbound evidence-backed collection, and explicit closure. It does
//! not use wall-clock deadlines: `close_if_quorum` applies deterministic
//! complete-rater weight, while `close_at_finalized_wave` supplies a finality-
//! driven fallback.

pub mod cutoff;
pub mod quorum;

use std::fmt;

use cordial_miners_core::{Blocklace, NodeId};
use cordial_por::{PorConfig, RatingBatch, ReputationState};

use crate::ordered_output::OrderedFinalizedOutput;

use self::{
    cutoff::{PorRatingCutoffError, PorRatingRoundCutoffPolicy},
    quorum::{PorRatingQuorumError, PorRatingQuorumProgress, PorRatingRoundClosurePolicy},
};
use super::{
    collector::{BlockProductionRatingCollector, PorRatingCollectorError},
    finality::FinalizedRatingRound,
    ratings::{PorRatingError, build_finalized_block_production_rating_batch},
    transport::{
        PorRatingTransportError, RatingEnvelopeBroadcaster, encode_rating_batch,
        receive_rating_envelope,
    },
};

/// Observable lifecycle state of a rating-round coordinator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PorRatingRoundStatus {
    Open,
    Closed,
}

/// Failures while coordinating one local PoR rating round.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PorRatingRoundError {
    Rating(PorRatingError),
    Collector(PorRatingCollectorError),
    Transport(PorRatingTransportError),
    Quorum(PorRatingQuorumError),
    Cutoff(PorRatingCutoffError),
    LocalBatchAlreadyProduced,
    LocalBatchNotProduced,
    PendingOutboundRatings(usize),
    RoundNotClosed,
    RoundClosed,
    QuorumNotReached {
        completed_weight: u128,
        required_weight: u128,
    },
    Broadcast {
        delivered: usize,
        remaining: usize,
        message: String,
    },
}

impl fmt::Display for PorRatingRoundError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Rating(error) => error.fmt(f),
            Self::Collector(error) => error.fmt(f),
            Self::Transport(error) => error.fmt(f),
            Self::Quorum(error) => error.fmt(f),
            Self::Cutoff(error) => error.fmt(f),
            Self::LocalBatchAlreadyProduced => {
                write!(f, "local PoR rating batch was already produced")
            }
            Self::LocalBatchNotProduced => {
                write!(f, "local PoR rating batch has not been produced")
            }
            Self::PendingOutboundRatings(remaining) => write!(
                f,
                "cannot close PoR rating round with {remaining} pending outbound ratings"
            ),
            Self::RoundNotClosed => write!(f, "PoR rating round is still open"),
            Self::RoundClosed => write!(f, "PoR rating round is already closed"),
            Self::QuorumNotReached {
                completed_weight,
                required_weight,
            } => write!(
                f,
                "PoR rating quorum has weight {completed_weight}, but requires {required_weight}"
            ),
            Self::Broadcast {
                delivered,
                remaining,
                message,
            } => write!(
                f,
                "PoR rating broadcast failed after {delivered} new deliveries with {remaining} remaining: {message}"
            ),
        }
    }
}

impl std::error::Error for PorRatingRoundError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Rating(error) => Some(error),
            Self::Collector(error) => Some(error),
            Self::Transport(error) => Some(error),
            Self::Quorum(error) => Some(error),
            Self::Cutoff(error) => Some(error),
            _ => None,
        }
    }
}

impl From<PorRatingError> for PorRatingRoundError {
    fn from(error: PorRatingError) -> Self {
        Self::Rating(error)
    }
}

impl From<PorRatingCollectorError> for PorRatingRoundError {
    fn from(error: PorRatingCollectorError) -> Self {
        Self::Collector(error)
    }
}

impl From<PorRatingTransportError> for PorRatingRoundError {
    fn from(error: PorRatingTransportError) -> Self {
        Self::Transport(error)
    }
}

impl From<PorRatingQuorumError> for PorRatingRoundError {
    fn from(error: PorRatingQuorumError) -> Self {
        Self::Quorum(error)
    }
}

impl From<PorRatingCutoffError> for PorRatingRoundError {
    fn from(error: PorRatingCutoffError) -> Self {
        Self::Cutoff(error)
    }
}

/// Deterministic trigger that closed a PoR rating round.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PorRatingRoundCloseReason {
    Quorum {
        completed_weight: u128,
        required_weight: u128,
    },
    FinalizedWaveCutoff {
        observed_finalized_wave: u64,
        required_finalized_wave: u64,
    },
}

/// Owned output of a deterministically closed PoR rating round.
///
/// Consuming a coordinator into this value releases its immutable borrow of
/// the previous reputation state, allowing the completed batch to drive the
/// next atomic reputation-state transition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompletedPorRatingRound {
    opened: FinalizedRatingRound,
    batch: RatingBatch,
    close_reason: PorRatingRoundCloseReason,
}

impl CompletedPorRatingRound {
    pub fn opened(&self) -> FinalizedRatingRound {
        self.opened
    }

    pub fn batch(&self) -> &RatingBatch {
        &self.batch
    }

    pub fn close_reason(&self) -> PorRatingRoundCloseReason {
        self.close_reason
    }
}

/// Coordinates one local validator's view of an opened PoR rating round.
pub struct PorRatingRoundCoordinator<'a> {
    blocklace: &'a Blocklace,
    output: &'a OrderedFinalizedOutput,
    opened: FinalizedRatingRound,
    state: &'a ReputationState,
    config: &'a PorConfig,
    collector: Option<BlockProductionRatingCollector<'a>>,
    local_batch: Option<RatingBatch>,
    outbound_envelopes: Vec<Vec<u8>>,
    next_outbound: usize,
    completed_batch: Option<RatingBatch>,
    close_reason: Option<PorRatingRoundCloseReason>,
}

impl<'a> PorRatingRoundCoordinator<'a> {
    pub fn new(
        blocklace: &'a Blocklace,
        output: &'a OrderedFinalizedOutput,
        opened: FinalizedRatingRound,
        state: &'a ReputationState,
        config: &'a PorConfig,
    ) -> Result<Self, PorRatingRoundError> {
        let collector =
            BlockProductionRatingCollector::new(blocklace, output, opened, state, config)?;

        Ok(Self {
            blocklace,
            output,
            opened,
            state,
            config,
            collector: Some(collector),
            local_batch: None,
            outbound_envelopes: Vec::new(),
            next_outbound: 0,
            completed_batch: None,
            close_reason: None,
        })
    }

    pub fn status(&self) -> PorRatingRoundStatus {
        if self.completed_batch.is_some() {
            PorRatingRoundStatus::Closed
        } else {
            PorRatingRoundStatus::Open
        }
    }

    pub fn opened(&self) -> FinalizedRatingRound {
        self.opened
    }

    pub fn local_batch(&self) -> Option<&RatingBatch> {
        self.local_batch.as_ref()
    }

    pub fn completed_batch(&self) -> Option<&RatingBatch> {
        self.completed_batch.as_ref()
    }

    pub fn close_reason(&self) -> Option<PorRatingRoundCloseReason> {
        self.close_reason
    }

    /// Consume a closed coordinator and return its owned finalized output.
    ///
    /// Open coordinators are rejected so callers cannot bypass quorum or the
    /// deterministic finalized-wave cutoff.
    pub fn into_completed(self) -> Result<CompletedPorRatingRound, PorRatingRoundError> {
        let batch = self
            .completed_batch
            .ok_or(PorRatingRoundError::RoundNotClosed)?;
        let close_reason = self
            .close_reason
            .expect("a completed coordinator always records its close reason");

        Ok(CompletedPorRatingRound {
            opened: self.opened,
            batch,
            close_reason,
        })
    }

    pub fn collected_len(&self) -> usize {
        match (&self.collector, &self.completed_batch) {
            (Some(collector), _) => collector.len(),
            (None, Some(batch)) => batch.ratings.len(),
            (None, None) => 0,
        }
    }

    pub fn pending_outbound(&self) -> usize {
        self.outbound_envelopes
            .len()
            .saturating_sub(self.next_outbound)
    }

    /// Evaluate deterministic complete-rater participation while the round is open.
    pub fn quorum_progress(
        &self,
        policy: &PorRatingRoundClosurePolicy,
    ) -> Result<PorRatingQuorumProgress, PorRatingRoundError> {
        self.require_open()?;
        Ok(policy.evaluate(
            self.collector
                .as_ref()
                .expect("open coordinator always has a collector"),
        )?)
    }

    /// Build, pre-encode, and locally collect this validator's rating batch.
    ///
    /// All fallible preparation happens before coordinator state is mutated.
    pub fn produce_local_batch(
        &mut self,
        rater: &NodeId,
        private_key: &[u8],
    ) -> Result<&RatingBatch, PorRatingRoundError> {
        self.require_open()?;
        if self.local_batch.is_some() {
            return Err(PorRatingRoundError::LocalBatchAlreadyProduced);
        }

        let batch = build_finalized_block_production_rating_batch(
            self.blocklace,
            self.output,
            self.opened,
            rater,
            self.state,
            self.config,
            private_key,
        )?;
        let outbound = encode_rating_batch(self.opened.finalized_wave, &batch)?;
        self.collector
            .as_mut()
            .expect("open coordinator always has a collector")
            .insert_batch(batch.clone())?;

        self.outbound_envelopes = outbound;
        self.next_outbound = 0;
        self.local_batch = Some(batch);
        Ok(self
            .local_batch
            .as_ref()
            .expect("local batch was just stored"))
    }

    /// Broadcast all envelopes not delivered by an earlier attempt.
    pub fn broadcast_pending(
        &mut self,
        broadcaster: &impl RatingEnvelopeBroadcaster,
    ) -> Result<usize, PorRatingRoundError> {
        self.require_open()?;
        if self.local_batch.is_none() {
            return Err(PorRatingRoundError::LocalBatchNotProduced);
        }

        let starting_index = self.next_outbound;
        while self.next_outbound < self.outbound_envelopes.len() {
            let envelope = &self.outbound_envelopes[self.next_outbound];
            if let Err(message) = broadcaster.broadcast_rating_envelope(envelope) {
                return Err(PorRatingRoundError::Broadcast {
                    delivered: self.next_outbound - starting_index,
                    remaining: self.pending_outbound(),
                    message,
                });
            }
            self.next_outbound += 1;
        }

        Ok(self.next_outbound - starting_index)
    }

    /// Decode and collect one inbound block-production rating envelope.
    pub fn receive_envelope(&mut self, encoded: &[u8]) -> Result<(), PorRatingRoundError> {
        self.require_open()?;
        receive_rating_envelope(
            self.collector
                .as_mut()
                .expect("open coordinator always has a collector"),
            encoded,
        )?;
        Ok(())
    }

    /// Close only after complete raters hold the policy's required weight.
    pub fn close_if_quorum(
        &mut self,
        policy: &PorRatingRoundClosurePolicy,
    ) -> Result<&RatingBatch, PorRatingRoundError> {
        self.require_close_prerequisites()?;
        let progress = self.quorum_progress(policy)?;
        if !progress.is_reached() {
            return Err(PorRatingRoundError::QuorumNotReached {
                completed_weight: progress.completed_weight,
                required_weight: progress.required_weight,
            });
        }

        self.finalize(PorRatingRoundCloseReason::Quorum {
            completed_weight: progress.completed_weight,
            required_weight: progress.required_weight,
        })
    }

    /// Close once the policy's deterministic finalized-wave cutoff is reached.
    ///
    /// `observed_finalized_wave` must come from the adapter's validated
    /// finality path rather than a local clock or unverified peer claim.
    pub fn close_at_finalized_wave(
        &mut self,
        policy: &PorRatingRoundCutoffPolicy,
        observed_finalized_wave: u64,
    ) -> Result<&RatingBatch, PorRatingRoundError> {
        self.require_close_prerequisites()?;
        let required_finalized_wave =
            policy.ensure_reached(self.opened, observed_finalized_wave)?;

        self.finalize(PorRatingRoundCloseReason::FinalizedWaveCutoff {
            observed_finalized_wave,
            required_finalized_wave,
        })
    }

    fn finalize(
        &mut self,
        reason: PorRatingRoundCloseReason,
    ) -> Result<&RatingBatch, PorRatingRoundError> {
        let batch = self
            .collector
            .as_ref()
            .expect("open coordinator always has a collector")
            .build_complete_batch()?;
        self.collector = None;
        self.close_reason = Some(reason);
        self.completed_batch = Some(batch);
        Ok(self
            .completed_batch
            .as_ref()
            .expect("completed batch was just stored"))
    }

    fn require_close_prerequisites(&self) -> Result<(), PorRatingRoundError> {
        self.require_open()?;
        if self.local_batch.is_none() {
            return Err(PorRatingRoundError::LocalBatchNotProduced);
        }
        let pending = self.pending_outbound();
        if pending != 0 {
            return Err(PorRatingRoundError::PendingOutboundRatings(pending));
        }

        Ok(())
    }

    fn require_open(&self) -> Result<(), PorRatingRoundError> {
        if self.completed_batch.is_some() {
            Err(PorRatingRoundError::RoundClosed)
        } else {
            Ok(())
        }
    }
}
