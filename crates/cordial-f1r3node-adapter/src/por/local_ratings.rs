//! Stage one validator's next signed rating batch from finalized evidence.
//!
//! The latest finalized output may jump across waves. Until per-wave replay is
//! available, a jump beyond the next durable reputation round fails closed.

use anyhow::{Result, ensure};
use cordial_miners_core::Blocklace;
use cordial_por::{PorConfig, ReputationState, rating_round_from_finalized_wave};

use crate::ordered_output::OrderedFinalizedOutput;

use super::{
    finality::{FinalizedRatingRound, finalized_wave_from_output},
    lifecycle::PorRatingRoundCoordinator,
    rating_key::BondedRatingKey,
    transport::outbox::RatingOutbox,
};

/// The local batch staged for future peer delivery. No PoR round is committed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StagedLocalRatingBatch {
    pub finalized_wave: u64,
    pub rating_round: u64,
    pub rating_count: usize,
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
