//! Canonical signing and verification for Proof-of-Reputation ratings.
//!
//! `cordial-por` defines the signed payload, while this adapter owns access to
//! validator keys and the f1r3node-aligned cryptographic implementation.

use std::fmt;

use cordial_miners_core::{
    Blocklace,
    crypto::{Blake2b256Hasher, Hasher, Secp256k1Scheme, SignatureScheme},
    types::NodeId,
};
use cordial_por::{
    AdmittedInteraction, PorConfig, PorError, RatingBatch, RatingRecord, ReputationRound,
    ReputationState, build_rating_batch, build_rating_from_interaction, canonical_rating_payload,
    score_admitted_interaction, validate_rating,
};

use crate::{
    ordered_output::OrderedFinalizedOutput,
    por_finality::FinalizedRatingRound,
    por_interactions::{PorInteractionError, admit_finalized_block_production_interactions},
};

/// Failures at the adapter's signed-rating boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PorRatingError {
    Interaction(PorInteractionError),
    Protocol(PorError),
    Signing(String),
    InvalidSignature,
}

impl fmt::Display for PorRatingError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Interaction(error) => error.fmt(f),
            Self::Protocol(error) => error.fmt(f),
            Self::Signing(error) => write!(f, "PoR rating signing failed: {error}"),
            Self::InvalidSignature => write!(f, "PoR rating signature is invalid"),
        }
    }
}

impl std::error::Error for PorRatingError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Interaction(error) => Some(error),
            Self::Protocol(error) => Some(error),
            Self::Signing(_) | Self::InvalidSignature => None,
        }
    }
}

impl From<PorError> for PorRatingError {
    fn from(error: PorError) -> Self {
        Self::Protocol(error)
    }
}

impl From<PorInteractionError> for PorRatingError {
    fn from(error: PorInteractionError) -> Self {
        Self::Interaction(error)
    }
}

/// Hash the canonical rating payload with Blake2b-256.
///
/// The signature field is excluded by `canonical_rating_payload`, while the
/// interaction reference and every other rating field are covered.
pub fn rating_signing_hash(rating: &RatingRecord) -> Result<[u8; 32], PorRatingError> {
    require_interaction_reference(rating)?;
    Ok(Blake2b256Hasher.hash(&canonical_rating_payload(rating)))
}

/// Score and sign one previously admitted interaction.
///
/// The validator identified by `interaction.evidence().rater` is the signer.
/// A final self-verification prevents a caller from pairing the interaction
/// with another validator's private key.
pub fn sign_admitted_interaction(
    interaction: AdmittedInteraction,
    config: &PorConfig,
    private_key: &[u8],
) -> Result<RatingRecord, PorRatingError> {
    let evidence = interaction.evidence();
    let unsigned = RatingRecord {
        round: evidence.round,
        rater: evidence.rater.clone(),
        recipient: evidence.recipient.clone(),
        score: score_admitted_interaction(&interaction, config)?,
        signature: Vec::new(),
        interaction_ref: Some(evidence.evidence_ref.clone()),
    };
    let signing_hash = rating_signing_hash(&unsigned)?;
    let signature = Secp256k1Scheme
        .sign(&signing_hash, private_key)
        .map_err(PorRatingError::Signing)?;
    let rating = build_rating_from_interaction(interaction, signature, config)?;

    verify_rating_signature(&rating)?;
    Ok(rating)
}

/// Build the local validator's signed block-production ratings for one
/// finalized Cordial wave.
///
/// Extraction and admission use the finalized output and preceding reputation
/// state. Every admitted interaction is then deterministically scored, signed
/// by `rater`, verified, and assembled into the opened rating round. The
/// operation is atomic from the caller's perspective: no partial rating vector
/// or batch is returned if any stage fails.
pub fn build_finalized_block_production_rating_batch(
    blocklace: &Blocklace,
    output: &OrderedFinalizedOutput,
    opened: FinalizedRatingRound,
    rater: &NodeId,
    state: &ReputationState,
    config: &PorConfig,
    private_key: &[u8],
) -> Result<RatingBatch, PorRatingError> {
    let admitted =
        admit_finalized_block_production_interactions(blocklace, output, opened, rater, state)?;
    let ratings = admitted
        .into_iter()
        .map(|interaction| sign_admitted_interaction(interaction, config, private_key))
        .collect::<Result<Vec<_>, _>>()?;

    build_verified_rating_batch(opened.rating_round, ratings, config)
}

/// Verify a rating's DER-encoded secp256k1 signature against its rater key.
pub fn verify_rating_signature(rating: &RatingRecord) -> Result<(), PorRatingError> {
    let signing_hash = rating_signing_hash(rating)?;

    if Secp256k1Scheme.verify(&signing_hash, &rating.rater.0, &rating.signature) {
        Ok(())
    } else {
        Err(PorRatingError::InvalidSignature)
    }
}

/// Apply structural, policy, interaction-reference, and signature checks.
pub fn validate_signed_rating(
    rating: &RatingRecord,
    config: &PorConfig,
) -> Result<(), PorRatingError> {
    validate_rating(rating, config)?;
    verify_rating_signature(rating)
}

/// Verify all incoming ratings before constructing their deterministic batch.
///
/// This is the adapter ingress path for remotely supplied ratings. The core
/// `build_rating_batch` remains responsible for round matching, duplicate
/// rejection, and canonical ordering.
pub fn build_verified_rating_batch(
    round: ReputationRound,
    ratings: Vec<RatingRecord>,
    config: &PorConfig,
) -> Result<RatingBatch, PorRatingError> {
    for rating in &ratings {
        validate_signed_rating(rating, config)?;
    }

    Ok(build_rating_batch(round, ratings, config)?)
}

fn require_interaction_reference(rating: &RatingRecord) -> Result<(), PorRatingError> {
    match rating.interaction_ref.as_deref() {
        Some(interaction_ref) if !interaction_ref.is_empty() => Ok(()),
        _ => Err(PorError::MissingInteractionReference.into()),
    }
}
