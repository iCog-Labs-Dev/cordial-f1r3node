//! End-to-end test for issue #243: proves that reputation weights
//! computed by `cordial-por`'s pipeline are the same weights Cordial
//! Miners' weighted consensus actually consumes, not a parallel or
//! untested path.
//!
//! This deliberately stays on the `cordial-por -> cordial-miners-core`
//! boundary only: no f1r3node live networking, no adapter event
//! extraction, and no RSpace execution are involved.
//!
//! Two tests, each independently reviewable:
//! - `por_weights_drive_cordial_miners_weighted_finality` drives four
//!   validators through every public pipeline stage (batch -> matrix ->
//!   normalize -> Liquid-Rank -> blend -> clamp), exports weights via
//!   `reputation_weights()`, confirms an ejected validator is dropped
//!   from the export, and feeds the result into `weighted_super_ratifies`.
//! - `weighted_finality_diverges_from_unweighted_supermajority` proves
//!   the weighted result actually depends on PoR-derived reputation, by
//!   constructing a case where a count-based supermajority of witnesses
//!   fails the weight-based check.
//!
//! One property of the pipeline is easy to miss when writing rating
//! fixtures: `normalize_recipient_group` normalizes each rating
//! relative to that specific recipient's own raters' min/max. Unanimous
//! agreement (including the trivial case of a single rater) always
//! collapses to `scale`, regardless of the absolute score given. So
//! separation between recipients' final reputation comes from *rater
//! count and disagreement*, not from picking different absolute score
//! values per recipient — both tests below are built around that fact.

use std::collections::{HashMap, HashSet};

use cordial_por::{
    MissingEntryPolicy, PorConfig, RatingRecord, ReputationEntry, ReputationState,
    ReputationVector, blend_reputation_transition, build_rating_batch, build_rating_matrix,
    clamp_reputation_transition, compute_liquid_rank_contribution, normalize_rating_matrix,
    replay_reputation_transition, reputation_weights,
};

use cordial_miners_core::consensus::{is_supermajority, super_ratifies, weighted_super_ratifies};
use cordial_miners_core::crypto::CryptoVerifier;
use cordial_miners_core::{Block, BlockContent, BlockIdentity, Blocklace, NodeId};

struct MockVerifier;
impl CryptoVerifier for MockVerifier {
    type Error = String;
    fn verify_block(
        &self,
        _c: &BlockContent,
        _s: &[u8],
        _creator: &NodeId,
    ) -> Result<(), Self::Error> {
        Ok(())
    }
}

fn node(id: u8) -> NodeId {
    NodeId(vec![id])
}

fn block(creator_id: u8, tag: u8, predecessors: HashSet<BlockIdentity>) -> Block {
    let mut content_hash = [0u8; 32];
    content_hash[0] = creator_id;
    content_hash[1] = tag;
    Block {
        identity: BlockIdentity {
            content_hash,
            creator: node(creator_id),
            signature: vec![],
        },
        content: BlockContent {
            payload: vec![],
            predecessors,
        },
    }
}

fn insert(bl: &mut Blocklace, b: &Block) {
    bl.insert(b.clone(), &MockVerifier).expect("insert failed");
}
