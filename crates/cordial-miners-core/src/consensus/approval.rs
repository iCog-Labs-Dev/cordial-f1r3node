//! Approval predicates for Cordial Miners.
//!
//! This module implements the approval relation that determines whether a block
//! approves a target block according to the Cordial Miners protocol.
//!
//! See Definition 18 of "Cordial Miners: Voluntary Participation in Blockchains"
//! (arXiv:2205.09174) for the formal specification.

use std::collections::{BTreeSet, HashMap, HashSet};

use crate::block::Block;
use crate::blocklace::Blocklace;
#[cfg(feature = "trace")]
use crate::consensus::round::depth;
use crate::consensus::weight_snapshot::WeightSnapshot;
#[cfg(feature = "trace")]
use crate::trace::{self, AcceptApprovalEvent, TraceEvent};
use crate::types::{BlockIdentity, NodeId};

#[derive(Default)]
pub(crate) struct ApprovalMemo {
    pub(crate) observe_cache: HashMap<BlockIdentity, BTreeSet<BlockIdentity>>,
    approves_cache: HashMap<(BlockIdentity, BlockIdentity), bool>,
}

/// A block `approver` approves a `target` block if:
/// 1. The approver observes the target (i.e., target is in the approver's predecessor closure)
/// 2. The approver does not observe any equivocating sibling of the target
///
/// This implements Definition 18 from the Cordial Miners paper (arXiv:2205.09174).
/// A block approves a target it observes only when no equivocating sibling of that target
/// is also in the observed set.
pub fn approves(blocklace: &Blocklace, approver: &BlockIdentity, target: &BlockIdentity) -> bool {
    let mut memo = ApprovalMemo::default();
    approves_with_memo(blocklace, approver, target, &mut memo)
}

pub(crate) fn approves_with_memo(
    blocklace: &Blocklace,
    approver: &BlockIdentity,
    target: &BlockIdentity,
    memo: &mut ApprovalMemo,
) -> bool {
    let cache_key = (approver.clone(), target.clone());
    if let Some(result) = memo.approves_cache.get(&cache_key) {
        return *result;
    }

    let result = approves_uncached(blocklace, approver, target, memo);
    #[cfg(feature = "trace")]
    if result && let Some(approver_block) = blocklace.get(approver) {
        trace::emit(TraceEvent::AcceptApproval(AcceptApprovalEvent {
            node_id: trace::hex(&approver_block.identity.creator.0),
            wave: None,
            round: depth(blocklace, approver).unwrap_or(0),
            approver: trace::hex(&approver_block.identity.creator.0),
            approver_hash: trace::hex(&approver_block.identity.content_hash),
            target_hash: trace::hex(&target.content_hash),
        }));
    }
    memo.approves_cache.insert(cache_key, result);
    result
}

fn approves_uncached(
    blocklace: &Blocklace,
    approver: &BlockIdentity,
    target: &BlockIdentity,
    _memo: &mut ApprovalMemo,
) -> bool {
    let Some(resolved_approver) = blocklace.resolve_identity(approver) else {
        return false;
    };

    // Answered from the shared ancestor index (#263): the approver must observe
    // the target and must not observe an equivocating sibling of it. The index
    // keeps those siblings per block, so a creator that forked once only affects
    // its forked blocks. This is Definition 18 evaluated exactly; the per-block
    // closure sets it replaces cost O(N^2) time and memory per memo.
    let index = blocklace.ancestor_index();
    match (index.index_of(resolved_approver), index.index_of(target)) {
        (Some(observer), Some(observed)) => index.approves(observer, observed),
        _ => false,
    }
}

/// Return the set of all blocks in the blocklace that approve the target.
///
/// This function walks every block in the blocklace and collects those blocks for which
/// the `approves` predicate returns true with respect to the target.
///
/// See Definition 18 of "Cordial Miners: Voluntary Participation in Blockchains"
/// (arXiv:2205.09174) for the formal specification of approval.
pub fn approving_blocks(blocklace: &Blocklace, target: &BlockIdentity) -> HashSet<Block> {
    blocklace
        .dom()
        .into_iter()
        .filter_map(|block_id| blocklace.get(block_id))
        .filter(|block| approves(blocklace, &block.identity, target))
        .collect()
}

/// Return the bonded creators in `blocks` whose blocks approve `target`.
///
/// This helper does not redefine approval. It delegates to the paper-native
/// [`approves`] predicate and only changes support accounting from creator
/// cardinality to positive bonded stake.
pub fn weighted_approving_creators(
    blocklace: &Blocklace,
    blocks: &HashSet<Block>,
    target: &BlockIdentity,
    bonds: &HashMap<NodeId, u64>,
) -> HashSet<NodeId> {
    let weights = WeightSnapshot::from_bonds(bonds);
    let mut memo = ApprovalMemo::default();
    weighted_approving_creators_with_memo(blocklace, blocks, target, &weights, &mut memo)
}

pub(crate) fn weighted_approving_creators_with_memo(
    blocklace: &Blocklace,
    blocks: &HashSet<Block>,
    target: &BlockIdentity,
    weights: &WeightSnapshot,
    memo: &mut ApprovalMemo,
) -> HashSet<NodeId> {
    // Approval checks emit trace evidence. Never let HashSet's randomized
    // iteration order leak into the canonical event stream. The returned value
    // is a HashSet, so this ordering changes only trace record order, not the
    // approval result or its weighted support.
    #[cfg(feature = "trace")]
    let ordered_blocks = {
        let mut blocks = blocks.iter().collect::<Vec<_>>();
        blocks.sort_by_key(|block| block.identity.clone());
        blocks
    };
    #[cfg(not(feature = "trace"))]
    let ordered_blocks: Vec<_> = blocks.iter().collect();
    ordered_blocks
        .into_iter()
        .filter(|block| approves_with_memo(blocklace, &block.identity, target, memo))
        .filter_map(|block| {
            let creator = &block.identity.creator;
            (weights.weight_of(creator) > 0).then(|| creator.clone())
        })
        .collect()
}
