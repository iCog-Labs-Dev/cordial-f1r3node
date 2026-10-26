//! Authorization intents finalized in exactly one 16-block checkpoint window.

use super::{
    finality::FinalizedPrefixCheckpoint, interactions::validate_finalized_checkpoint_window,
    lifecycle::checkpoint::CompletedCheckpointRatingRound,
};
use crate::ordered_output::OrderedFinalizedOutput;
use cordial_miners_core::{
    Blocklace, NodeId,
    consensus::{
        CordialEquivocationEvidence, EvidenceVerificationError, VerifiedEquivocationEvidence,
        depth, verify_equivocation_evidence,
    },
    crypto,
    execution::{CordialBlockPayload, PorEquivocationAuthorization},
};
use cordial_por::{
    CheckpointEquivocationAuthorizationContext, CheckpointEquivocationAuthorizationPolicy,
    EquivocationPenalty, POR_CHECKPOINT_EQUIVOCATION_AUTHORIZATION_DOMAIN,
    POR_FINALIZED_BLOCKS_PER_CHECKPOINT, PorError, ReputationList, ReputationPenaltyEvents,
    checkpoint_equivocation_authorization_payload,
    decode_checkpoint_equivocation_authorization_payload, penalty_events_commitment,
};
use std::collections::{BTreeMap, BTreeSet, HashMap};

fn invalid(message: impl ToString) -> PorError {
    PorError::InvalidPenaltyEvents(message.to_string())
}

fn window<'a>(
    blocklace: &Blocklace,
    output: &'a OrderedFinalizedOutput,
    checkpoint: &FinalizedPrefixCheckpoint,
) -> Result<&'a [cordial_miners_core::BlockIdentity], PorError> {
    validate_finalized_checkpoint_window(blocklace, output, checkpoint).map_err(invalid)?;
    let end = usize::try_from(checkpoint.finalized_prefix_len).map_err(invalid)?;
    Ok(&output.blocks[end - POR_FINALIZED_BLOCKS_PER_CHECKPOINT as usize..end])
}

/// Capture the complete set before a caller supplies any evidence list.
pub(super) fn finalized_checkpoint_authorizations(
    blocklace: &Blocklace,
    output: &OrderedFinalizedOutput,
    checkpoint: &FinalizedPrefixCheckpoint,
) -> Result<BTreeSet<PorEquivocationAuthorization>, PorError> {
    let mut messages = BTreeSet::new();
    for id in window(blocklace, output, checkpoint)? {
        let block = blocklace
            .get(id)
            .ok_or_else(|| invalid("missing finalized authorization block"))?;
        for carrier in CordialBlockPayload::por_authorizations_from_bytes(&block.content.payload)
            .map_err(invalid)?
        {
            if carrier
                .message
                .starts_with(POR_CHECKPOINT_EQUIVOCATION_AUTHORIZATION_DOMAIN)
                && carrier.source_finalized_wave == 0
                && carrier.round == checkpoint.por_round
            {
                messages.insert(carrier);
            }
        }
    }
    Ok(messages)
}

struct FinalizedCheckpointAuthorization<'a> {
    blocklace: &'a Blocklace,
    output: &'a OrderedFinalizedOutput,
    checkpoint: &'a FinalizedPrefixCheckpoint,
    membership: &'a HashMap<NodeId, u64>,
    preceding_reputation: &'a ReputationList,
}

impl<'a> FinalizedCheckpointAuthorization<'a> {
    fn extract_valid_claim(
        &self,
        carrier: &PorEquivocationAuthorization,
        shard_id: &[u8],
    ) -> Result<Option<CordialEquivocationEvidence>, PorError> {
        if carrier.shard_id != shard_id
            || carrier.source_finalized_wave != 0
            || carrier.round != self.checkpoint.por_round
        {
            return Ok(None);
        }
        // Only definitive invalidity may be ignored. Missing local verification
        // data must prevent completion rather than produce a different penalty set.
        let claim = match decode_checkpoint_equivocation_authorization_payload(
            &carrier.message,
            shard_id,
            self.checkpoint.por_round,
        ) {
            Ok(claim) => claim,
            Err(_) => return Ok(None),
        };
        // Membership and ejection are protocol facts, not proof failures.
        if !self.eligible_offender(&claim.validator) {
            return Ok(None);
        }
        let proof = match verify_equivocation_evidence(&claim, self.blocklace) {
            Ok(proof) => proof,
            Err(EvidenceVerificationError::MissingHistory) => {
                return Err(invalid(
                    "equivocation verification requires unavailable ancestry",
                ));
            }
            Err(_) => return Ok(None),
        };
        let context = CheckpointEquivocationAuthorizationContext {
            shard_id: shard_id.to_vec(),
            source: self.checkpoint.rating_source(),
        };
        if checkpoint_equivocation_authorization_payload(shard_id, context.source.round, &proof)?
            != carrier.message
            || !self.is_authorized(&context, &proof)?
        {
            return Ok(None);
        }
        Ok(Some(claim))
    }
}

impl FinalizedCheckpointAuthorization<'_> {
    fn eligible_offender(&self, offender: &NodeId) -> bool {
        self.membership.contains_key(offender)
            && self
                .preceding_reputation
                .entries
                .iter()
                .any(|entry| &entry.node_id == offender && !entry.is_excluded)
    }

    // False is a deterministic policy rejection. Errors represent unavailable
    // local data or a violated source invariant and must prevent completion.
    fn is_authorized(
        &self,
        context: &CheckpointEquivocationAuthorizationContext,
        proof: &VerifiedEquivocationEvidence,
    ) -> Result<bool, PorError> {
        if context.source != self.checkpoint.rating_source() {
            return Err(invalid(
                "invalid checkpoint authorization source or offender",
            ));
        }
        if !self.eligible_offender(proof.validator()) {
            return Ok(false);
        }
        let expected = checkpoint_equivocation_authorization_payload(
            &context.shard_id,
            context.source.round,
            proof,
        )?;
        for id in window(self.blocklace, self.output, self.checkpoint)? {
            let block = self
                .blocklace
                .get(id)
                .ok_or_else(|| invalid("missing finalized block"))?;
            let carriers =
                CordialBlockPayload::por_authorizations_from_bytes(&block.content.payload)
                    .map_err(invalid)?;
            if !carriers.iter().any(|carrier| {
                carrier.source_finalized_wave == 0
                    && carrier.round == context.source.round
                    && carrier.shard_id == context.shard_id
                    && carrier.message == expected
            }) {
                continue;
            }
            if self
                .membership
                .get(&block.identity.creator)
                .copied()
                .unwrap_or(0)
                == 0
                || crypto::block_hash(&block.identity.creator, &block.content)
                    != block.identity.content_hash
                || !crypto::verify(
                    &block.identity.content_hash,
                    &block.identity.creator.0,
                    &block.identity.signature,
                )
            {
                continue;
            }
            let authorization_depth = depth(self.blocklace, &block.identity)
                .ok_or_else(|| invalid("missing authorization ancestry"))?;
            let mut too_new = false;
            for conflicting in proof.blocks() {
                let mut content = conflicting.content.clone();
                content.predecessors = content
                    .predecessors
                    .iter()
                    .map(|id| id.consensus_identity())
                    .collect();
                let proof_depth = cordial_miners_core::consensus::round::candidate_depth(
                    self.blocklace,
                    &content,
                )
                .ok_or_else(|| invalid("missing equivocation ancestry"))?;
                if proof_depth > authorization_depth {
                    too_new = true;
                    break;
                }
            }
            if !too_new {
                return Ok(true);
            }
        }
        Ok(false)
    }
}

impl CheckpointEquivocationAuthorizationPolicy for FinalizedCheckpointAuthorization<'_> {
    fn authorize_checkpoint(
        &self,
        context: &CheckpointEquivocationAuthorizationContext,
        proof: &VerifiedEquivocationEvidence,
    ) -> Result<(), PorError> {
        if self.is_authorized(context, proof)? {
            Ok(())
        } else {
            Err(invalid(
                "proof has no eligible authorization in its target checkpoint",
            ))
        }
    }
}

impl CompletedCheckpointRatingRound {
    /// Decode untrusted carrier contents, then authenticate every proof and publisher.
    pub fn with_finalized_equivocations(
        self,
        blocklace: &Blocklace,
        output: &OrderedFinalizedOutput,
        membership: &HashMap<NodeId, u64>,
        shard_id: &[u8],
    ) -> Result<Self, PorError> {
        let policy = FinalizedCheckpointAuthorization {
            blocklace,
            output,
            checkpoint: self.checkpoint(),
            membership,
            preceding_reputation: &self.preceding_reputation,
        };
        let mut raw = Vec::new();
        for carrier in self
            .finalized_authorizations
            .iter()
            .filter(|carrier| carrier.shard_id == shard_id)
        {
            if let Some(claim) = policy.extract_valid_claim(carrier, shard_id)? {
                raw.push(claim);
            }
        }
        self.with_authorized_equivocations(&raw, blocklace, output, membership, shard_id)
    }

    /// Verify an explicit proof list and require exact equality with the retained
    /// finalized set. Duplicate publications and proof arrival order have no effect.
    pub fn with_authorized_equivocations(
        mut self,
        raw: &[CordialEquivocationEvidence],
        blocklace: &Blocklace,
        output: &OrderedFinalizedOutput,
        membership: &HashMap<NodeId, u64>,
        shard_id: &[u8],
    ) -> Result<Self, PorError> {
        let policy = FinalizedCheckpointAuthorization {
            blocklace,
            output,
            checkpoint: self.checkpoint(),
            membership,
            preceding_reputation: &self.preceding_reputation,
        };
        // Re-extract from the admitting node's trusted DAG, including for an
        // empty submission. A caller's completed snapshot cannot hide carriers
        // by supplying different content under the same output identities.
        if finalized_checkpoint_authorizations(blocklace, output, self.checkpoint())?
            != self.finalized_authorizations
        {
            return Err(invalid(
                "retained authorizations differ from finalized checkpoint contents",
            ));
        }
        let context = CheckpointEquivocationAuthorizationContext {
            shard_id: shard_id.to_vec(),
            source: self.source(),
        };

        // Determine the complete set of valid authorizations finalized in this window.
        let mut valid_authorizations = BTreeSet::new();
        for carrier in self
            .finalized_authorizations
            .iter()
            .filter(|carrier| carrier.shard_id == shard_id)
        {
            if policy.extract_valid_claim(carrier, shard_id)?.is_some() {
                valid_authorizations.insert(carrier.clone());
            }
        }

        let mut selected = BTreeMap::new();
        let mut admitted = BTreeSet::new();
        for claim in raw {
            let proof = verify_equivocation_evidence(claim, blocklace).map_err(invalid)?;
            let message = checkpoint_equivocation_authorization_payload(
                shard_id,
                context.source.round,
                &proof,
            )?;
            let event =
                EquivocationPenalty::authorize_checkpoint(&proof, context.clone(), &policy)?;
            admitted.insert(PorEquivocationAuthorization {
                shard_id: shard_id.to_vec(),
                source_finalized_wave: 0,
                round: context.source.round,
                message: message.clone(),
            });
            // Authorization checks preceding-state exclusion, not weight.
            selected.insert(message, event);
        }

        // Completeness check: caller-admitted authorizations must match
        // exactly the valid authorizations finalized in this checkpoint window.
        if admitted != valid_authorizations {
            return Err(invalid(
                "checkpoint penalties omit or add finalized authorizations",
            ));
        }

        let mut by_offender = BTreeMap::new();
        for event in selected.into_values() {
            by_offender.entry(event.offender.clone()).or_insert(event);
        }
        let events = ReputationPenaltyEvents {
            round: context.source.round,
            equivocations: by_offender.into_values().collect(),
            inactivity: vec![],
        };
        penalty_events_commitment(events.round, Some(&events))?;
        self.penalties = Some(events);
        self.admitted_authorizations = admitted;
        // Keep the original signed carrier set immutable for runtime reauthorization.
        // Ignored claims are derived here; callers cannot omit valid claims by
        // supplying their own filtering decisions.
        self.ignored_authorizations = self
            .finalized_authorizations
            .iter()
            .filter(|carrier| {
                carrier.shard_id == shard_id && !valid_authorizations.contains(*carrier)
            })
            .cloned()
            .collect();
        self.validate_complete_authorizations(shard_id)?;
        Ok(self)
    }
}
