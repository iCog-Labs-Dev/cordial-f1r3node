//! Evidence-backed collection of signed Proof-of-Reputation ratings.
//!
//! This module is transport-independent. Network and storage adapters can feed
//! received ratings or per-validator batches into the collector only after a
//! Cordial wave has finalized. Each submission is checked against the
//! canonical block-production evidence before the round can be closed.

use std::{collections::BTreeMap, fmt};

use cordial_miners_core::{Blocklace, NodeId};
use cordial_por::{
    PorConfig, PorError, RatingBatch, RatingRecord, ReputationRound, ReputationState,
    canonical_rating_payload, score_admitted_interaction,
};

use crate::{
    ordered_output::OrderedFinalizedOutput,
    por_finality::FinalizedRatingRound,
    por_interactions::{
        PorInteractionError, admit_finalized_block_production_interactions,
        validate_finalized_rating_round,
    },
    por_rating_wire::BlockProductionRatingEnvelopeV1,
    por_ratings::{PorRatingError, build_verified_rating_batch, validate_signed_rating},
};

/// Failures while collecting an evidence-backed PoR rating round.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PorRatingCollectorError {
    Interaction(PorInteractionError),
    Rating(PorRatingError),
    InvalidStateRound,
    InvalidFinalizedWave,
    InvalidRatingRound,
    InvalidBatchRound,
    MissingFinalizedInteraction,
    InteractionReferenceMismatch,
    ScoreMismatch,
    DuplicateRating,
    ConflictingRating,
}

impl fmt::Display for PorRatingCollectorError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Interaction(error) => error.fmt(f),
            Self::Rating(error) => error.fmt(f),
            Self::InvalidStateRound => write!(
                f,
                "rating round must immediately follow the collector's reputation state"
            ),
            Self::InvalidFinalizedWave => {
                write!(
                    f,
                    "rating envelope does not belong to the collector's finalized wave"
                )
            }
            Self::InvalidRatingRound => {
                write!(f, "rating does not belong to the collector's opened round")
            }
            Self::InvalidBatchRound => {
                write!(
                    f,
                    "rating batch does not belong to the collector's opened round"
                )
            }
            Self::MissingFinalizedInteraction => write!(
                f,
                "rating recipient has no canonical finalized block-production interaction"
            ),
            Self::InteractionReferenceMismatch => write!(
                f,
                "rating interaction reference does not match finalized evidence"
            ),
            Self::ScoreMismatch => {
                write!(
                    f,
                    "rating score does not match deterministic interaction scoring"
                )
            }
            Self::DuplicateRating => {
                write!(f, "duplicate rating for the same rater and recipient")
            }
            Self::ConflictingRating => {
                write!(f, "conflicting rating for the same rater and recipient")
            }
        }
    }
}

impl std::error::Error for PorRatingCollectorError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Interaction(error) => Some(error),
            Self::Rating(error) => Some(error),
            _ => None,
        }
    }
}

impl From<PorInteractionError> for PorRatingCollectorError {
    fn from(error: PorInteractionError) -> Self {
        Self::Interaction(error)
    }
}

impl From<PorRatingError> for PorRatingCollectorError {
    fn from(error: PorRatingError) -> Self {
        Self::Rating(error)
    }
}

impl From<PorError> for PorRatingCollectorError {
    fn from(error: PorError) -> Self {
        Self::Rating(PorRatingError::Protocol(error))
    }
}

/// Accumulates signed ratings for one finalized block-production round.
///
/// Construction validates the finality anchor and preceding state round even
/// when no ratings arrive. Insertions verify authorship and reconstruct the
/// canonical interaction from finalized Cordial output. `finish` consumes the
/// collector and returns the globally ordered verified batch.
pub struct BlockProductionRatingCollector<'a> {
    blocklace: &'a Blocklace,
    output: &'a OrderedFinalizedOutput,
    opened: FinalizedRatingRound,
    state: &'a ReputationState,
    config: &'a PorConfig,
    ratings: BTreeMap<(NodeId, NodeId), RatingRecord>,
}

impl<'a> BlockProductionRatingCollector<'a> {
    pub fn new(
        blocklace: &'a Blocklace,
        output: &'a OrderedFinalizedOutput,
        opened: FinalizedRatingRound,
        state: &'a ReputationState,
        config: &'a PorConfig,
    ) -> Result<Self, PorRatingCollectorError> {
        validate_finalized_rating_round(blocklace, output, opened)?;

        if state.round().checked_add(1) != Some(opened.rating_round) {
            return Err(PorRatingCollectorError::InvalidStateRound);
        }

        Ok(Self {
            blocklace,
            output,
            opened,
            state,
            config,
            ratings: BTreeMap::new(),
        })
    }

    pub fn round(&self) -> ReputationRound {
        self.opened.rating_round
    }

    pub fn len(&self) -> usize {
        self.ratings.len()
    }

    pub fn is_empty(&self) -> bool {
        self.ratings.is_empty()
    }

    /// Insert one signed rating after signature and finalized-evidence checks.
    pub fn insert(&mut self, rating: RatingRecord) -> Result<(), PorRatingCollectorError> {
        self.validate_submission(&self.ratings, &rating)?;
        self.ratings.insert(rating_key(&rating), rating);
        Ok(())
    }

    /// Insert one decoded v1 block-production rating envelope.
    pub fn insert_envelope(
        &mut self,
        envelope: BlockProductionRatingEnvelopeV1,
    ) -> Result<(), PorRatingCollectorError> {
        if envelope.finalized_wave() != self.opened.finalized_wave {
            return Err(PorRatingCollectorError::InvalidFinalizedWave);
        }

        self.insert(envelope.into_rating())
    }

    /// Insert a per-validator batch atomically.
    ///
    /// If any rating is invalid, none of the batch's ratings are retained.
    pub fn insert_batch(&mut self, batch: RatingBatch) -> Result<(), PorRatingCollectorError> {
        if batch.round != self.opened.rating_round {
            return Err(PorRatingCollectorError::InvalidBatchRound);
        }

        let mut staged = self.ratings.clone();
        for rating in batch.ratings {
            self.validate_submission(&staged, &rating)?;
            staged.insert(rating_key(&rating), rating);
        }

        self.ratings = staged;
        Ok(())
    }

    /// Close collection and build the canonical verified round batch.
    ///
    /// Quorum and timeout policy are intentionally external; the caller
    /// decides when the collection window is complete.
    pub fn finish(self) -> Result<RatingBatch, PorRatingCollectorError> {
        self.build_batch()
    }

    /// Build a canonical verified snapshot without consuming the collector.
    pub fn build_batch(&self) -> Result<RatingBatch, PorRatingCollectorError> {
        let ratings = self.ratings.values().cloned().collect();
        Ok(build_verified_rating_batch(
            self.opened.rating_round,
            ratings,
            self.config,
        )?)
    }

    fn validate_submission(
        &self,
        accepted: &BTreeMap<(NodeId, NodeId), RatingRecord>,
        rating: &RatingRecord,
    ) -> Result<(), PorRatingCollectorError> {
        if rating.round != self.opened.rating_round {
            return Err(PorRatingCollectorError::InvalidRatingRound);
        }

        validate_signed_rating(rating, self.config)?;

        if let Some(existing) = accepted.get(&rating_key(rating)) {
            return if canonical_rating_payload(existing) == canonical_rating_payload(rating) {
                Err(PorRatingCollectorError::DuplicateRating)
            } else {
                Err(PorRatingCollectorError::ConflictingRating)
            };
        }

        let admitted = admit_finalized_block_production_interactions(
            self.blocklace,
            self.output,
            self.opened,
            &rating.rater,
            self.state,
        )?;
        let interaction = admitted
            .iter()
            .find(|interaction| interaction.evidence().recipient == rating.recipient)
            .ok_or(PorRatingCollectorError::MissingFinalizedInteraction)?;

        if rating.interaction_ref.as_deref() != Some(interaction.evidence().evidence_ref.as_slice())
        {
            return Err(PorRatingCollectorError::InteractionReferenceMismatch);
        }

        if rating.score != score_admitted_interaction(interaction, self.config)? {
            return Err(PorRatingCollectorError::ScoreMismatch);
        }

        Ok(())
    }
}

fn rating_key(rating: &RatingRecord) -> (NodeId, NodeId) {
    (rating.rater.clone(), rating.recipient.clone())
}
