//! Canonical, domain-separated commitments for reputation protocol data.
//!
//! Every variable-width field is length-prefixed and every integer is encoded
//! big-endian. The domain strings include a format version so a future encoding
//! cannot accidentally validate under this one.

use cordial_miners_core::crypto::{Blake2b256Hasher, Hasher};

use crate::{
    block::validate_reputation_block,
    config::{MissingEntryPolicy, PorConfig},
    error::PorError,
    ratings::{build_rating_batch, canonical_rating_payload},
    types::{RatingBatch, ReputationBlock, ReputationCommitment, ReputationList, ReputationVector},
};

/// V2 includes all slash and inactivity parameters; legacy v1 hashes are not
/// interchangeable with this replay policy.
pub const POR_CONFIG_COMMITMENT_DOMAIN: &[u8] = b"cordial-por:config-commitment:v2";
pub const POR_RATING_BATCH_COMMITMENT_DOMAIN: &[u8] = b"cordial-por:rating-batch-commitment:v1";
pub const POR_REPUTATION_LIST_COMMITMENT_DOMAIN: &[u8] =
    b"cordial-por:reputation-list-commitment:v2";
pub const POR_REPUTATION_BLOCK_COMMITMENT_DOMAIN: &[u8] =
    b"cordial-por:reputation-block-commitment:v2";

/// Commit to every protocol parameter consumed by reputation replay.
pub fn config_commitment(config: &PorConfig) -> ReputationCommitment {
    let mut payload = Vec::with_capacity(POR_CONFIG_COMMITMENT_DOMAIN.len() + 8 * 8 + 1);
    payload.extend_from_slice(POR_CONFIG_COMMITMENT_DOMAIN);
    payload.extend_from_slice(&config.scale.to_be_bytes());
    payload.extend_from_slice(&config.initial_reputation.to_be_bytes());
    payload.extend_from_slice(&config.liquid_rank_alpha.to_be_bytes());
    payload.extend_from_slice(&config.minimum_rating.to_be_bytes());
    payload.extend_from_slice(&config.maximum_rating.to_be_bytes());
    payload.push(match config.missing_entry_policy {
        MissingEntryPolicy::Reject => 0,
        MissingEntryPolicy::CarryForward => 1,
        MissingEntryPolicy::Neutral => 2,
    });
    payload.extend_from_slice(&config.correlation_threshold.to_be_bytes());
    payload.extend_from_slice(&config.base_slash_penalty.to_be_bytes());
    payload.extend_from_slice(&config.inactivity_decay_gamma.to_be_bytes());
    Blake2b256Hasher.hash(&payload)
}

/// Commit to a logical rating batch in canonical `(recipient, rater)` order.
///
/// Signatures are included in addition to the canonical signed payload, so the
/// commitment binds both the rating semantics and the exact attestations.
pub fn rating_batch_commitment(
    batch: &RatingBatch,
    config: &PorConfig,
) -> Result<ReputationCommitment, PorError> {
    let canonical = build_rating_batch(batch.round, batch.ratings.clone(), config)?;
    let mut payload = Vec::new();
    payload.extend_from_slice(POR_RATING_BATCH_COMMITMENT_DOMAIN);
    payload.extend_from_slice(&canonical.round.to_be_bytes());
    put_len(&mut payload, canonical.ratings.len())?;

    for rating in &canonical.ratings {
        put_bytes(&mut payload, &canonical_rating_payload(rating))?;
        put_bytes(&mut payload, &rating.signature)?;
    }

    Ok(Blake2b256Hasher.hash(&payload))
}

/// Penalty events are sorted by offender within each event category. The domain,
/// round, category counts, offender IDs, exact evidence bytes and missed-round
/// counts are all bound. None and an explicit empty set have the same encoding.
pub const POR_PENALTY_EVENTS_COMMITMENT_DOMAIN: &[u8] = b"cordial-por:penalty-events-commitment:v1";
pub const MAX_PENALTY_EVIDENCE_LEN: usize = 64 * 1024;

pub fn penalty_events_commitment(
    round: u64,
    events: Option<&crate::types::ReputationPenaltyEvents>,
) -> Result<ReputationCommitment, PorError> {
    use crate::block::{
        MAX_REPUTATION_BLOCK_ENTRIES, MAX_REPUTATION_BLOCK_NODE_ID_LEN,
        MAX_REPUTATION_BLOCK_WIRE_LEN,
    };
    use std::collections::BTreeSet;
    let mut equivocations: Vec<&crate::types::EquivocationPenalty> = Vec::new();
    let mut inactivity: Vec<&crate::types::InactivityPenalty> = Vec::new();
    if let Some(events) = events {
        if events.round != round {
            return Err(PorError::InvalidPenaltyEvents(
                "event round must match the block".into(),
            ));
        }
        if events
            .equivocations
            .len()
            .saturating_add(events.inactivity.len())
            > MAX_REPUTATION_BLOCK_ENTRIES
        {
            return Err(PorError::InvalidPenaltyEvents(
                "too many penalty events".into(),
            ));
        }
        equivocations.extend(events.equivocations.iter());
        inactivity.extend(events.inactivity.iter());
    }
    equivocations.sort_by(|a, b| a.offender.cmp(&b.offender));
    inactivity.sort_by(|a, b| a.offender.cmp(&b.offender));
    let mut seen = BTreeSet::new();
    let mut payload = POR_PENALTY_EVENTS_COMMITMENT_DOMAIN.to_vec();
    payload.extend_from_slice(&round.to_be_bytes());
    put_len(&mut payload, equivocations.len())?;
    for event in equivocations {
        if !seen.insert(&event.offender)
            || event.offender.0.len() > MAX_REPUTATION_BLOCK_NODE_ID_LEN
            || event.evidence.is_empty()
            || event.evidence.len() > MAX_PENALTY_EVIDENCE_LEN
        {
            return Err(PorError::InvalidPenaltyEvents(
                "duplicate offender or invalid evidence/identifier size".into(),
            ));
        }
        put_bytes(&mut payload, &event.offender.0)?;
        put_bytes(&mut payload, &event.evidence)?;
        if payload.len() > MAX_REPUTATION_BLOCK_WIRE_LEN {
            return Err(PorError::InvalidPenaltyEvents(
                "penalty payload too large".into(),
            ));
        }
    }
    put_len(&mut payload, inactivity.len())?;
    for event in inactivity {
        if !seen.insert(&event.offender)
            || event.offender.0.len() > MAX_REPUTATION_BLOCK_NODE_ID_LEN
            || event.missed_rounds != 1
        {
            return Err(PorError::InvalidPenaltyEvents(
                "inactivity requires a unique offender and one missed round".into(),
            ));
        }
        put_bytes(&mut payload, &event.offender.0)?;
        payload.extend_from_slice(&event.missed_rounds.to_be_bytes());
        if payload.len() > MAX_REPUTATION_BLOCK_WIRE_LEN {
            return Err(PorError::InvalidPenaltyEvents(
                "penalty payload too large".into(),
            ));
        }
    }
    if payload.len() > MAX_REPUTATION_BLOCK_WIRE_LEN {
        return Err(PorError::InvalidPenaltyEvents(
            "penalty payload too large".into(),
        ));
    }
    Ok(Blake2b256Hasher.hash(&payload))
}

/// Commit to a canonically ordered reputation list.
pub fn reputation_list_commitment(list: &ReputationList) -> Result<ReputationCommitment, PorError> {
    validate_reputation_entries(&list.entries)?;

    let mut payload = Vec::new();
    payload.extend_from_slice(POR_REPUTATION_LIST_COMMITMENT_DOMAIN);
    payload.extend_from_slice(&list.round.to_be_bytes());
    put_len(&mut payload, list.entries.len())?;

    for entry in &list.entries {
        put_bytes(&mut payload, &entry.node_id.0)?;
        payload.extend_from_slice(&entry.reputation.to_be_bytes());
        payload.push(u8::from(entry.is_excluded));
        payload.extend_from_slice(&entry.retained_reputation.to_be_bytes());
    }

    Ok(Blake2b256Hasher.hash(&payload))
}

/// Hash a structurally valid reputation block header.
///
/// The list is transitively covered by `reputation_root`, which structural
/// validation recomputes before the header hash is returned.
pub fn reputation_block_hash(block: &ReputationBlock) -> Result<ReputationCommitment, PorError> {
    validate_reputation_block(block)?;

    let header = &block.header;
    let mut payload = Vec::new();
    payload.extend_from_slice(POR_REPUTATION_BLOCK_COMMITMENT_DOMAIN);
    payload.extend_from_slice(&header.version.to_be_bytes());
    put_bytes(&mut payload, &header.shard_id)?;
    payload.extend_from_slice(&header.source_finalized_wave.to_be_bytes());
    payload.extend_from_slice(&header.round.to_be_bytes());
    match header.previous_reputation_hash {
        Some(previous) => {
            payload.push(1);
            payload.extend_from_slice(&previous);
        }
        None => payload.push(0),
    }
    payload.extend_from_slice(&header.config_hash);
    payload.extend_from_slice(&header.ratings_hash);
    payload.extend_from_slice(&header.penalties_hash);
    payload.extend_from_slice(&header.reputation_root);

    Ok(Blake2b256Hasher.hash(&payload))
}

pub(crate) fn validate_reputation_entries(
    entries: &[crate::types::ReputationEntry],
) -> Result<(), PorError> {
    for entry in entries {
        if (entry.is_excluded && entry.reputation != 0)
            || (!entry.is_excluded && entry.retained_reputation != 0)
        {
            return Err(PorError::ReputationExclusionMismatch);
        }
    }
    for pair in entries.windows(2) {
        match pair[0].node_id.cmp(&pair[1].node_id) {
            std::cmp::Ordering::Less => {}
            std::cmp::Ordering::Equal => return Err(PorError::DuplicateReputationEntry),
            std::cmp::Ordering::Greater => return Err(PorError::UnsortedReputationVector),
        }
    }

    Ok(())
}

pub(crate) fn validate_reputation_vector(vector: &ReputationVector) -> Result<(), PorError> {
    validate_reputation_entries(&vector.values)
}

fn put_len(output: &mut Vec<u8>, len: usize) -> Result<(), PorError> {
    let len = u64::try_from(len).map_err(|_| PorError::CommitmentLengthOverflow)?;
    output.extend_from_slice(&len.to_be_bytes());
    Ok(())
}

fn put_bytes(output: &mut Vec<u8>, bytes: &[u8]) -> Result<(), PorError> {
    put_len(output, bytes.len())?;
    output.extend_from_slice(bytes);
    Ok(())
}
