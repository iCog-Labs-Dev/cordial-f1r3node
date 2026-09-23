//! Extract Proof-of-Reputation interactions from finalized Cordial output.
//!
//! This module is an adapter boundary. It identifies replayable protocol
//! evidence but does not admit, score, sign, or batch ratings.

use std::{collections::BTreeMap, fmt};

use cordial_miners_core::{
    Blocklace,
    consensus::{depth, wave_of_round},
    types::{BlockIdentity, NodeId},
};
use cordial_por::{
    InteractionEvidence, InteractionKind, PorError, rating_round_from_finalized_wave,
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
        }
    }
}

impl std::error::Error for PorInteractionError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::RatingRound(error) => Some(error),
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
