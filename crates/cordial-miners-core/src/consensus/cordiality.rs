//! Equivocation and cordiality predicates for Cordial Miners.
//!
//! This module gathers the protocol-facing DAG predicates that sit between the
//! structural helpers (`round`, `wave`) and the enforcement layer
//! (`validation`).
//!
//! The paper distinguishes:
//! - equivocation: a validator produces multiple conflicting blocks
//! - cordiality: a block does not hide relevant information from the DAG view
//!
//! In this implementation, the "known" portion of "known equivocations" is
//! interpreted conservatively as "already present in the local blocklace".
//! That makes these predicates usable inside block validation, where the
//! creator's private local view is not available.

use std::collections::{BTreeSet, HashMap, HashSet};

use crate::block::Block;
use crate::blocklace::Blocklace;
#[cfg(feature = "trace")]
use crate::consensus::approval::approves_with_memo;
use crate::consensus::approval::{ApprovalMemo, approves, weighted_approving_creators_with_memo};
use crate::consensus::round::{blocks_at_depth, depth};
#[cfg(feature = "trace")]
use crate::trace::{self, DetectEquivocationEvent, ThresholdCertificateEvent, TraceEvent};
use crate::types::{BlockIdentity, NodeId};

/// A same-round equivocation detected in the blocklace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Equivocation {
    pub creator: NodeId,
    pub round: u64,
    pub blocks: Vec<BlockIdentity>,
}

/// A globally known equivocation that a candidate block fails to acknowledge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HiddenEquivocation {
    pub creator: NodeId,
    pub round: u64,
    pub hidden: Vec<BlockIdentity>,
}

#[derive(Default)]
struct WeightedRatificationMemo {
    approval_memo: ApprovalMemo,
    weighted_ratifies_cache: HashMap<(BlockIdentity, BlockIdentity), bool>,
}

/// Return all blocks  created by 'creator' at exactly 'round' in the blocklace.
pub fn creator_blocks_at_round(
    blocklace: &Blocklace,
    creator: &NodeId,
    round: u64,
) -> HashSet<Block> {
    blocks_at_depth(blocklace, round)
        .into_iter()
        .filter(|b| b.identity.creator == *creator)
        .collect()
}

/// Return all same-round equivocation branches for `creator` at `round`.
/// Under the user story for this task, a validator equivocates when they create at least two different blocks in the exact same round.
pub fn equivocation_blocks_at_round(
    blocklace: &Blocklace,
    creator: &NodeId,
    round: u64,
) -> HashSet<Block> {
    let blocks = creator_blocks_at_round(blocklace, creator, round);
    if blocks.len() >= 2 {
        blocks
    } else {
        HashSet::new()
    }
}

/// Return every same-round equivocation currently present in the blocklace.
///
/// This pure query has no observer identity, so it does not emit a trace event.
/// Call [`all_equivocations_for_observer`] from a node-owned execution path when
/// a real local observer is available.
pub fn all_equivocations(blocklace: &Blocklace) -> Vec<Equivocation> {
    all_equivocations_with_observer(blocklace, None)
}

/// Return equivocations and emit detection events attributed to `observer`.
///
/// `node_id` in a detection event is the node observing the fork; the
/// `equivocator` field remains the validator that produced the conflicting
/// blocks. Keeping the observer explicit prevents the two identities from being
/// silently conflated at generic blocklace-query call sites.
pub fn all_equivocations_for_observer(
    blocklace: &Blocklace,
    observer: &NodeId,
) -> Vec<Equivocation> {
    all_equivocations_with_observer(blocklace, Some(observer))
}

fn all_equivocations_with_observer(
    blocklace: &Blocklace,
    observer: Option<&NodeId>,
) -> Vec<Equivocation> {
    #[cfg(not(feature = "trace"))]
    let _ = observer;
    let Some(max_round) = blocklace
        .dom()
        .into_iter()
        .filter_map(|id| depth(blocklace, id))
        .max()
    else {
        return Vec::new();
    };
    let creators: BTreeSet<NodeId> = blocklace
        .dom()
        .iter()
        .map(|id| id.creator.clone())
        .collect();

    let mut equivocations = Vec::new();

    for creator in creators {
        for round in 0..=max_round {
            #[cfg(feature = "trace")]
            let blocks: Vec<BlockIdentity> = {
                let mut blocks = equivocation_blocks_at_round(blocklace, &creator, round)
                    .into_iter()
                    .map(|b| b.identity)
                    .collect::<Vec<_>>();
                blocks.sort();
                blocks
            };
            #[cfg(not(feature = "trace"))]
            let blocks: Vec<BlockIdentity> =
                equivocation_blocks_at_round(blocklace, &creator, round)
                    .into_iter()
                    .map(|b| b.identity)
                    .collect();

            if blocks.len() >= 2 {
                #[cfg(feature = "trace")]
                if let Some(observer) = observer {
                    trace::emit(TraceEvent::DetectEquivocation(DetectEquivocationEvent {
                        node_id: trace::hex(&observer.0),
                        equivocator: trace::hex(&creator.0),
                        round,
                        conflicting_block_hashes: blocks
                            .iter()
                            .map(|id| trace::hex(&id.content_hash))
                            .collect(),
                    }));
                }

                equivocations.push(Equivocation {
                    creator: creator.clone(),
                    round,
                    blocks,
                });
            }
        }
    }

    equivocations
}

/// Return the set of block ids acknowledged by a candidate block through its
/// predecessor closure.
///
/// This is the reconstructed DAG view induced by the candidate's declared
/// predecessors, without inserting the candidate into the blocklace.
pub fn observed_block_ids(blocklace: &Blocklace, block: &Block) -> HashSet<BlockIdentity> {
    let mut observed = HashSet::new();

    for pred_id in &block.content.predecessors {
        observed.extend(blocklace.observe(pred_id));
    }

    observed
}

/// Return whether `block` acknowledges every branch of the same-round
/// equivocation by `creator` at `round`.
pub fn acknowledges_equivocation(
    blocklace: &Blocklace,
    block: &Block,
    creator: &NodeId,
    round: u64,
) -> bool {
    let equivocation = equivocation_blocks_at_round(blocklace, creator, round);
    if equivocation.is_empty() {
        return true;
    }

    let observed = observed_block_ids(blocklace, block);
    equivocation
        .iter()
        .all(|equiv_block| observed.contains(&equiv_block.identity))
}

/// Return the globally known equivocations hidden by `block`.
///
/// This uses the local blocklace as the source of knowledge. If the blocklace
/// already contains a same-round equivocation, then a candidate block is
/// considered to hide it when its predecessor closure does not acknowledge all
/// branches.
pub fn hidden_equivocations(blocklace: &Blocklace, block: &Block) -> Vec<HiddenEquivocation> {
    let observed = observed_block_ids(blocklace, block);

    all_equivocations(blocklace)
        .into_iter()
        .filter_map(|equivocation| {
            let hidden: Vec<BlockIdentity> = equivocation
                .blocks
                .iter()
                .filter(|id| !observed.contains(*id))
                .cloned()
                .collect();

            if hidden.is_empty() {
                None
            } else {
                Some(HiddenEquivocation {
                    creator: equivocation.creator,
                    round: equivocation.round,
                    hidden,
                })
            }
        })
        .collect()
}

// Return the validator tips ommitted by 'block'.
pub fn missing_known_tips(
    block: &Block,
    known_tips: &HashMap<NodeId, BlockIdentity>,
) -> Vec<BlockIdentity> {
    let mut missing: Vec<BlockIdentity> = known_tips
        .values()
        .filter(|tip_id| {
            block.identity != **tip_id && !block.content.predecessors.contains(*tip_id)
        })
        .cloned()
        .collect();
    missing.sort();
    missing
}

/// Check whether a block is cordial with respect to:
/// - known validator tips, and
/// - globally known same-round equivocations already present in the blocklace.
pub fn is_cordial_block(
    blocklace: &Blocklace,
    block: &Block,
    known_tips: &HashMap<NodeId, BlockIdentity>,
) -> bool {
    missing_known_tips(block, known_tips).is_empty()
        && hidden_equivocations(blocklace, block).is_empty()
}

/// Check whether a block b ratifies block b' when:
/// closure of b includes a supermajority of blocks that approve b'.
///
/// Per Definition 22 from Cordial Miners paper: "A block b ratifies b' if the closure
/// of b includes a supermajority of blocks that approve b'"
pub fn ratifies(
    blocklace: &Blocklace,
    ratifier: &Block,
    target: &Block,
    n: usize,
    f: usize,
) -> bool {
    // Ratification is defined over the closure [b] of the ratifier block,
    // which is inclusive of the ratifier itself.
    let observed_ids = blocklace.observe(&ratifier.identity);

    // find all blocks in ratifier's closure that approve target
    let approving: HashSet<Block> = observed_ids
        .iter()
        .filter_map(|id| blocklace.get(id))
        .filter(|block| approves(blocklace, &block.identity, &target.identity))
        .collect();

    is_supermajority(&approving, n, f)
}

/// Check whether a set of blocks super-ratifies a block b' when:
///
/// A set B super-ratifies a block b' if it includes a supermajority of blocks that ratify b'.
pub fn super_ratifies(
    blocklace: &Blocklace,
    blocks: &HashSet<Block>,
    target: &Block,
    n: usize,
    f: usize,
) -> bool {
    #[cfg(feature = "trace")]
    let ordered_blocks = {
        let mut blocks = blocks.iter().collect::<Vec<_>>();
        blocks.sort_by_key(|block| block.identity.clone());
        blocks
    };
    #[cfg(not(feature = "trace"))]
    let ordered_blocks: Vec<_> = blocks.iter().collect();
    let ratifying_blocks: HashSet<Block> = ordered_blocks
        .into_iter()
        .filter(|b| ratifies(blocklace, b, target, n, f))
        .cloned()
        .collect();

    is_supermajority(&ratifying_blocks, n, f)
}

/// Check whether `ratifier` ratifies `target` using bonded stake.
///
/// This is the f1r3node-compatible weighted variant of [`ratifies`]. It keeps
/// the same approval relation and inclusive ratifier closure, but measures a
/// strict two-thirds threshold over `bonds` instead of creator cardinality.
pub fn weighted_ratifies(
    blocklace: &Blocklace,
    ratifier: &Block,
    target: &Block,
    bonds: &HashMap<NodeId, u64>,
) -> bool {
    let mut memo = WeightedRatificationMemo::default();
    weighted_ratifies_with_memo(blocklace, ratifier, target, bonds, &mut memo)
}

fn weighted_ratifies_with_memo(
    blocklace: &Blocklace,
    ratifier: &Block,
    target: &Block,
    bonds: &HashMap<NodeId, u64>,
    memo: &mut WeightedRatificationMemo,
) -> bool {
    let cache_key = (ratifier.identity.clone(), target.identity.clone());
    if let Some(result) = memo.weighted_ratifies_cache.get(&cache_key) {
        return *result;
    }

    let result = if blocklace.get(&ratifier.identity).is_none()
        || blocklace.get(&target.identity).is_none()
    {
        false
    } else {
        let observed_ids = blocklace.observe(&ratifier.identity);
        let observed_blocks: HashSet<Block> = observed_ids
            .iter()
            .filter_map(|id| blocklace.get(id))
            .collect();

        let approving_creators = weighted_approving_creators_with_memo(
            blocklace,
            &observed_blocks,
            &target.identity,
            bonds,
            &mut memo.approval_memo,
        );

        let passed = is_weighted_supermajority(&approving_creators, bonds);

        #[cfg(feature = "trace")]
        if passed {
            let mut ordered_observed: Vec<_> = observed_blocks.iter().collect();
            ordered_observed.sort_by_key(|block| block.identity.clone());
            let approver_blocks: Vec<_> = ordered_observed
                .into_iter()
                .filter(|block| {
                    // Every pair was evaluated immediately above. Reuse that
                    // memoized result so formatting certificate evidence does
                    // not emit duplicate, HashSet-ordered approval events.
                    approves_with_memo(
                        blocklace,
                        &block.identity,
                        &target.identity,
                        &mut memo.approval_memo,
                    ) && bonds.get(&block.identity.creator).copied().unwrap_or(0) > 0
                })
                .collect();
            let mut creators: Vec<_> = approving_creators.iter().collect();
            creators.sort();
            let support = checked_bond_weight(
                approving_creators
                    .iter()
                    .filter_map(|creator| bonds.get(creator))
                    .copied(),
            )
            .unwrap_or(0);
            let total = checked_bond_weight(bonds.values().copied()).unwrap_or(0);
            let leader_hash = trace::hex(&target.identity.content_hash);
            let ratifier_hash = trace::hex(&ratifier.identity.content_hash);
            trace::emit(TraceEvent::BuildThresholdCertificate(
                ThresholdCertificateEvent {
                    node_id: trace::hex(&ratifier.identity.creator.0),
                    wave: None,
                    kind: "ratification".into(),
                    leader_hash: leader_hash.clone(),
                    ratifier_hash: Some(ratifier_hash.clone()),
                    certificate_id: trace::certificate_id(
                        "ratification",
                        &leader_hash,
                        Some(&ratifier_hash),
                    ),
                    approver_hashes: approver_blocks
                        .iter()
                        .map(|block| trace::hex(&block.identity.content_hash))
                        .collect(),
                    approvers: creators
                        .iter()
                        .map(|creator| trace::hex(&creator.0))
                        .collect(),
                    approver_count: approving_creators.len(),
                    approver_weight: support,
                    total_weight: total,
                    weight_table_hash: trace::weight_table_hash(bonds),
                },
            ));
        }

        passed
    };

    memo.weighted_ratifies_cache.insert(cache_key, result);
    result
}

/// Check whether the supplied witness set super-ratifies `target` using bonded
/// stake.
///
/// The caller must pass the block set for the relevant round, wave, or
/// f1r3node adapter context. This function does not select leaders, waves, or
/// finality windows.
pub fn weighted_super_ratifies(
    blocklace: &Blocklace,
    blocks: &HashSet<Block>,
    target: &Block,
    bonds: &HashMap<NodeId, u64>,
) -> bool {
    let mut memo = WeightedRatificationMemo::default();
    weighted_super_ratifies_with_memo(blocklace, blocks, target, bonds, &mut memo)
}

fn weighted_super_ratifies_with_memo(
    blocklace: &Blocklace,
    blocks: &HashSet<Block>,
    target: &Block,
    bonds: &HashMap<NodeId, u64>,
    memo: &mut WeightedRatificationMemo,
) -> bool {
    // Ratification recursively emits approvals and certificates.  Evaluate
    // witnesses in identity order so equivalent executions have byte-stable
    // traces despite HashSet's randomized iteration order.
    #[cfg(feature = "trace")]
    let ordered_blocks = {
        let mut blocks = blocks.iter().collect::<Vec<_>>();
        blocks.sort_by_key(|block| block.identity.clone());
        blocks
    };
    #[cfg(not(feature = "trace"))]
    let ordered_blocks: Vec<_> = blocks.iter().collect();
    let ratifying_blocks: Vec<&Block> = ordered_blocks
        .into_iter()
        .filter(|block| weighted_ratifies_with_memo(blocklace, block, target, bonds, memo))
        .filter(|block| {
            let creator = &block.identity.creator;
            bonds.get(creator).copied().unwrap_or(0) > 0
        })
        .collect();
    let ratifying_creators: HashSet<NodeId> = ratifying_blocks
        .iter()
        .map(|block| block.identity.creator.clone())
        .collect();

    let passed = is_weighted_supermajority(&ratifying_creators, bonds);

    #[cfg(feature = "trace")]
    if passed {
        let mut evidence = ratifying_blocks;
        evidence.sort_by_key(|block| block.identity.clone());
        let mut creators: Vec<_> = ratifying_creators.iter().collect();
        creators.sort();
        let support = checked_bond_weight(
            ratifying_creators
                .iter()
                .filter_map(|creator| bonds.get(creator))
                .copied(),
        )
        .unwrap_or(0);
        let total = checked_bond_weight(bonds.values().copied()).unwrap_or(0);
        let leader_hash = trace::hex(&target.identity.content_hash);
        trace::emit(TraceEvent::BuildThresholdCertificate(
            ThresholdCertificateEvent {
                node_id: trace::hex(&target.identity.creator.0),
                wave: None,
                kind: "super_ratification".into(),
                leader_hash: leader_hash.clone(),
                ratifier_hash: None,
                certificate_id: trace::certificate_id("super_ratification", &leader_hash, None),
                approver_hashes: evidence
                    .iter()
                    .map(|block| trace::hex(&block.identity.content_hash))
                    .collect(),
                approvers: creators
                    .iter()
                    .map(|creator| trace::hex(&creator.0))
                    .collect(),
                approver_count: ratifying_creators.len(),
                approver_weight: support,
                total_weight: total,
                weight_table_hash: trace::weight_table_hash(bonds),
            },
        ));
    }

    passed
}

/// Check whether `creators` hold strictly more than two-thirds of total bonded
/// stake.
///
/// `bonds` is the full active validator set for the decision context. Unknown
/// creators do not contribute support. Overflow returns `false`.
pub fn is_weighted_supermajority(creators: &HashSet<NodeId>, bonds: &HashMap<NodeId, u64>) -> bool {
    let Some(total_weight) = checked_bond_weight(bonds.values().copied()) else {
        return false;
    };

    if total_weight == 0 {
        return false;
    }

    let Some(support_weight) = checked_bond_weight(
        creators
            .iter()
            .filter_map(|creator| bonds.get(creator))
            .copied(),
    ) else {
        return false;
    };

    strict_two_thirds(support_weight, total_weight)
}

fn checked_bond_weight(weights: impl IntoIterator<Item = u64>) -> Option<u128> {
    weights
        .into_iter()
        .try_fold(0u128, |total, weight| total.checked_add(u128::from(weight)))
}

#[cfg(not(feature = "trace-threshold-mutation"))]
fn strict_two_thirds(support_weight: u128, total_weight: u128) -> bool {
    let Some(weighted_support) = support_weight.checked_mul(3) else {
        return false;
    };
    let Some(threshold) = total_weight.checked_mul(2) else {
        return false;
    };

    weighted_support > threshold
}

/// Deliberately weakened quorum predicate for the Issue #188 mutation test.
///
/// This is compiled only by the dedicated mutation harness. It exercises the
/// real approval, certificate and finality call graph while demonstrating
/// that Lean rejects a Rust implementation changed from strict two-thirds to
/// a simple majority.
#[cfg(feature = "trace-threshold-mutation")]
fn strict_two_thirds(support_weight: u128, total_weight: u128) -> bool {
    let Some(weighted_support) = support_weight.checked_mul(2) else {
        return false;
    };

    weighted_support > total_weight
}

/// Check if a set of blocks constitutes a supermajority.
///
/// Supermajority: > (n+f)/2 distinct creators
pub fn is_supermajority(blocks: &HashSet<Block>, n: usize, f: usize) -> bool {
    let distinct_creators: HashSet<_> =
        blocks.iter().map(|block| &block.identity.creator).collect();

    distinct_creators.len() > (n + f) / 2
}
