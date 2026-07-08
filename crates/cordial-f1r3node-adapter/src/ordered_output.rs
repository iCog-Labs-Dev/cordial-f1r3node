//! Stable export seam for ordered finalized output.
//!
//! This module is the adapter-side boundary through which the ordered
//! finalized fragment of the mirrored blocklace is exposed to callers —
//! today, the `live_mirror_check` debugging harness; later, real downstream
//! consumers.
//!
//! ## Why this exists
//!
//! Before this module, the harness computed the "latest finalized ordered
//! fragment" itself by calling `approved_blocks_for_leader` and `xsort`
//! directly against the mirrored blocklace. That mixed harness/debugging
//! logic with ordering computation and left no single place downstream
//! integrations could depend on.
//!
//! This module owns that computation instead. Callers get back a small,
//! stable `OrderedFragment` value — the linearized set of blocks approved
//! by the current finalized leader, each annotated with the summary fields
//! (creator, block number, round, wave) useful for debugging — without
//! needing to know anything about `Blocklace`, `xsort`, or how finality is
//! determined.
//!
//! ## Non-goals
//!
//! This module does **not** serve ordered output over HTTP or gRPC, and it
//! does not address node-side consumption. It is purely the in-process
//! export boundary; transport-level exposure is separate follow-up work.

use std::collections::HashMap;

use cordial_miners_core::blocklace::Blocklace;
use cordial_miners_core::consensus::{approved_blocks_for_leader, depth, wave_of_round, xsort};
use cordial_miners_core::execution::CordialBlockPayload;
use cordial_miners_core::types::{BlockIdentity, NodeId};

use crate::snapshot::latest_finalized_block_id;

/// Default wave length used when converting a block's round into a wave for
/// summary purposes. This mirrors the wave length used elsewhere in the
/// adapter (e.g. weighted final leader computation in `live_mirror_check`).
pub const DEFAULT_WAVE_LENGTH: u64 = 3;

/// Summary metadata for a single block within an [`OrderedFragment`].
///
/// These are the fields useful for debugging and light inspection; the
/// export seam intentionally keeps this flat and serialization-friendly
/// rather than exposing internal `Block`/`Blocklace` types.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrderedBlockSummary {
    /// Content hash identifying this block.
    pub content_hash: Vec<u8>,
    /// Raw creator/node id bytes for this block.
    pub creator: Vec<u8>,
    /// Cordial block number carried in the block's execution payload.
    pub block_number: u64,
    /// Blocklace depth/round of this block, if it could be computed.
    pub round: Option<u64>,
    /// Wave derived from `round`, if both could be computed.
    pub wave: Option<u64>,
}

/// The ordered finalized fragment: the blocks approved by a given finalized
/// leader, linearized via weighted tau ordering, each with summary
/// metadata attached.
///
/// This is the stable, exported representation of "ordered finalized
/// output" for a single finalized leader. It is intentionally decoupled
/// from `Blocklace` so it can be printed, serialized, or handed to a
/// downstream consumer without re-exposing internal adapter state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrderedFragment {
    /// Content hash of the finalized leader block this fragment is anchored
    /// to.
    pub leader_hash: Vec<u8>,
    /// The linearized, approved blocks, in canonical (weighted tau) order.
    pub blocks: Vec<OrderedBlockSummary>,
}

impl OrderedFragment {
    /// Whether this fragment contains no blocks.
    pub fn is_empty(&self) -> bool {
        self.blocks.is_empty()
    }

    /// Number of blocks in this fragment.
    pub fn len(&self) -> usize {
        self.blocks.len()
    }

    /// Reduce the fragment to bare content hashes, in order. Useful for
    /// callers that only need the linearized identity sequence (e.g. to
    /// persist or diff against a previous run), rather than the full
    /// per-block summary.
    pub fn hashes(&self) -> Vec<Vec<u8>> {
        self.blocks.iter().map(|b| b.content_hash.clone()).collect()
    }
}

/// Errors produced while computing ordered output through this export seam.
#[derive(Debug)]
pub enum OrderedOutputError {
    /// The weighted tau ordering function failed on the approved set.
    Ordering(String),
}

impl std::fmt::Display for OrderedOutputError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Ordering(err) => write!(f, "failed to order finalized fragment: {err}"),
        }
    }
}

impl std::error::Error for OrderedOutputError {}

/// Compute the ordered finalized fragment anchored at the current latest
/// finalized block in `blocklace`: the blocks approved by that leader,
/// linearized via weighted tau ordering, each annotated with summary
/// metadata.
///
/// Returns `Ok(None)` when the mirrored state does not yet have a finalized
/// leader (e.g. still bootstrapping) — this is a normal, expected condition
/// rather than an error.
///
/// This is the stable export seam for ordered finalized output. Callers
/// (inspection harnesses today, other downstream consumers later) should
/// go through this function rather than recomputing
/// `approved_blocks_for_leader` + `xsort` themselves, so that ordering
/// computation stays in one place.
pub fn latest_finalized_ordered_fragment(
    blocklace: &Blocklace,
    bonds: &HashMap<NodeId, u64>,
    wave_length: u64,
) -> Result<Option<OrderedFragment>, OrderedOutputError> {
    let Some(leader) = latest_finalized_block_id(blocklace, bonds) else {
        return Ok(None);
    };

    let leader_hash = leader.content_hash.to_vec();
    let approved = approved_blocks_for_leader(blocklace, &leader);
    let ordered =
        xsort(&approved).map_err(|err| OrderedOutputError::Ordering(format!("{err:?}")))?;

    let blocks = ordered
        .into_iter()
        .map(|id| summarize_block(blocklace, &id, wave_length))
        .collect();

    Ok(Some(OrderedFragment { leader_hash, blocks }))
}

/// Convenience wrapper over [`latest_finalized_ordered_fragment`] using
/// [`DEFAULT_WAVE_LENGTH`].
pub fn latest_finalized_ordered_fragment_default(
    blocklace: &Blocklace,
    bonds: &HashMap<NodeId, u64>,
) -> Result<Option<OrderedFragment>, OrderedOutputError> {
    latest_finalized_ordered_fragment(blocklace, bonds, DEFAULT_WAVE_LENGTH)
}

fn summarize_block(
    blocklace: &Blocklace,
    id: &BlockIdentity,
    wave_length: u64,
) -> OrderedBlockSummary {
    let round = depth(blocklace, id);
    let wave = round.and_then(|round| wave_of_round(round, wave_length));
    let block_number = blocklace
        .content(id)
        .and_then(|content| CordialBlockPayload::from_bytes(&content.payload).ok())
        .map(|payload| payload.state.block_number)
        .unwrap_or(0);

    OrderedBlockSummary {
        content_hash: id.content_hash.to_vec(),
        creator: id.creator.0.clone(),
        block_number,
        round,
        wave,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_blocklace_has_no_finalized_fragment() {
        let blocklace = Blocklace::new();
        let bonds = HashMap::new();
        let result = latest_finalized_ordered_fragment_default(&blocklace, &bonds)
            .expect("computing over an empty blocklace should not error");
        assert!(result.is_none());
    }

    #[test]
    fn fragment_helpers_report_empty_correctly() {
        let fragment = OrderedFragment {
            leader_hash: vec![1, 2, 3],
            blocks: Vec::new(),
        };
        assert!(fragment.is_empty());
        assert_eq!(fragment.len(), 0);
        assert!(fragment.hashes().is_empty());
    }
}