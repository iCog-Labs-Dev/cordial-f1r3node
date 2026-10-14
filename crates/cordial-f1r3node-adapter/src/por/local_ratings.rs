//! Stage one validator's next signed rating batch from finalized evidence.
//!
//! The caller supplies the finalized output for the durable state's next wave.
//! A later output must first be replayed to that wave through `por::finality`.

use std::collections::HashSet;

use anyhow::{Context, Result, ensure};
use cordial_miners_core::{Blocklace, NodeId};
use cordial_por::{PorConfig, ReputationState, rating_round_from_finalized_wave};

use crate::ordered_output::OrderedFinalizedOutput;

use super::{
    collector::BlockProductionRatingCollector,
    finality::{FinalizedRatingRound, finalized_wave_from_output},
    lifecycle::{PorRatingRoundCoordinator, quorum::PorRatingRoundClosurePolicy},
    rating_key::BondedRatingKey,
    transport::{file_exchange::FileRatingExchange, outbox::RatingOutbox, receive_rating_envelope},
};

/// The local batch staged for future peer delivery. No PoR round is committed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StagedLocalRatingBatch {
    pub finalized_wave: u64,
    pub rating_round: u64,
    pub rating_count: usize,
}

/// Evidence-checked exchange progress. This does not close or commit a round.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RatingExchangeProgress {
    pub finalized_wave: u64,
    pub rating_round: u64,
    pub local_envelopes: usize,
    pub peer_envelopes: usize,
    pub eligible_raters: usize,
    pub completed_raters: usize,
    pub quorum_reached: bool,
    pub all_raters_complete: bool,
}

/// Sign and durably stage only the next rating round expected by `state`.
///
/// Repeated calls for the same finalized output are idempotent through the
/// outbox. A changed batch for the same wave fails before replacing it.
pub fn stage_local_rating_batch(
    blocklace: &Blocklace,
    output: &OrderedFinalizedOutput,
    state: &ReputationState,
    config: &PorConfig,
    key: &BondedRatingKey,
    outbox: &RatingOutbox,
) -> Result<Option<StagedLocalRatingBatch>> {
    let Some(finalized_wave) = finalized_wave_from_output(blocklace, output)? else {
        return Ok(None);
    };
    let committed_round = state.round();
    if finalized_wave < committed_round {
        return Ok(None);
    }
    ensure!(
        finalized_wave == committed_round,
        "cannot stage PoR rating round after finalized wave gap: committed round {committed_round}, observed wave {finalized_wave}"
    );

    let rating_round = rating_round_from_finalized_wave(finalized_wave)?;
    let opened = FinalizedRatingRound {
        finalized_wave,
        rating_round,
    };
    let mut coordinator = PorRatingRoundCoordinator::new(blocklace, output, opened, state, config)?;
    let batch = coordinator.produce_local_batch(key.rater(), key.private_key_bytes())?;
    outbox.persist_local_batch(finalized_wave, key.rater(), batch)?;

    Ok(Some(StagedLocalRatingBatch {
        finalized_wave,
        rating_round,
        rating_count: batch.ratings.len(),
    }))
}

/// Publish the staged batch and replay all available peer envelopes.
///
/// Every local and peer envelope is checked against the same finalized
/// evidence before completeness is reported. The durable outbox is the source
/// for retries after a restart; a partial publication can be safely retried.
pub fn observe_rating_exchange(
    blocklace: &Blocklace,
    output: &OrderedFinalizedOutput,
    state: &ReputationState,
    config: &PorConfig,
    key: &BondedRatingKey,
    authorized: &HashSet<NodeId>,
    outbox: &RatingOutbox,
    exchange: &FileRatingExchange,
) -> Result<RatingExchangeProgress> {
    let finalized_wave = finalized_wave_from_output(blocklace, output)?
        .context("cannot exchange ratings without a finalized leader")?;
    ensure!(
        finalized_wave == state.round(),
        "PoR rating exchange wave does not match the durable reputation round"
    );
    let rating_round = rating_round_from_finalized_wave(finalized_wave)?;
    let opened = FinalizedRatingRound {
        finalized_wave,
        rating_round,
    };
    let local = outbox
        .load(finalized_wave, key.rater())?
        .context("staged local PoR rating batch is missing")?;
    let mut collector =
        BlockProductionRatingCollector::new(blocklace, output, opened, state, config)?;
    for envelope in &local {
        receive_rating_envelope(&mut collector, envelope)?;
    }

    let broadcaster = exchange.broadcaster(finalized_wave, key.rater().clone());
    for envelope in &local {
        broadcaster.publish(envelope)?;
    }
    let peer = exchange.read_peer_envelopes(finalized_wave, key.rater(), authorized)?;
    for envelope in &peer {
        receive_rating_envelope(&mut collector, envelope)?;
    }
    let progress = PorRatingRoundClosurePolicy::default().evaluate(&collector)?;
    Ok(RatingExchangeProgress {
        finalized_wave,
        rating_round,
        local_envelopes: local.len(),
        peer_envelopes: peer.len(),
        eligible_raters: progress.eligible_raters,
        completed_raters: progress.completed_raters.len(),
        quorum_reached: progress.is_reached(),
        all_raters_complete: progress.completed_raters.len() == progress.eligible_raters,
    })
}
