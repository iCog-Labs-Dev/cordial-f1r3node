//! Read-only consumer boundary for exported Cordial ordered output.
//!
//! `OrderedFinalizedOutput` is the stable data model. This module adds the
//! first in-process consumer seam: a small container that stores the latest
//! output and exposes it through a read-only trait.

use std::time::{SystemTime, UNIX_EPOCH};

use crate::ordered_output::OrderedFinalizedOutput;

/// Error returned when a shared ordered-output update would violate the
/// append-only prefix contract.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SharedOrderedOutputError {
    PrefixViolation,
}

impl std::fmt::Display for SharedOrderedOutputError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::PrefixViolation => {
                write!(f, "ordered output update does not preserve previous prefix")
            }
        }
    }
}

impl std::error::Error for SharedOrderedOutputError {}

/// Read-only access to the latest finalized ordered output.
pub trait ReadOrderedOutput {
    /// Return the latest computed ordered output, or `None` if no output has
    /// been published into this reader yet.
    fn latest(&self) -> Option<&OrderedFinalizedOutput>;

    /// Return the latest output's anchor hash, or `None` if no output or no
    /// anchor exists.
    fn anchor_hash(&self) -> Option<Vec<u8>> {
        self.latest().and_then(OrderedFinalizedOutput::anchor_hash)
    }

    /// Return `true` if the latest output is older than `max_age_ns`
    /// nanoseconds. Empty readers are considered stale.
    fn is_stale(&self, max_age_ns: u128) -> bool {
        let Some(output) = self.latest() else {
            return true;
        };

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();

        now.saturating_sub(output.computed_at_ns) > max_age_ns
    }
}

/// Adapter-side container for the latest finalized ordered output.
#[derive(Debug, Clone, Default)]
pub struct SharedOrderedOutput {
    latest: Option<OrderedFinalizedOutput>,
}

impl SharedOrderedOutput {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn from_output(output: OrderedFinalizedOutput) -> Self {
        Self {
            latest: Some(output),
        }
    }

    /// Publish a new output if it preserves the existing finalized prefix.
    pub fn update(
        &mut self,
        output: OrderedFinalizedOutput,
    ) -> Result<(), SharedOrderedOutputError> {
        if let Some(previous) = self.latest.as_ref()
            && !output.preserves_prefix(previous)
        {
            return Err(SharedOrderedOutputError::PrefixViolation);
        }

        self.latest = Some(output);
        Ok(())
    }

    /// Clear the latest output. Useful for tests and lifecycle resets.
    pub fn clear(&mut self) {
        self.latest = None;
    }
}

impl ReadOrderedOutput for SharedOrderedOutput {
    fn latest(&self) -> Option<&OrderedFinalizedOutput> {
        self.latest.as_ref()
    }
}
