//! Admission boundary for evidence-backed validator interactions.
//!
//! The adapter extracts protocol evidence after Cordial Miners finalizes a
//! wave. This module validates that evidence against the preceding reputation
//! state before a later scoring policy turns it into a `RatingRecord`.

use cordial_miners_core::NodeId;

use crate::{
    error::PorError, ratings::rating_round_from_finalized_wave, state::ReputationState,
    types::ReputationRound,
};

/// Protocol interaction categories eligible for ordinary reputation ratings.
///
/// Objective faults such as equivocation and inactivity are deliberately
/// absent because they belong to deterministic penalty paths.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum InteractionKind {
    BlockProduction,
    CordialReferences,
    ExecutionResult,
    DeployInclusion,
}

/// Replayable evidence for one validator interaction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InteractionEvidence {
    /// Cordial Miners wave whose finality made this evidence admissible.
    pub finalized_wave: u64,
    /// PoR round that will consume the evidence; always `finalized_wave + 1`.
    pub round: ReputationRound,
    pub kind: InteractionKind,
    pub rater: NodeId,
    pub recipient: NodeId,
    /// Stable protocol reference such as a block or state-transition hash.
    pub evidence_ref: Vec<u8>,
}

/// Interaction evidence that passed the PoR admission boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdmittedInteraction(InteractionEvidence);

impl AdmittedInteraction {
    pub fn evidence(&self) -> &InteractionEvidence {
        &self.0
    }

    pub fn into_evidence(self) -> InteractionEvidence {
        self.0
    }
}

/// Validate finalized interaction evidence for the next PoR round.
///
/// Admission verifies the finalized-wave/rating-round relationship, requires
/// the current reputation state to be the immediately preceding state, and
/// restricts ordinary ratings to known, non-ejected validators. Signature and
/// score validation remain in the later `RatingRecord` admission stage.
pub fn admit_interaction_evidence(
    evidence: InteractionEvidence,
    state: &ReputationState,
) -> Result<AdmittedInteraction, PorError> {
    let expected_round = rating_round_from_finalized_wave(evidence.finalized_wave)?;
    if evidence.round != expected_round {
        return Err(PorError::InvalidInteractionRound);
    }

    if state.round().checked_add(1) != Some(evidence.round) {
        return Err(PorError::InvalidInteractionStateRound);
    }

    if evidence.rater == evidence.recipient {
        return Err(PorError::SelfInteraction);
    }

    if evidence.evidence_ref.is_empty() {
        return Err(PorError::MissingInteractionReference);
    }

    if !state_contains(state, &evidence.rater) {
        return Err(PorError::UnknownInteractionRater);
    }

    if state.is_ejected(&evidence.rater) {
        return Err(PorError::EjectedInteractionRater);
    }

    if !state_contains(state, &evidence.recipient) {
        return Err(PorError::UnknownInteractionRecipient);
    }

    if state.is_ejected(&evidence.recipient) {
        return Err(PorError::EjectedInteractionRecipient);
    }

    Ok(AdmittedInteraction(evidence))
}

fn state_contains(state: &ReputationState, node_id: &NodeId) -> bool {
    state
        .reputation_list()
        .entries
        .binary_search_by(|entry| entry.node_id.cmp(node_id))
        .is_ok()
}
