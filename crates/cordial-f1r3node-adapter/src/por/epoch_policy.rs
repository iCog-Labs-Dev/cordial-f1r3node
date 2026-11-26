//! Deterministic effective-wave calculation for checkpoint PoR weights.
//!
//! A deployment must agree on the delay before ratings are exchanged. The
//! current local head or the time at which an observer receives ratings must
//! never move the cutover; a late proposal is rejected by the ingress preview.

use cordial_miners_core::{
    Blocklace,
    consensus::{depth, wave_of_round},
};
use thiserror::Error;

use crate::ordered_output::OrderedFinalizedOutput;

use super::finality::{
    FinalizedPrefixCheckpoint, PorFinalityError, validate_finalized_prefix_checkpoint,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PorWeightEpochPolicy {
    delay_waves: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum PorWeightEpochPolicyError {
    #[error("PoR weight epoch delay must be positive")]
    ZeroDelay,
    #[error("PoR weight epoch source is not in the published finalized prefix: {0}")]
    Source(#[from] PorFinalityError),
    #[error("PoR weight epoch source terminal block is absent from the local blocklace")]
    UnknownTerminalBlock,
    #[error("PoR weight epoch effective wave overflowed")]
    WaveOverflow,
}

impl PorWeightEpochPolicy {
    pub fn new(delay_waves: u64) -> Result<Self, PorWeightEpochPolicyError> {
        if delay_waves == 0 {
            return Err(PorWeightEpochPolicyError::ZeroDelay);
        }
        Ok(Self { delay_waves })
    }

    pub fn delay_waves(&self) -> u64 {
        self.delay_waves
    }

    /// Derive a cutover from replayable source identity and shared policy.
    ///
    /// The terminal block's Cordial wave is stable across later finalized
    /// outputs that retain the source prefix. Whether the derived wave is still
    /// available for activation is checked separately by `LiveIngress`.
    pub fn effective_wave(
        &self,
        blocklace: &Blocklace,
        output: &OrderedFinalizedOutput,
        source: &FinalizedPrefixCheckpoint,
    ) -> Result<u64, PorWeightEpochPolicyError> {
        validate_finalized_prefix_checkpoint(output, source)?;
        let round = depth(blocklace, &source.terminal_block)
            .ok_or(PorWeightEpochPolicyError::UnknownTerminalBlock)?;
        let source_wave = wave_of_round(round, output.wavelength).ok_or(
            PorWeightEpochPolicyError::Source(PorFinalityError::InvalidWavelength),
        )?;
        source_wave
            .checked_add(self.delay_waves)
            .ok_or(PorWeightEpochPolicyError::WaveOverflow)
    }
}
