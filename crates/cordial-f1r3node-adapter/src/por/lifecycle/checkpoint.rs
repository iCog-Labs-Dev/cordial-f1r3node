//! Closed checkpoint rating round for the all-rater PoC path.

use cordial_miners_core::execution::PorEquivocationAuthorization;
use cordial_por::{
    PorError, RatingBatch, RatingSourceCheckpoint, ReputationList, ReputationPenaltyEvents,
    ReputationState,
};
use std::collections::BTreeSet;

use crate::por::finality::FinalizedPrefixCheckpoint;

/// A complete, evidence-checked rating batch for one finalized-prefix source.
///
/// Construction is restricted to the exchange closure path. This value has
/// not yet committed a reputation transition or activated Cordial weights.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompletedCheckpointRatingRound {
    checkpoint: FinalizedPrefixCheckpoint,
    batch: RatingBatch,
    pub(in crate::por) finalized_authorizations: BTreeSet<PorEquivocationAuthorization>,
    pub(in crate::por) admitted_authorizations: BTreeSet<PorEquivocationAuthorization>,
    pub(in crate::por) ignored_authorizations: BTreeSet<PorEquivocationAuthorization>,
    pub(in crate::por) preceding_reputation: ReputationList,
    pub(in crate::por) penalties: Option<ReputationPenaltyEvents>,
}

impl CompletedCheckpointRatingRound {
    pub(crate) fn new(
        checkpoint: FinalizedPrefixCheckpoint,
        batch: RatingBatch,
        finalized_authorizations: BTreeSet<PorEquivocationAuthorization>,
        state: &ReputationState,
    ) -> Self {
        Self {
            checkpoint,
            batch,
            finalized_authorizations,
            admitted_authorizations: BTreeSet::new(),
            ignored_authorizations: BTreeSet::new(),
            preceding_reputation: state.reputation_list().clone(),
            penalties: None,
        }
    }

    pub(in crate::por) fn validate_preceding_state(
        &self,
        state: &ReputationState,
    ) -> Result<(), PorError> {
        if &self.preceding_reputation != state.reputation_list() {
            return Err(PorError::InvalidPenaltyEvents(
                "checkpoint authorization used a different preceding state".into(),
            ));
        }
        Ok(())
    }

    pub fn penalties(&self) -> Option<&ReputationPenaltyEvents> {
        self.penalties.as_ref()
    }

    pub(in crate::por) fn validate_complete_authorizations(
        &self,
        shard_id: &[u8],
    ) -> Result<(), PorError> {
        let expected: BTreeSet<_> = self
            .finalized_authorizations
            .iter()
            .filter(|carrier| carrier.shard_id == shard_id)
            .cloned()
            .collect();
        let accounted: BTreeSet<_> = self
            .admitted_authorizations
            .union(&self.ignored_authorizations)
            .cloned()
            .collect();
        if !self
            .admitted_authorizations
            .is_disjoint(&self.ignored_authorizations)
            || expected != accounted
        {
            return Err(PorError::InvalidPenaltyEvents(
                "checkpoint penalties omit or add finalized authorizations".into(),
            ));
        }
        Ok(())
    }

    pub fn checkpoint(&self) -> &FinalizedPrefixCheckpoint {
        &self.checkpoint
    }

    pub fn source(&self) -> RatingSourceCheckpoint {
        self.checkpoint.rating_source()
    }

    pub fn batch(&self) -> &RatingBatch {
        &self.batch
    }
}
