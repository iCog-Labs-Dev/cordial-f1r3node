//! Extract Proof-of-Reputation interactions from finalized Cordial output.
//!
//! This module is an adapter boundary. It identifies replayable protocol
//! evidence and delegates its admission to `cordial-por`. Scoring, signing,
//! and batching live in the adapter's `por_ratings` module.

use std::{collections::BTreeMap, fmt};

use cordial_miners_core::{
    Blocklace,
    consensus::{depth, wave_of_round},
    types::{BlockIdentity, NodeId},
};
use cordial_por::{
    AdmittedInteraction, InteractionEvidence, InteractionKind, PorError, ReputationState,
    admit_interaction_evidence, rating_round_from_finalized_wave,
};

use crate::{ordered_output::OrderedFinalizedOutput, por_finality::FinalizedRatingRound};

/// Errors while extracting PoR evidence from finalized adapter output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PorInteractionError {
    MissingFinalLeader,
    InvalidWavelength,
    UnknownFinalLeader,
    UnknownFinalizedBlock,
    FinalizedRoundMismatch,
    RatingRound(PorError),
    Admission(PorError),
}

impl fmt::Display for PorInteractionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingFinalLeader => write!(f, "finalized output has no final leader anchor"),
            Self::InvalidWavelength => write!(f, "finalized output wavelength must be non-zero"),
            Self::UnknownFinalLeader => {
                write!(f, "final leader anchor is not present in the blocklace")
            }
            Self::UnknownFinalizedBlock => {
                write!(
                    f,
                    "finalized output contains a block absent from the blocklace"
                )
            }
            Self::FinalizedRoundMismatch => write!(
                f,
                "opened PoR round does not match the finalized output wave"
            ),
            Self::RatingRound(error) => error.fmt(f),
            Self::Admission(error) => write!(f, "PoR interaction admission failed: {error}"),
        }
    }
}

impl std::error::Error for PorInteractionError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::RatingRound(error) | Self::Admission(error) => Some(error),
            _ => None,
        }
    }
}

/// Extract one deterministic block-production interaction per recipient.
///
/// `output` is cumulative, so only blocks belonging to
/// `opened.finalized_wave` are considered. For a producer with multiple
/// finalized blocks in that wave, the minimum `BlockIdentity` is used as the
/// evidence reference. The local `rater` is omitted as a recipient because
/// PoR forbids self-ratings.
///
/// The returned vector is ordered by recipient `NodeId`. Evidence remains
/// unsigned; the validator signing boundary supplies signatures later.
pub fn extract_block_production_evidence(
    blocklace: &Blocklace,
    output: &OrderedFinalizedOutput,
    opened: FinalizedRatingRound,
    rater: &NodeId,
) -> Result<Vec<InteractionEvidence>, PorInteractionError> {
    validate_finalized_rating_round(blocklace, output, opened)?;

    let finalized_wave = opened.finalized_wave;
    let expected_rating_round = opened.rating_round;
    let mut canonical_blocks: BTreeMap<&NodeId, &BlockIdentity> = BTreeMap::new();

    for block in &output.blocks {
        let block_round =
            depth(blocklace, block).ok_or(PorInteractionError::UnknownFinalizedBlock)?;
        let block_wave = wave_of_round(block_round, output.wavelength)
            .ok_or(PorInteractionError::InvalidWavelength)?;

        if block_wave != finalized_wave || &block.creator == rater {
            continue;
        }

        canonical_blocks
            .entry(&block.creator)
            .and_modify(|canonical| {
                if block < *canonical {
                    *canonical = block;
                }
            })
            .or_insert(block);
    }

    Ok(canonical_blocks
        .into_iter()
        .map(|(recipient, block)| InteractionEvidence {
            finalized_wave,
            round: expected_rating_round,
            kind: InteractionKind::BlockProduction,
            rater: rater.clone(),
            recipient: recipient.clone(),
            evidence_ref: block.content_hash.to_vec(),
        })
        .collect())
}

/// Validate that finalized output opens the supplied PoR rating round.
///
/// This performs the round-anchor checks independently of any rater or
/// extracted interaction, allowing an empty rating-round collector to retain
/// the same finality guarantees as a non-empty one.
pub fn validate_finalized_rating_round(
    blocklace: &Blocklace,
    output: &OrderedFinalizedOutput,
    opened: FinalizedRatingRound,
) -> Result<(), PorInteractionError> {
    let final_leader = output
        .anchor
        .as_ref()
        .ok_or(PorInteractionError::MissingFinalLeader)?;

    if output.wavelength == 0 {
        return Err(PorInteractionError::InvalidWavelength);
    }

    let leader_round =
        depth(blocklace, final_leader).ok_or(PorInteractionError::UnknownFinalLeader)?;
    let finalized_wave = wave_of_round(leader_round, output.wavelength)
        .ok_or(PorInteractionError::InvalidWavelength)?;

    let expected_rating_round = rating_round_from_finalized_wave(finalized_wave)
        .map_err(PorInteractionError::RatingRound)?;
    if finalized_wave != opened.finalized_wave || expected_rating_round != opened.rating_round {
        return Err(PorInteractionError::FinalizedRoundMismatch);
    }

    Ok(())
}

/// Extract and admit finalized block-production interactions atomically.
///
/// Evidence extraction remains adapter-owned, while admission is delegated to
/// `cordial_por` so validator membership, ejection, and round policy have one
/// authoritative implementation. No score or signature is produced here.
pub fn admit_finalized_block_production_interactions(
    blocklace: &Blocklace,
    output: &OrderedFinalizedOutput,
    opened: FinalizedRatingRound,
    rater: &NodeId,
    state: &ReputationState,
) -> Result<Vec<AdmittedInteraction>, PorInteractionError> {
    extract_block_production_evidence(blocklace, output, opened, rater)?
        .into_iter()
        .map(|evidence| {
            admit_interaction_evidence(evidence, state).map_err(PorInteractionError::Admission)
        })
        .collect()
}
