//! Finalized Cordial authorization for PoR equivocation penalties.
//!
//! Evidence-pool entries are untrusted claims. The authorization payload must
//! be signed by an authorized validator and included in the trusted finalized
//! output for this PoR source wave. Checkpoint attestations alone do not
//! authenticate a proof or authorize its inclusion.
//!
//! Hosts retain raw proofs and complete ancestry so receivers can repeat
//! verification. Authority is never decoded from peer bytes.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use cordial_miners_core::{
    Blocklace, NodeId,
    consensus::{
        CordialEquivocationEvidence, VerifiedEquivocationEvidence, depth,
        verify_equivocation_evidence, wave_of_round,
    },
    crypto,
    execution::{CordialBlockPayload, PorEquivocationAuthorization},
};
use cordial_por::{
    EquivocationAuthorizationContext, EquivocationAuthorizationPolicy, EquivocationPenalty,
    PorError, ReputationPenaltyEvents, equivocation_authorization_payload,
    penalty_events_commitment,
};

use crate::ordered_output::OrderedFinalizedOutput;

use super::{
    finality::FinalizedRatingRound, interactions::validate_finalized_rating_round,
    lifecycle::CompletedPorRatingRound,
};

/// Policy over trusted Cordial finalized output and membership.
/// Both inputs must come from the adapter's consensus path, never from a peer.
pub struct FinalizedEquivocationAuthorization<'a> {
    blocklace: &'a Blocklace,
    output: &'a OrderedFinalizedOutput,
    opened: FinalizedRatingRound,
    authorized_validators: &'a HashMap<NodeId, u64>,
}

impl<'a> FinalizedEquivocationAuthorization<'a> {
    pub fn new(
        blocklace: &'a Blocklace,
        output: &'a OrderedFinalizedOutput,
        opened: FinalizedRatingRound,
        authorized_validators: &'a HashMap<NodeId, u64>,
    ) -> Result<Self, PorError> {
        validate_finalized_rating_round(blocklace, output, opened)
            .map_err(|error| PorError::InvalidPenaltyEvents(error.to_string()))?;
        Ok(Self {
            blocklace,
            output,
            opened,
            authorized_validators,
        })
    }
}

impl EquivocationAuthorizationPolicy for FinalizedEquivocationAuthorization<'_> {
    fn authorize(
        &self,
        context: &EquivocationAuthorizationContext,
        evidence: &VerifiedEquivocationEvidence,
    ) -> Result<(), PorError> {
        let invalid = |message: &str| PorError::InvalidPenaltyEvents(message.into());
        if context.round != self.opened.rating_round
            || context.source_finalized_wave != self.opened.finalized_wave
            || self
                .authorized_validators
                .get(evidence.validator())
                .copied()
                .unwrap_or(0)
                == 0
        {
            return Err(invalid(
                "equivocation is outside the authorized validator/round context",
            ));
        }
        for block in evidence.blocks() {
            let round = cordial_miners_core::consensus::round::candidate_depth(
                self.blocklace,
                &block.content,
            )
            .ok_or_else(|| invalid("equivocation ancestry is unavailable"))?;
            if wave_of_round(round, self.output.wavelength)
                .is_none_or(|wave| wave > self.opened.finalized_wave)
            {
                return Err(invalid(
                    "equivocation is newer than the finalized source wave",
                ));
            }
        }
        let payload = equivocation_authorization_payload(context, evidence)?;
        for reference in &self.output.blocks {
            let block = self
                .blocklace
                .get(reference)
                .ok_or_else(|| invalid("finalized authorization block is unavailable"))?;
            let authorizations =
                CordialBlockPayload::por_authorizations_from_bytes(&block.content.payload)
                    .map_err(PorError::InvalidPenaltyEvents)?;
            if !authorizations.iter().any(|authorization| {
                authorization.shard_id == context.shard_id
                    && authorization.source_finalized_wave == context.source_finalized_wave
                    && authorization.round == context.round
                    && authorization.message == payload
            }) {
                continue;
            }
            let block_round = depth(self.blocklace, &block.identity)
                .ok_or_else(|| invalid("authorization block ancestry is unavailable"))?;
            if wave_of_round(block_round, self.output.wavelength)
                != Some(self.opened.finalized_wave)
                || self
                    .authorized_validators
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
                return Err(invalid(
                    "invalid finalized equivocation authorization block",
                ));
            }
            return Ok(());
        }
        Err(invalid(
            "verified equivocation has no finalized authorization payload",
        ))
    }
}

/// Snapshot every carrier in this trusted finalized source wave. No caller-
/// supplied proof list participates in selecting the required messages.
pub(super) fn finalized_authorizations(
    blocklace: &Blocklace,
    output: &OrderedFinalizedOutput,
    opened: FinalizedRatingRound,
) -> Result<BTreeSet<PorEquivocationAuthorization>, PorError> {
    let mut messages = BTreeSet::new();
    for reference in &output.blocks {
        let block = blocklace.get(reference).ok_or_else(|| {
            PorError::InvalidPenaltyEvents("finalized block is unavailable".into())
        })?;
        let carriers = CordialBlockPayload::por_authorizations_from_bytes(&block.content.payload)
            .map_err(PorError::InvalidPenaltyEvents)?;
        if carriers.is_empty() {
            continue;
        }
        let round = depth(blocklace, &block.identity).ok_or_else(|| {
            PorError::InvalidPenaltyEvents("authorization ancestry is unavailable".into())
        })?;
        if wave_of_round(round, output.wavelength) == Some(opened.finalized_wave) {
            messages.extend(carriers.into_iter().filter(|carrier| {
                !carrier
                    .message
                    .starts_with(cordial_por::POR_CHECKPOINT_EQUIVOCATION_AUTHORIZATION_DOMAIN)
                    && carrier.source_finalized_wave == opened.finalized_wave
                    && carrier.round == opened.rating_round
            }));
        }
    }
    Ok(messages)
}

impl CompletedPorRatingRound {
    /// Verify raw retained evidence and require finalized protocol authorization.
    /// Admission is atomic. One immutable event set drives local construction,
    /// checkpoint replay, durable application, and weight export.
    pub fn with_authorized_equivocations(
        mut self,
        raw: &[CordialEquivocationEvidence],
        blocklace: &Blocklace,
        output: &OrderedFinalizedOutput,
        authorized_validators: &HashMap<NodeId, u64>,
        shard_id: &[u8],
    ) -> Result<Self, PorError> {
        let policy = FinalizedEquivocationAuthorization::new(
            blocklace,
            output,
            self.opened(),
            authorized_validators,
        )?;
        let context = EquivocationAuthorizationContext {
            shard_id: shard_id.to_vec(),
            source_finalized_wave: self.opened().finalized_wave,
            round: self.batch().round,
        };
        let mut selected = BTreeMap::new();
        let mut admitted = BTreeSet::new();
        for claim in raw {
            let proof = verify_equivocation_evidence(claim, blocklace)
                .map_err(|error| PorError::InvalidPenaltyEvents(error.to_string()))?;
            let message = equivocation_authorization_payload(&context, &proof)?;
            let event = EquivocationPenalty::authorize(&proof, context.clone(), &policy)?;
            admitted.insert(PorEquivocationAuthorization {
                shard_id: context.shard_id.clone(),
                source_finalized_wave: context.source_finalized_wave,
                round: context.round,
                message: message.clone(),
            });
            selected.insert(message, event);
        }
        // Verify every finalized message, but penalize an offender only once.
        // If several proofs name the same offender, choose the lexicographically
        // first canonical message, independent of arrival/proof-list order.
        let mut by_offender = BTreeMap::new();
        for event in selected.into_values() {
            by_offender.entry(event.offender.clone()).or_insert(event);
        }
        let equivocations = by_offender.into_values().collect();
        let events = ReputationPenaltyEvents {
            round: self.batch().round,
            equivocations,
            inactivity: Vec::new(),
        };
        penalty_events_commitment(events.round, Some(&events))?;
        self.set_authorized_penalties(events, admitted);
        self.validate_complete_authorizations(shard_id)?;
        Ok(self)
    }
}
