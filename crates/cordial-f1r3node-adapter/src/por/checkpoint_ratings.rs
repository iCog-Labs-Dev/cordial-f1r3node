//! Durable local staging and all-rater closure for checkpoint-bound ratings.
//!
//! This PoC path closes only when every non-ejected authorized rater's deterministic
//! evidence-backed batch is complete. It has no wall-clock cutoff.

use std::collections::HashSet;

use anyhow::{Context, Result, ensure};
use cordial_miners_core::{Blocklace, NodeId};
use cordial_por::{PorConfig, ReputationState};

use crate::ordered_output::OrderedFinalizedOutput;

use super::{
    collector::CheckpointBlockProductionRatingCollector,
    finality::FinalizedPrefixCheckpoint,
    lifecycle::checkpoint::CompletedCheckpointRatingRound,
    rating_key::BondedRatingKey,
    ratings::build_finalized_checkpoint_rating_batch,
    transport::{
        encode_checkpoint_rating_batch, file_exchange::FileRatingExchange, outbox::RatingOutbox,
        receive_checkpoint_rating_envelope,
    },
};

/// Sign and stage the next checkpoint batch before any peer delivery.
pub fn stage_local_checkpoint_rating_batch(
    blocklace: &Blocklace,
    output: &OrderedFinalizedOutput,
    checkpoint: &FinalizedPrefixCheckpoint,
    state: &ReputationState,
    config: &PorConfig,
    key: &BondedRatingKey,
    outbox: &RatingOutbox,
) -> Result<usize> {
    super::interactions::validate_finalized_checkpoint_window(blocklace, output, checkpoint)?;
    ensure!(
        state.round().checked_add(1) == Some(checkpoint.por_round),
        "checkpoint does not follow the preceding state"
    );
    // Ejected observers can still receive and audit checkpoints, but never sign.
    if state.is_ejected(key.rater()) {
        return Ok(0);
    }
    let batch = build_finalized_checkpoint_rating_batch(
        blocklace,
        output,
        checkpoint,
        key.rater(),
        state,
        config,
        key.private_key_bytes(),
    )?;
    outbox.persist_checkpoint_batch(checkpoint.rating_source(), key.rater(), &batch)?;
    Ok(batch.ratings.len())
}

/// Publish the staged local batch and close only after all non-ejected authorized raters
/// have supplied every rating required by the canonical checkpoint window.
///
/// Each call rebuilds the candidate from durable local bytes and the shared
/// exchange. A partial peer prefix leaves the round open; malformed or
/// conflicting evidence fails closed. The returned value is still uncommitted.
#[allow(clippy::too_many_arguments)]
pub fn close_checkpoint_rating_exchange_if_complete(
    blocklace: &Blocklace,
    output: &OrderedFinalizedOutput,
    checkpoint: &FinalizedPrefixCheckpoint,
    state: &ReputationState,
    config: &PorConfig,
    key: &BondedRatingKey,
    authorized: &HashSet<NodeId>,
    outbox: &RatingOutbox,
    exchange: &FileRatingExchange,
) -> Result<Option<CompletedCheckpointRatingRound>> {
    ensure!(
        authorized.contains(key.rater()),
        "local PoR checkpoint rater is not authorized"
    );
    let source = checkpoint.rating_source();
    let eligible: HashSet<_> = authorized
        .iter()
        .filter(|rater| !state.is_ejected(rater))
        .cloned()
        .collect();
    let mut collector = CheckpointBlockProductionRatingCollector::new(
        blocklace,
        output,
        checkpoint.clone(),
        authorized.iter().cloned(),
        state,
        config,
    )?;
    if eligible.contains(key.rater()) {
        let signed = build_finalized_checkpoint_rating_batch(
            blocklace,
            output,
            checkpoint,
            key.rater(),
            state,
            config,
            key.private_key_bytes(),
        )?;
        let expected = encode_checkpoint_rating_batch(source, &signed)?;
        let local = outbox
            .load_checkpoint_batch(source, key.rater())?
            .context("staged local PoR checkpoint rating batch is missing")?;
        ensure!(
            local == expected,
            "staged local PoR checkpoint ratings differ from finalized evidence"
        );
        for envelope in &local {
            receive_checkpoint_rating_envelope(&mut collector, envelope)?;
        }
        let broadcaster = exchange.checkpoint_broadcaster(source, key.rater().clone())?;
        for envelope in &local {
            broadcaster.publish(envelope)?;
        }
    }
    for envelope in exchange.read_checkpoint_peer_envelopes_for_eligible(
        source,
        key.rater(),
        authorized,
        &eligible,
    )? {
        receive_checkpoint_rating_envelope(&mut collector, &envelope)?;
    }

    if !collector.all_authorized_raters_complete()? {
        return Ok(None);
    }
    let batch = collector.build_complete_batch()?;
    Ok(Some(CompletedCheckpointRatingRound::new(
        checkpoint.clone(),
        batch,
        super::checkpoint_evidence::finalized_checkpoint_authorizations(
            blocklace, output, checkpoint,
        )?,
        state,
    )))
}
