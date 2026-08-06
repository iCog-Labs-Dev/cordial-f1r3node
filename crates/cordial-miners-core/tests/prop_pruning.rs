//! Property tests for checkpoint pruning (issue #164, area 1).
//!
//! Covers:
//! - P1.1: pruning does not rewrite the already-finalized tau prefix.
//!   (regression fixed in issue #170; test now enforced)
//! - P1.2: late blocks referencing pruned history are classified cleanly.
//! - P1.3: checkpoint order prefixes are not replayed or lost across
//!   repeated/successive prunes.
//! - P1.4: invalid checkpoint transitions (regression, disconnected) are
//!   rejected deterministically via `Blocklace::prune_below_checkpoint`.
//!
//! P1.1 uses the shared round-based generator in `tests/mod.rs`, since it
//! only needs *some* checkpoint to be established, not real removal.
//!
//! P1.2-P1.4 instead use a hand-built two-wave, single-chain DAG
//! (`build_two_wave_dag`). The round-based generator is fully-connected
//! (every validator references every block from the previous round), and
//! pruning only ever removes blocks that are both ancestors of the
//! checkpoint *and* not still needed by anything else retained in a
//! fully-connected DAG, every branch stays mutually dependent on the same
//! shared history, so nothing is ever actually removable. A genuine
//! single-chain structure is required to exercise real removal.
//!
//! P1.1 originally found a real regression, tracked as issue #170: `tau()`
//! re-derived finality from scratch on every call rather than trusting
//! `blocklace.checkpoint()`, and that re-derivation could lose approving
//! evidence collapsed by pruning, causing it to regress to an earlier
//! leader than the checkpoint it just established. Fixed in #170 by
//! short-circuiting `latest_final_leader` / `latest_weighted_final_leader`
//! to the recorded checkpoint; this test now runs to guard the fix.

#[path = "mod.rs"]
#[allow(dead_code)]
mod common;

use common::{build_dag, dag_spec_strategy};

use cordial_miners_core::consensus::validation::{
    InvalidBlock, ValidationConfig, ValidationResult, validated_insert,
};
use cordial_miners_core::consensus::{CheckpointGc, PruneError, checkpoint_after_finality, tau};
use cordial_miners_core::{Block, BlockContent, BlockIdentity, Blocklace, NodeId};

use std::collections::{HashMap, HashSet};

use proptest::prelude::*;

const WAVELENGTH: u64 = 3;

fn leader_v1(_wave: u64) -> Option<NodeId> {
    Some(common::node(1))
}

fn make_block(creator_id: u8, tag: u16, predecessors: HashSet<BlockIdentity>) -> Block {
    let mut content_hash = [0u8; 32];
    content_hash[0] = creator_id;
    content_hash[1..3].copy_from_slice(&tag.to_le_bytes());

    Block {
        identity: BlockIdentity {
            content_hash,
            creator: common::node(creator_id),
            signature: vec![],
        },
        content: BlockContent {
            payload: tag.to_le_bytes().to_vec(),
            predecessors,
        },
    }
}

/// Builds a two-wave, single-chain DAG (n=4, f=1): genesis leader -> round1
/// (3 blocks) -> round2 (3 blocks) -> wave1 leader -> round1 (3 blocks) ->
/// round2 (3 blocks). Returns the blocklace along with the last wave's
/// round-2 tips, so callers can extend it with further waves.
fn build_two_wave_dag() -> (Blocklace, HashSet<BlockIdentity>) {
    let mut blocklace = Blocklace::new();
    let (v1, v2, v3, v4) = (1u8, 2u8, 3u8, 4u8);

    let w0_leader = make_block(v1, 1, HashSet::new());
    common::insert(&mut blocklace, &w0_leader);
    let w0_leader_id = w0_leader.identity.clone();
    let w0_r1: Vec<Block> = [(v2, 2u16), (v3, 3), (v4, 4)]
        .into_iter()
        .map(|(v, t)| make_block(v, t, HashSet::from([w0_leader_id.clone()])))
        .collect();
    for b in &w0_r1 {
        common::insert(&mut blocklace, b);
    }
    let w0_r1_ids: HashSet<BlockIdentity> = w0_r1.iter().map(|b| b.identity.clone()).collect();
    let w0_r2: Vec<Block> = [(v2, 5u16), (v3, 6), (v4, 7)]
        .into_iter()
        .map(|(v, t)| make_block(v, t, w0_r1_ids.clone()))
        .collect();
    for b in &w0_r2 {
        common::insert(&mut blocklace, b);
    }
    let w0_r2_ids: HashSet<BlockIdentity> = w0_r2.iter().map(|b| b.identity.clone()).collect();
    let w1_leader = make_block(v1, 8, w0_r2_ids);
    common::insert(&mut blocklace, &w1_leader);
    let w1_leader_id = w1_leader.identity.clone();
    let w1_r1: Vec<Block> = [(v2, 9u16), (v3, 10), (v4, 11)]
        .into_iter()
        .map(|(v, t)| make_block(v, t, HashSet::from([w1_leader_id.clone()])))
        .collect();
    for b in &w1_r1 {
        common::insert(&mut blocklace, b);
    }
    let w1_r1_ids: HashSet<BlockIdentity> = w1_r1.iter().map(|b| b.identity.clone()).collect();
    let w1_r2: Vec<Block> = [(v2, 12u16), (v3, 13), (v4, 14)]
        .into_iter()
        .map(|(v, t)| make_block(v, t, w1_r1_ids.clone()))
        .collect();
    for b in &w1_r2 {
        common::insert(&mut blocklace, b);
    }
    let w1_r2_ids: HashSet<BlockIdentity> = w1_r2.iter().map(|b| b.identity.clone()).collect();

    (blocklace, w1_r2_ids)
}

/// Extends `blocklace` with one more wave on top of `tips`, returning the
/// new round-2 tips so waves can be chained.
fn extend_with_wave(
    blocklace: &mut Blocklace,
    tips: HashSet<BlockIdentity>,
    leader_tag: u16,
) -> HashSet<BlockIdentity> {
    let (v2, v3, v4) = (2u8, 3u8, 4u8);
    let leader = make_block(1, leader_tag, tips);
    common::insert(blocklace, &leader);
    let leader_id = leader.identity.clone();
    let r1: Vec<Block> = [
        (v2, leader_tag + 1),
        (v3, leader_tag + 2),
        (v4, leader_tag + 3),
    ]
    .into_iter()
    .map(|(v, t)| make_block(v, t, HashSet::from([leader_id.clone()])))
    .collect();
    for b in &r1 {
        common::insert(blocklace, b);
    }
    let r1_ids: HashSet<BlockIdentity> = r1.iter().map(|b| b.identity.clone()).collect();
    let r2: Vec<Block> = [
        (v2, leader_tag + 4),
        (v3, leader_tag + 5),
        (v4, leader_tag + 6),
    ]
    .into_iter()
    .map(|(v, t)| make_block(v, t, r1_ids.clone()))
    .collect();
    for b in &r2 {
        common::insert(blocklace, b);
    }
    r2.iter().map(|b| b.identity.clone()).collect()
}

#[test]
fn late_block_referencing_pruned_history_is_rejected_cleanly() {
    let mut blocklace = Blocklace::new();
    let n = 4usize;
    let f = common::fault_tolerance(n);
    let (v1, v2, v3, v4) = (1u8, 2u8, 3u8, 4u8);

    let w0_leader = make_block(v1, 1, HashSet::new());
    common::insert(&mut blocklace, &w0_leader);

    let w0_leader_id = w0_leader.identity.clone();
    let w0_r1: Vec<Block> = [(v2, 2u16), (v3, 3), (v4, 4)]
        .into_iter()
        .map(|(v, t)| make_block(v, t, HashSet::from([w0_leader_id.clone()])))
        .collect();
    for b in &w0_r1 {
        common::insert(&mut blocklace, b);
    }

    let w0_r1_ids: HashSet<BlockIdentity> = w0_r1.iter().map(|b| b.identity.clone()).collect();
    let w0_r2: Vec<Block> = [(v2, 5u16), (v3, 6), (v4, 7)]
        .into_iter()
        .map(|(v, t)| make_block(v, t, w0_r1_ids.clone()))
        .collect();
    for b in &w0_r2 {
        common::insert(&mut blocklace, b);
    }

    let w0_r2_ids: HashSet<BlockIdentity> = w0_r2.iter().map(|b| b.identity.clone()).collect();
    let w1_leader = make_block(v1, 8, w0_r2_ids);
    common::insert(&mut blocklace, &w1_leader);

    let w1_leader_id = w1_leader.identity.clone();
    let w1_r1: Vec<Block> = [(v2, 9u16), (v3, 10), (v4, 11)]
        .into_iter()
        .map(|(v, t)| make_block(v, t, HashSet::from([w1_leader_id.clone()])))
        .collect();
    for b in &w1_r1 {
        common::insert(&mut blocklace, b);
    }

    let w1_r1_ids: HashSet<BlockIdentity> = w1_r1.iter().map(|b| b.identity.clone()).collect();
    let w1_r2: Vec<Block> = [(v2, 12u16), (v3, 13), (v4, 14)]
        .into_iter()
        .map(|(v, t)| make_block(v, t, w1_r1_ids.clone()))
        .collect();
    for b in &w1_r2 {
        common::insert(&mut blocklace, b);
    }

    let report = checkpoint_after_finality(&mut blocklace, WAVELENGTH, n, f, leader_v1)
        .expect("checkpoint_after_finality should succeed on this two-wave DAG")
        .expect("two finalised waves should produce a checkpoint");
    let prune_id = report
        .removed
        .into_iter()
        .next()
        .expect("expected this fixed scenario to actually prune at least one block");

    let late_block = make_block(v2, 99, HashSet::from([prune_id.clone()]));
    let late_block_id = late_block.identity.clone();

    let bonds: HashMap<NodeId, u64> = [v1, v2, v3, v4]
        .into_iter()
        .map(|v| (common::node(v), 1u64))
        .collect();
    let config = ValidationConfig {
        check_content_hash: false,
        check_signature: false,
        check_sender: true,
        check_closure: true,
        check_chain_axiom: false,
        check_cordial: false,
    };

    let result = validated_insert(late_block, &mut blocklace, &bonds, &config);

    match result {
        ValidationResult::Invalid(errors) => {
            assert!(
                errors.iter().any(|e| matches!(
                    e,
                    InvalidBlock::MissingPredecessors { missing } if missing.contains(&prune_id)
                )),
                "expected MissingPredecessors referencing the pruned block, got: {errors:?}"
            );
        }
        ValidationResult::Valid => panic!(
            "expected rejection: block references pruned history, but validated_insert accepted it"
        ),
    }

    assert!(
        blocklace.content(&late_block_id).is_none(),
        "block should not have been inserted into the blocklace"
    )
}

#[test]
fn checkpoint_order_prefixes_are_not_replayed_or_lost() {
    let n = 4usize;
    let f = common::fault_tolerance(n);
    let (mut blocklace, tips) = build_two_wave_dag();

    let report1 = checkpoint_after_finality(&mut blocklace, WAVELENGTH, n, f, leader_v1)
        .expect("first prune should succeed")
        .expect("two waves should produce a checkpoint");
    let tau_after_first = tau(&blocklace, WAVELENGTH, n, f, leader_v1)
        .expect("tau should succeed after the first prune");

    // Nothing new has happened. A second immediate call must be a no-op,
    // not a replay of the same checkpoint.
    let repeat = checkpoint_after_finality(&mut blocklace, WAVELENGTH, n, f, leader_v1)
        .expect("repeated prune call should not error");
    assert!(
        repeat.is_none(),
        "expected no new checkpoint when nothing new is final"
    );
    assert_eq!(blocklace.checkpoint(), Some(&report1.checkpoint));

    // Extend with a third wave, then prune again.
    let _ = extend_with_wave(&mut blocklace, tips, 15);

    let report2 = checkpoint_after_finality(&mut blocklace, WAVELENGTH, n, f, leader_v1)
        .expect("second real prune should succeed")
        .expect("the third wave should produce a new checkpoint");
    assert_ne!(
        report2.checkpoint, report1.checkpoint,
        "checkpoint should have advanced"
    );

    let tau_after_second = tau(&blocklace, WAVELENGTH, n, f, leader_v1)
        .expect("tau should succeed after the second prune");

    // The core property: the prefix established by the first prune must
    // survive completely unchanged and unreordered inside the later,
    // longer prefix. Successive prunes concatenate, they don't replay
    // or lose earlier history.
    assert_eq!(tau_after_second.len(), report2.tau_prefix_len);
    assert_eq!(
        &tau_after_second[..tau_after_first.len()],
        tau_after_first.as_slice()
    );
}

#[test]
fn checkpoint_regression_is_rejected_and_state_unchanged() {
    let n = 4usize;
    let f = common::fault_tolerance(n);
    let (mut blocklace, _tips) = build_two_wave_dag();

    let report = checkpoint_after_finality(&mut blocklace, WAVELENGTH, n, f, leader_v1)
        .expect("prune should succeed")
        .expect("two waves should produce a checkpoint");

    // A freshly-inserted, unrelated block has low depth (0), which is
    // guaranteed to be less than the checkpoint's depth and since it's
    // inserted *after* pruning, it can't have been pruned away itself.
    let shallow = make_block(9, 500, HashSet::new());
    common::insert(&mut blocklace, &shallow);
    let shallow_id = shallow.identity.clone();

    let checkpoint_before = blocklace.checkpoint().cloned();
    let dom_before: HashSet<BlockIdentity> =
        blocklace.dom().iter().map(|id| (*id).clone()).collect();

    let result = blocklace.prune_below_checkpoint(&shallow_id);
    match result {
        Err(PruneError::CheckpointRegression { current, requested }) => {
            assert_eq!(*current, report.checkpoint);
            assert_eq!(*requested, shallow_id);
        }
        other => panic!("expected CheckpointRegression, got: {other:?}"),
    }

    assert_eq!(blocklace.checkpoint().cloned(), checkpoint_before);
    let dom_after: HashSet<BlockIdentity> =
        blocklace.dom().iter().map(|id| (*id).clone()).collect();
    assert_eq!(
        dom_after, dom_before,
        "blocklace contents must be unchanged after a rejected prune"
    );
}

#[test]
fn disconnected_checkpoint_is_rejected_and_state_unchanged() {
    let n = 4usize;
    let f = common::fault_tolerance(n);
    let (mut blocklace, _tips) = build_two_wave_dag();

    let report = checkpoint_after_finality(&mut blocklace, WAVELENGTH, n, f, leader_v1)
        .expect("prune should succeed")
        .expect("two waves should produce a checkpoint");

    // Build a genuinely separate chain, rooted at its own fresh genesis,
    // never referencing anything in the checkpointed history deep
    // enough to rule out CheckpointRegression, but causally unrelated.
    let mut rogue = make_block(9, 900, HashSet::new());
    common::insert(&mut blocklace, &rogue);
    let mut rogue_id = rogue.identity.clone();
    for i in 1..12u16 {
        rogue = make_block(9, 900 + i, HashSet::from([rogue_id.clone()]));
        common::insert(&mut blocklace, &rogue);
        rogue_id = rogue.identity.clone();
    }

    let checkpoint_before = blocklace.checkpoint().cloned();
    let dom_before: HashSet<BlockIdentity> =
        blocklace.dom().iter().map(|id| (*id).clone()).collect();

    let result = blocklace.prune_below_checkpoint(&rogue_id);
    match result {
        Err(PruneError::DisconnectedCheckpoint { current, requested }) => {
            assert_eq!(*current, report.checkpoint);
            assert_eq!(*requested, rogue_id);
        }
        other => panic!("expected DisconnectedCheckpoint, got: {other:?}"),
    }

    assert_eq!(blocklace.checkpoint().cloned(), checkpoint_before);
    let dom_after: HashSet<BlockIdentity> =
        blocklace.dom().iter().map(|id| (*id).clone()).collect();
    assert_eq!(
        dom_after, dom_before,
        "blocklace contents must be unchanged after a rejected prune"
    );
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    #[test]
    fn pruning_preserves_finalized_tau_prefix(spec in dag_spec_strategy()) {
        let mut dag = build_dag(&spec);
        let n = spec.validators.len();
        let f = common::fault_tolerance(n);

        let before = tau(&dag.blocklace, WAVELENGTH, n, f, leader_v1);

        let report = checkpoint_after_finality(&mut dag.blocklace, WAVELENGTH, n, f, leader_v1);

        if let Ok(Some(r)) = &report {
            prop_assert!(before.is_ok());
            let before_ids = before.unwrap();
            prop_assert_eq!(r.tau_prefix_len, before_ids.len());

            let after = tau(&dag.blocklace, WAVELENGTH, n, f, leader_v1);
            let after_ids = after.expect("tau must still succeed after a successful prune");
            prop_assert_eq!(&after_ids[..r.tau_prefix_len], before_ids.as_slice());
        }
    }
}
