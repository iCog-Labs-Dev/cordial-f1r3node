//! Admission boundary for evidence-backed validator interactions.
//!
//! The adapter extracts protocol evidence after Cordial Miners finalizes a
//! wave. This module validates that evidence against the preceding reputation
//! state before a later scoring policy turns it into a `RatingRecord`.

use cordial_miners_core::NodeId;

use crate::{
    PorConfig, RatingRecord, RatingScore,
    error::PorError,
    ratings::{RatingSourceCheckpoint, rating_round_from_finalized_wave, validate_rating},
    state::ReputationState,
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

/// Evidence assigned to one finalized-prefix checkpoint rather than a wave.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckpointInteractionEvidence {
    pub source: RatingSourceCheckpoint,
    pub kind: InteractionKind,
    pub rater: NodeId,
    pub recipient: NodeId,
    pub evidence_ref: Vec<u8>,
}

/// Checkpoint evidence that passed the preceding-state admission policy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdmittedCheckpointInteraction(CheckpointInteractionEvidence);

impl AdmittedCheckpointInteraction {
    pub fn evidence(&self) -> &CheckpointInteractionEvidence {
        &self.0
    }

    pub fn into_evidence(self) -> CheckpointInteractionEvidence {
        self.0
    }
}

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

    validate_interaction_participants(
        evidence.round,
        &evidence.rater,
        &evidence.recipient,
        &evidence.evidence_ref,
        state,
    )?;

    Ok(AdmittedInteraction(evidence))
}

/// Admit checkpoint evidence against the immediately preceding PoR state.
///
/// The adapter must first prove that the reference belongs to the canonical
/// finalized source window; this function enforces source and state policy.
pub fn admit_checkpoint_interaction_evidence(
    evidence: CheckpointInteractionEvidence,
    state: &ReputationState,
) -> Result<AdmittedCheckpointInteraction, PorError> {
    evidence.source.validate()?;
    validate_interaction_participants(
        evidence.source.round,
        &evidence.rater,
        &evidence.recipient,
        &evidence.evidence_ref,
        state,
    )?;
    Ok(AdmittedCheckpointInteraction(evidence))
}

/// Check that a checkpoint rater is active even when it has no recipients.
pub fn validate_checkpoint_rater(
    source: &RatingSourceCheckpoint,
    rater: &NodeId,
    state: &ReputationState,
) -> Result<(), PorError> {
    source.validate()?;
    validate_interaction_rater(source.round, rater, state)
}

fn validate_interaction_rater(
    round: ReputationRound,
    rater: &NodeId,
    state: &ReputationState,
) -> Result<(), PorError> {
    if state.round().checked_add(1) != Some(round) {
        return Err(PorError::InvalidInteractionStateRound);
    }
    if !state_contains(state, rater) {
        return Err(PorError::UnknownInteractionRater);
    }
    if state.is_ejected(rater) {
        return Err(PorError::EjectedInteractionRater);
    }
    Ok(())
}

fn validate_interaction_participants(
    round: ReputationRound,
    rater: &NodeId,
    recipient: &NodeId,
    evidence_ref: &[u8],
    state: &ReputationState,
) -> Result<(), PorError> {
    if state.round().checked_add(1) != Some(round) {
        return Err(PorError::InvalidInteractionStateRound);
    }
    if rater == recipient {
        return Err(PorError::SelfInteraction);
    }
    if evidence_ref.is_empty() {
        return Err(PorError::MissingInteractionReference);
    }
    validate_interaction_rater(round, rater, state)?;
    if !state_contains(state, recipient) {
        return Err(PorError::UnknownInteractionRecipient);
    }
    if state.is_ejected(recipient) {
        return Err(PorError::EjectedInteractionRecipient);
    }
    Ok(())
}

fn state_contains(state: &ReputationState, node_id: &NodeId) -> bool {
    state
        .reputation_list()
        .entries
        .binary_search_by(|entry| entry.node_id.cmp(node_id))
        .is_ok()
}

pub fn score_admitted_interaction(
    interaction: &AdmittedInteraction,
    config: &PorConfig,
) -> Result<RatingScore, PorError> {
    score_interaction_kind(interaction.evidence().kind, config)
}

/// Score previously admitted checkpoint evidence with the same v1 policy.
pub fn score_admitted_checkpoint_interaction(
    interaction: &AdmittedCheckpointInteraction,
    config: &PorConfig,
) -> Result<RatingScore, PorError> {
    score_interaction_kind(interaction.evidence().kind, config)
}

fn score_interaction_kind(
    kind: InteractionKind,
    config: &PorConfig,
) -> Result<RatingScore, PorError> {
    if config.minimum_rating > config.maximum_rating {
        return Err(PorError::InvalidConfiguration(
            "minimum rating must not exceed maximum rating".to_string(),
        ));
    }

    match kind {
        InteractionKind::BlockProduction
        | InteractionKind::CordialReferences
        | InteractionKind::ExecutionResult
        | InteractionKind::DeployInclusion => Ok(config.maximum_rating),
    }
}

pub fn build_rating_from_interaction(
    interaction: AdmittedInteraction,
    signature: Vec<u8>,
    config: &PorConfig,
) -> Result<RatingRecord, PorError> {
    let score = score_admitted_interaction(&interaction, config)?;

    let InteractionEvidence {
        round,
        rater,
        recipient,
        evidence_ref,
        ..
    } = interaction.into_evidence();

    let mut rating = RatingRecord::new(round, rater, recipient, score, signature);

    rating.interaction_ref = Some(evidence_ref);

    validate_rating(&rating, config)?;

    Ok(rating)
}
