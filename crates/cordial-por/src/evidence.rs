//! Verified proof -> protocol authorization -> penalty capability.
//!
//! Policies are trusted protocol implementations, never selected by a peer.
//! No byte-only constructor or decoder can create an authorized penalty.

use cordial_miners_core::{
    NodeId,
    consensus::VerifiedEquivocationEvidence,
    crypto::{Blake2b256Hasher, Hasher},
};

use crate::{
    CheckpointReputationBlockContext, EquivocationPenalty, MAX_PENALTY_EVIDENCE_LEN,
    MAX_REPUTATION_BLOCK_SHARD_ID_LEN, PorError, RatingSourceCheckpoint, ReputationBlockContext,
    rating_round_from_finalized_wave,
};

pub const POR_EQUIVOCATION_AUTHORIZATION_DOMAIN: &[u8] =
    b"cordial-por:equivocation-authorization:v1";

/// Context approved by the protocol. Cordial depths and PoR rounds are distinct.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EquivocationAuthorizationContext {
    pub shard_id: Vec<u8>,
    pub source_finalized_wave: u64,
    pub round: u64,
}

/// V2 intents name a target 16-block window. The containing block cannot sign
/// the resulting prefix hash (which includes itself); admission binds that hash.
pub const POR_CHECKPOINT_EQUIVOCATION_AUTHORIZATION_DOMAIN: &[u8] =
    b"cordial-por:equivocation-authorization:v2";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckpointEquivocationAuthorizationContext {
    pub shard_id: Vec<u8>,
    pub source: RatingSourceCheckpoint,
}

pub trait CheckpointEquivocationAuthorizationPolicy {
    fn authorize_checkpoint(
        &self,
        context: &CheckpointEquivocationAuthorizationContext,
        evidence: &VerifiedEquivocationEvidence,
    ) -> Result<(), PorError>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum AuthorizationContext {
    Wave(EquivocationAuthorizationContext),
    Checkpoint(CheckpointEquivocationAuthorizationContext),
}

/// The adapter must supply a policy backed by its trusted authorization path.
/// Proof authentication has already succeeded when this method is called.
pub trait EquivocationAuthorizationPolicy {
    fn authorize(
        &self,
        context: &EquivocationAuthorizationContext,
        evidence: &VerifiedEquivocationEvidence,
    ) -> Result<(), PorError>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct EquivocationAuthorization {
    context: AuthorizationContext,
    binding: [u8; 32],
    proof_round: u64,
}

/// Exact payload a host may finalize to authorize this proof in this context.
/// Building this payload grants no authority; the policy must establish approval.
pub fn equivocation_authorization_payload(
    context: &EquivocationAuthorizationContext,
    evidence: &VerifiedEquivocationEvidence,
) -> Result<Vec<u8>, PorError> {
    authorization_payload(context, evidence.validator(), &evidence.canonical_bytes())
}

fn authorization_payload(
    context: &EquivocationAuthorizationContext,
    offender: &NodeId,
    evidence: &[u8],
) -> Result<Vec<u8>, PorError> {
    if context.shard_id.is_empty()
        || context.shard_id.len() > MAX_REPUTATION_BLOCK_SHARD_ID_LEN
        || rating_round_from_finalized_wave(context.source_finalized_wave)? != context.round
        || evidence.is_empty()
        || evidence.len() > MAX_PENALTY_EVIDENCE_LEN
    {
        return Err(PorError::InvalidPenaltyEvents(
            "invalid equivocation authorization context or proof size".into(),
        ));
    }
    fn put_bytes(output: &mut Vec<u8>, bytes: &[u8]) {
        output.extend_from_slice(&(bytes.len() as u64).to_be_bytes());
        output.extend_from_slice(bytes);
    }
    let mut payload = POR_EQUIVOCATION_AUTHORIZATION_DOMAIN.to_vec();
    put_bytes(&mut payload, &context.shard_id);
    payload.extend_from_slice(&context.source_finalized_wave.to_be_bytes());
    payload.extend_from_slice(&context.round.to_be_bytes());
    put_bytes(&mut payload, &offender.0);
    put_bytes(&mut payload, evidence);
    Ok(payload)
}

impl EquivocationPenalty {
    /// Retain a raw claim for inspection. This value always fails penalty replay
    /// until replaced by the result of authorize with an authenticated proof.
    pub fn unverified(offender: NodeId, evidence: Vec<u8>) -> Self {
        Self {
            offender,
            evidence,
            authorization: None,
        }
    }

    /// Derive the offender and exact proof bytes; callers cannot substitute them.
    pub fn authorize(
        proof: &VerifiedEquivocationEvidence,
        context: EquivocationAuthorizationContext,
        policy: &impl EquivocationAuthorizationPolicy,
    ) -> Result<Self, PorError> {
        let evidence = proof.canonical_bytes();
        let payload = authorization_payload(&context, proof.validator(), &evidence)?;
        policy.authorize(&context, proof)?;
        Ok(Self {
            offender: proof.validator().clone(),
            evidence,
            authorization: Some(EquivocationAuthorization {
                context: AuthorizationContext::Wave(context),
                binding: Blake2b256Hasher.hash(&payload),
                proof_round: proof.round(),
            }),
        })
    }

    pub(crate) fn proof_round(&self) -> Result<u64, PorError> {
        self.authorization
            .as_ref()
            .map(|authorization| authorization.proof_round)
            .ok_or_else(|| {
                PorError::InvalidPenaltyEvents("equivocation has no verified authorization".into())
            })
    }

    pub(crate) fn validate_authorization(&self, round: u64) -> Result<(), PorError> {
        let authorization = self.authorization.as_ref().ok_or_else(|| {
            PorError::InvalidPenaltyEvents("equivocation has no verified authorization".into())
        })?;
        let (authorized_round, payload) = match &authorization.context {
            AuthorizationContext::Wave(context) => (
                context.round,
                authorization_payload(context, &self.offender, &self.evidence)?,
            ),
            AuthorizationContext::Checkpoint(context) => (
                context.source.round,
                checkpoint_capability_payload(context, &self.offender, &self.evidence)?,
            ),
        };
        if authorized_round != round || authorization.binding != Blake2b256Hasher.hash(&payload) {
            return Err(PorError::InvalidPenaltyEvents(
                "equivocation proof, offender or authorized round was changed".into(),
            ));
        }
        Ok(())
    }

    pub(crate) fn validate_context(
        &self,
        context: ReputationBlockContext<'_>,
    ) -> Result<(), PorError> {
        let authorization = self.authorization.as_ref().ok_or_else(|| {
            PorError::InvalidPenaltyEvents("equivocation has no verified authorization".into())
        })?;
        let AuthorizationContext::Wave(authorized) = &authorization.context else {
            return Err(PorError::InvalidPenaltyEvents(
                "checkpoint penalty used in wave transition".into(),
            ));
        };
        if authorized.shard_id != context.shard_id
            || authorized.source_finalized_wave != context.source_finalized_wave
        {
            return Err(PorError::InvalidPenaltyEvents(
                "equivocation authorization belongs to another shard or finalized wave".into(),
            ));
        }
        Ok(())
    }
}

pub(crate) fn validate_penalty_context(
    penalties: Option<&crate::ReputationPenaltyEvents>,
    context: ReputationBlockContext<'_>,
) -> Result<(), PorError> {
    if let Some(events) = penalties {
        for event in &events.equivocations {
            event.validate_context(context)?;
        }
    }
    Ok(())
}

/// Producer-signed intent. Only finalization in the target window authorizes it.
pub fn checkpoint_equivocation_authorization_payload(
    shard_id: &[u8],
    round: u64,
    evidence: &VerifiedEquivocationEvidence,
) -> Result<Vec<u8>, PorError> {
    checkpoint_intent_payload(
        shard_id,
        round,
        evidence.validator(),
        &evidence.canonical_bytes(),
    )
}

fn checkpoint_intent_payload(
    shard_id: &[u8],
    round: u64,
    offender: &NodeId,
    evidence: &[u8],
) -> Result<Vec<u8>, PorError> {
    if shard_id.is_empty()
        || shard_id.len() > MAX_REPUTATION_BLOCK_SHARD_ID_LEN
        || round == 0
        || round
            .checked_mul(crate::POR_FINALIZED_BLOCKS_PER_CHECKPOINT)
            .is_none()
        || evidence.is_empty()
        || evidence.len() > MAX_PENALTY_EVIDENCE_LEN
    {
        return Err(PorError::InvalidPenaltyEvents(
            "invalid checkpoint authorization intent".into(),
        ));
    }
    let mut bytes = POR_CHECKPOINT_EQUIVOCATION_AUTHORIZATION_DOMAIN.to_vec();
    bytes.extend_from_slice(&(shard_id.len() as u64).to_be_bytes());
    bytes.extend_from_slice(shard_id);
    bytes.extend_from_slice(&round.to_be_bytes());
    bytes.extend_from_slice(&(offender.0.len() as u64).to_be_bytes());
    bytes.extend_from_slice(&offender.0);
    bytes.extend_from_slice(&(evidence.len() as u64).to_be_bytes());
    bytes.extend_from_slice(evidence);
    Ok(bytes)
}

fn checkpoint_capability_payload(
    context: &CheckpointEquivocationAuthorizationContext,
    offender: &NodeId,
    evidence: &[u8],
) -> Result<Vec<u8>, PorError> {
    context.source.validate()?;
    let mut bytes =
        checkpoint_intent_payload(&context.shard_id, context.source.round, offender, evidence)?;
    bytes.extend_from_slice(&context.source.finalized_prefix_len.to_be_bytes());
    bytes.extend_from_slice(&context.source.finalized_prefix_hash);
    Ok(bytes)
}

impl EquivocationPenalty {
    pub fn authorize_checkpoint(
        proof: &VerifiedEquivocationEvidence,
        context: CheckpointEquivocationAuthorizationContext,
        policy: &impl CheckpointEquivocationAuthorizationPolicy,
    ) -> Result<Self, PorError> {
        let evidence = proof.canonical_bytes();
        let payload = checkpoint_capability_payload(&context, proof.validator(), &evidence)?;
        policy.authorize_checkpoint(&context, proof)?;
        Ok(Self {
            offender: proof.validator().clone(),
            evidence,
            authorization: Some(EquivocationAuthorization {
                context: AuthorizationContext::Checkpoint(context),
                binding: Blake2b256Hasher.hash(&payload),
                proof_round: proof.round(),
            }),
        })
    }
}

pub(crate) fn validate_checkpoint_penalty_context(
    penalties: Option<&crate::ReputationPenaltyEvents>,
    context: CheckpointReputationBlockContext<'_>,
) -> Result<(), PorError> {
    if let Some(events) = penalties {
        for event in &events.equivocations {
            let authorized = event.authorization.as_ref().ok_or_else(|| {
                PorError::InvalidPenaltyEvents("missing checkpoint authorization".into())
            })?;
            match &authorized.context {
                AuthorizationContext::Checkpoint(approved)
                    if approved.shard_id == context.shard_id
                        && approved.source == context.source => {}
                _ => {
                    return Err(PorError::InvalidPenaltyEvents(
                        "penalty belongs to another authorization source".into(),
                    ));
                }
            }
        }
    }
    Ok(())
}

/// Recover an unverified claim from a v2 signed carrier. Decoding never grants
/// permission to penalize; adapter admission must authenticate the recovered proof.
pub fn decode_checkpoint_equivocation_authorization_payload(
    bytes: &[u8],
    shard_id: &[u8],
    round: u64,
) -> Result<cordial_miners_core::consensus::CordialEquivocationEvidence, PorError> {
    fn invalid() -> PorError {
        PorError::InvalidPenaltyEvents("malformed checkpoint authorization".into())
    }
    fn take<'a>(input: &mut &'a [u8], n: usize) -> Result<&'a [u8], PorError> {
        if n > input.len() {
            return Err(invalid());
        }
        let (head, tail) = input.split_at(n);
        *input = tail;
        Ok(head)
    }
    fn number(input: &mut &[u8]) -> Result<u64, PorError> {
        Ok(u64::from_be_bytes(
            take(input, 8)?.try_into().map_err(|_| invalid())?,
        ))
    }
    fn field<'a>(input: &mut &'a [u8], max: usize) -> Result<&'a [u8], PorError> {
        let n = usize::try_from(number(input)?).map_err(|_| invalid())?;
        if n > max {
            return Err(invalid());
        }
        take(input, n)
    }
    let mut input = bytes
        .strip_prefix(POR_CHECKPOINT_EQUIVOCATION_AUTHORIZATION_DOMAIN)
        .ok_or_else(invalid)?;
    if field(&mut input, MAX_REPUTATION_BLOCK_SHARD_ID_LEN)? != shard_id
        || number(&mut input)? != round
    {
        return Err(invalid());
    }
    let offender = NodeId(field(&mut input, crate::MAX_REPUTATION_BLOCK_NODE_ID_LEN)?.to_vec());
    let proof = field(&mut input, MAX_PENALTY_EVIDENCE_LEN)?;
    if !input.is_empty() || checkpoint_intent_payload(shard_id, round, &offender, proof)? != bytes {
        return Err(invalid());
    }
    let raw = cordial_miners_core::consensus::decode_equivocation_evidence(proof)
        .map_err(|error| PorError::InvalidPenaltyEvents(error.to_string()))?;
    if raw.validator != offender {
        return Err(invalid());
    }
    Ok(raw)
}
