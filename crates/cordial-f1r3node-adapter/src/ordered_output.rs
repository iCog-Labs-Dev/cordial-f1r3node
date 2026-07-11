//! Stable adapter-side data model for finalized ordered output.
//!
//! This module defines the shape of Cordial's finalized ordered output as
//! computed from the live mirrored blocklace state. Downstream consumers
//! (binaries, tests, node-facing consumers in future reintegration seams)
//! read this type rather than recomputing or reinterpreting the ordering
//! ad hoc.
//!
//! ## What's in the model
//!
//! | Field                 | Purpose                                         |
//! |-----------------------|-------------------------------------------------|
//! | `blocks`              | Ordered [`BlockIdentity`] sequence (tau order)  |
//! | `anchor`              | Latest weighted final leader anchoring ordering |
//! | `wavelength`          | Consensus wave size used for finality           |
//! | `bond_count`          | Number of bonded validators at computation time |
//! | `total_mirrored_blocks` | Total blocks in the blocklace mirror (not just finalized prefix) |
//! | `computed_at_ns`      | Wall-clock timestamp for staleness inspection   |
//!
//! ## Relation to existing types
//!
//! - [`super::snapshot::CasperSnapshot`] carries `ordered_finalized_blocks:
//!   Vec<Vec<u8>>` (bare content hashes) as one field among many. That type
//!   is tied to f1r3node's snapshot shape and is not a stable export seam.
//! - [`OrderedFinalizedOutput`] is the *adapter-side* export type:
//!   self-describing, includes full block identities and consensus metadata,
//!   and is decoupled from f1r3node's snapshot layout.
//! - The core `weighted_tau` / `tau` functions in `cordial-miners-core`
//!   return raw `Vec<BlockIdentity>`. This type wraps that vector with the
//!   context needed to interpret it.

use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use cordial_miners_core::types::BlockIdentity;

/// A finalized ordered output fragment produced by Cordial weighted-tau
/// ordering over the live mirrored blocklace.
///
/// This is the stable export type that binaries, tests, and future
/// node-facing consumers should read instead of calling into ordering
/// internals directly.
///
/// ## Construction
///
/// Prefer the builder-style [`new`](Self::new) constructor or the
/// [`Default`] impl for test fixtures. Production code creates instances
/// via the adapter's snapshot or live-ingress helpers.
///
/// ## Ordering invariant
///
/// `blocks` appears in deterministic topological order (tau order):
/// predecessor-first tie-broken by [`BlockIdentity`]'s natural ordering.
/// Every block in this list is finalized according to the current bonded
/// validator set and consensus parameters.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OrderedFinalizedOutput {
    /// The ordered sequence of block identities in the finalized prefix.
    ///
    /// Each entry carries the full [`BlockIdentity`] (content hash, creator,
    /// signature) so consumers have everything needed without extra lookups
    /// into the blocklace.
    pub blocks: Vec<BlockIdentity>,

    /// The latest weighted final leader that anchors this ordering fragment.
    ///
    /// This is the leader block whose approval frontier produced the ordered
    /// output. `None` when no block has been finalized yet — the output is
    /// empty and the anchor is undefined.
    ///
    /// Consumers can use the anchor to reason about the extent of finality
    /// coverage or to compare against a known-good block hash.
    pub anchor: Option<BlockIdentity>,

    /// Consensus wavelength (wave size in rounds) used when computing
    /// this output.
    ///
    /// Currently hard-coded to `3` (matching the `ES_WAVELENGTH` in
    /// [`super::snapshot`]). Exposed here so that if the parameter becomes
    /// dynamic in the future, consumers can determine which wavelength
    /// produced the ordering.
    pub wavelength: u64,

    /// Number of bonded validators at the time the output was computed.
    ///
    /// Useful for inspection — a low bond count means finality reflects
    /// less stake weight. A value of `0` indicates no bonds were provided.
    pub bond_count: usize,

    /// Total blocks in the blocklace mirror when this output was computed.
    ///
    /// This is not the same as `blocks.len()` (which counts only the
    /// finalized prefix). It reflects the whole observed DAG and helps
    /// consumers gauge how much of the mirror has been finalized.
    pub total_mirrored_blocks: usize,

    /// Wall-clock timestamp (nanoseconds since the Unix epoch) when this
    /// output was produced.
    ///
    /// Consumers can compare timestamps across outputs to detect staleness
    /// or measure ordering latency. Two outputs with the same anchor but
    /// different timestamps mean the mirror was re-evaluated without new
    /// finalization progress.
    pub computed_at_ns: u128,
}

impl OrderedFinalizedOutput {
    /// Construct a new ordered finalized output from its parts.
    ///
    /// `computed_at_ns` defaults to the current system time (use
    /// [`with_timestamp`](Self::with_timestamp) to override).
    pub fn new(
        blocks: Vec<BlockIdentity>,
        anchor: Option<BlockIdentity>,
        wavelength: u64,
        bond_count: usize,
        total_mirrored_blocks: usize,
    ) -> Self {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();

        Self {
            blocks,
            anchor,
            wavelength,
            bond_count,
            total_mirrored_blocks,
            computed_at_ns: now,
        }
    }

    /// Return the ordered block content hashes as `Vec<Vec<u8>>`.
    ///
    /// Convenience for callers that only need hash references (e.g., to
    /// compare against a prior output or to format for logging).
    pub fn block_hashes(&self) -> Vec<Vec<u8>> {
        self.blocks
            .iter()
            .map(|id| id.content_hash.to_vec())
            .collect()
    }

    /// Number of blocks in the ordered fragment.
    pub fn len(&self) -> usize {
        self.blocks.len()
    }

    /// Whether the ordered fragment is empty (no finalized blocks).
    pub fn is_empty(&self) -> bool {
        self.blocks.is_empty()
    }

    /// Return `true` whether this output preserves a previously exported finalized prefix.
    ///
    /// A valid newer output may be identical to `previous` or may append new finalized blocks.
    /// Previously exported blocks must not be removed,replaced, or reordered.
    pub fn preserves_prefix(&self, previous: &Self) -> bool {
        self.blocks.starts_with(&previous.blocks)
    }

    /// Return the anchor block's content hash, or `None` if no anchor exists.
    pub fn anchor_hash(&self) -> Option<Vec<u8>> {
        self.anchor.as_ref().map(|id| id.content_hash.to_vec())
    }

    /// Interpret the stored timestamp as a [`SystemTime`].
    ///
    /// Returns [`UNIX_EPOCH`] when `computed_at_ns` exceeds `u64::MAX`
    /// nanoseconds (approximately 584 years), which won't happen in practice.
    pub fn computed_at(&self) -> SystemTime {
        let secs = (self.computed_at_ns / 1_000_000_000) as u64;
        let subsec_nanos = (self.computed_at_ns % 1_000_000_000) as u32;
        UNIX_EPOCH + std::time::Duration::new(secs, subsec_nanos)
    }

    /// Override the wall-clock timestamp.
    ///
    /// Useful for test fixtures or replay scenarios where the original
    /// timestamp should be preserved rather than set to "now".
    pub fn with_timestamp(mut self, ns: u128) -> Self {
        self.computed_at_ns = ns;
        self
    }
}

impl Default for OrderedFinalizedOutput {
    fn default() -> Self {
        Self {
            blocks: Vec::new(),
            anchor: None,
            wavelength: 3,
            bond_count: 0,
            total_mirrored_blocks: 0,
            computed_at_ns: 0,
        }
    }
}

#[cfg(test)]
mod test {
    use super::OrderedFinalizedOutput;
    use cordial_miners_core::types::{BlockIdentity, NodeId};

    fn block(tag: u8) -> BlockIdentity {
        BlockIdentity {
            content_hash: [tag; 32],
            creator: NodeId(vec![tag]),
            signature: vec![tag; 64],
        }
    }

    fn output(blocks: Vec<BlockIdentity>) -> OrderedFinalizedOutput {
        OrderedFinalizedOutput::new(blocks, None, 3, 1, 0).with_timestamp(0)
    }

    #[test]
    fn identical_output_preserves_previous_prefix() {
        let previous = output(vec![block(1), block(2)]);
        let current = output(vec![block(1), block(2)]);

        assert!(current.preserves_prefix(&previous));
    }

    #[test]
    fn appended_output_preserves_previous_prefix() {
        let previous = output(vec![block(1), block(2)]);
        let current = output(vec![block(1), block(2), block(3)]);

        assert!(current.preserves_prefix(&previous));
    }

    #[test]
    fn reordered_output_does_not_preserve_previous_prefix() {
        let previous = output(vec![block(1), block(2)]);
        let current = output(vec![block(2), block(1), block(3)]);

        assert!(!current.preserves_prefix(&previous));
    }

    #[test]
    fn truncated_output_does_not_preserve_previous_prefix() {
        let previous = output(vec![block(1), block(2), block(3)]);
        let current = output(vec![block(1), block(2)]);

        assert!(!current.preserves_prefix(&previous));
    }

    #[test]
    fn replaced_block_does_not_preserve_previous_prefix() {
        let previous = output(vec![block(1), block(2)]);
        let current = output(vec![block(1), block(9), block(3)]);

        assert!(!current.preserves_prefix(&previous));
    }
}
