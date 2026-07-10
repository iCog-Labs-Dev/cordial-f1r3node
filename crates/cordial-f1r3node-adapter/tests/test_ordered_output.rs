//! Tests for the ordered finalized-output export seam.
//!
//! Finalized blocks are exported as one ordered list. Later output may append
//! new finalized blocks, but it must never remove, replace, or reorder blocks
//! that were already exported.
//!
//! These tests cover the adapter path from `LiveIngress` mirroring through
//! finality and ordering to `OrderedFinalizedOutput`.
//!
//! Both single-validator and weighted multi-validator ordering paths are
//! covered. Blocks use `ingest_trusted_block` so these tests focus on mirror
//! and ordering behavior but message translation is tested elsewhere.

use std::collections::{HashMap, HashSet};

use cordial_f1r3node_adapter::grpc_ingest::BlocklaceAdapter;
use cordial_f1r3node_adapter::live_ingress::LiveIngress;
use cordial_f1r3node_adapter::ordered_output::OrderedFinalizedOutput;
use cordial_f1r3node_adapter::shard_conf::CasperShardConf;
use cordial_miners_core::Block;
use cordial_miners_core::crypto::hash_content;
use cordial_miners_core::execution::{BlockState, CordialBlockPayload};
use cordial_miners_core::types::{BlockContent, BlockIdentity, NodeId};

/// Number of rounds in one consensus wave which same as `ES_WAVELENGTH` in `snapshot.rs`.
const WAVELENGTH: u64 = 3;

// ════════════════════════════════════════════════════════════════════════════════════════
// Test helpers: build validators, blocks, and live-mirror setup.
// ════════════════════════════════════════════════════════════════════════════════════════

#[derive(Default)]
struct TestAdapter;

impl BlocklaceAdapter<BlockIdentity> for TestAdapter {
    fn on_block(&mut self, _block: Block) -> anyhow::Result<()> {
        Ok(())
    }
}

fn validator(tag: u8) -> NodeId {
    NodeId(vec![tag])
}

fn shard_conf() -> CasperShardConf {
    CasperShardConf {
        shard_name: "root".to_string(),
        max_number_of_parents: 16,
        fault_tolerance_threshold: 0.333,
        deploy_lifespan: 50,
        min_phlo_price: 1,
        ..CasperShardConf::default()
    }
}

fn payload(block_number: u64, state_tag: u8) -> CordialBlockPayload {
    CordialBlockPayload {
        state: BlockState {
            pre_state_hash: vec![state_tag; 32],
            post_state_hash: vec![state_tag.wrapping_add(1); 32],
            bonds: vec![],
            block_number,
        },
        deploys: vec![],
        rejected_deploys: vec![],
        system_deploys: vec![],
    }
}

/// Build a block with zero or more known parent identities.
fn make_block(
    creator: &NodeId,
    block_number: u64,
    state_tag: u8,
    predecessors: &[BlockIdentity],
) -> Block {
    let predecessors: HashSet<BlockIdentity> = predecessors.iter().cloned().collect();

    let content = BlockContent {
        payload: payload(block_number, state_tag).to_bytes(),
        predecessors,
    };

    Block {
        identity: BlockIdentity {
            content_hash: hash_content(&content),
            creator: creator.clone(),
            signature: vec![state_tag; 64],
        },
        content,
    }
}

/// Build a single-validator chain where every block follows the one before it.
/// block 0 -> block 1 -> block 2 -> block 3
fn sequential_blocks(creator: &NodeId, count: usize) -> Vec<Block> {
    let mut blocks = Vec::with_capacity(count);

    for index in 0..count {
        let block_number = u64::try_from(index).expect("test block index should fit into u64");
        let state_tag = u8::try_from(index + 1).expect("test block count should fit into u8");
        let predecessor = blocks.last().map(|block: &Block| block.identity.clone());
        let predecessors: Vec<BlockIdentity> = predecessor.into_iter().collect();

        blocks.push(make_block(creator, block_number, state_tag, &predecessors));
    }

    blocks
}

fn live_ingress_single(creator: &NodeId) -> LiveIngress<TestAdapter> {
    let mut bonds = HashMap::new();
    bonds.insert(creator.clone(), 100);

    LiveIngress::with_consensus_view(TestAdapter::default(), bonds, shard_conf(), "root")
}

fn live_ingress_multi(creators: &[NodeId]) -> LiveIngress<TestAdapter> {
    let mut bonds = HashMap::new();
    for creator in creators {
        bonds.insert(creator.clone(), 100);
    }

    LiveIngress::with_consensus_view(TestAdapter::default(), bonds, shard_conf(), "root")
}

fn live_ingress_weighted(validators: &[(NodeId, u64)]) -> LiveIngress<TestAdapter> {
    let bonds = validators.iter().cloned().collect();

    LiveIngress::with_consensus_view(TestAdapter::default(), bonds, shard_conf(), "root")
}

fn ingest_batch(ingress: &mut LiveIngress<TestAdapter>, blocks: &[Block]) {
    for block in blocks {
        ingress
            .ingest_trusted_block(block.clone())
            .expect("trusted test block should enter the live mirror");
    }
}

fn assert_monotonic(earlier: &OrderedFinalizedOutput, later: &OrderedFinalizedOutput, ctx: &str) {
    assert!(
        later.preserves_prefix(earlier),
        "{ctx}: later output must preserve the earlier finalized prefix.\n\
         earlier={:?}\nlater={:?}",
        earlier.block_hashes(),
        later.block_hashes(),
    );
}

// ════════════════════════════════════════════════════════════════════════════════════════
//  Single-validator ordered-output checks
// ════════════════════════════════════════════════════════════════════════════════════════

#[test]
fn ordered_output_grows_monotonically_across_live_mirror_batches() {
    let creator = validator(1);
    let blocks = sequential_blocks(&creator, 9);
    let mut ingress = live_ingress_single(&creator);

    ingest_batch(&mut ingress, &blocks[0..3]);
    let first = ingress
        .latest_finalized_ordered_output(WAVELENGTH)
        .expect("first ordered output should be computed");

    ingest_batch(&mut ingress, &blocks[3..6]);
    let second = ingress
        .latest_finalized_ordered_output(WAVELENGTH)
        .expect("second ordered output should be computed");

    ingest_batch(&mut ingress, &blocks[6..9]);
    let third = ingress
        .latest_finalized_ordered_output(WAVELENGTH)
        .expect("third ordered output should be computed");

    assert_monotonic(&first, &second, "batch 1 -> batch 2");
    assert_monotonic(&second, &third, "batch 2 -> batch 3");
    assert_monotonic(&first, &third, "batch 1 -> batch 3 (transitively)");

    assert!(first.len() <= second.len());
    assert!(second.len() <= third.len());
    assert!(
        first.len() < third.len(),
        "ordered output should grow after additional completed waves"
    );

    assert_eq!(first.total_mirrored_blocks, 3);
    assert_eq!(second.total_mirrored_blocks, 6);
    assert_eq!(third.total_mirrored_blocks, 9);

    assert!(!first.is_empty());
    assert!(!second.is_empty());
    assert!(!third.is_empty());
}

#[test]
fn previously_exported_blocks_keep_the_same_positions() {
    let creator = validator(2);
    let blocks = sequential_blocks(&creator, 6);
    let mut ingress = live_ingress_single(&creator);

    ingest_batch(&mut ingress, &blocks[0..3]);
    let previous = ingress
        .latest_finalized_ordered_output(WAVELENGTH)
        .expect("previous output should be computed");
    assert!(
        !previous.is_empty(),
        "first completed wave should export finalized blocks"
    );

    ingest_batch(&mut ingress, &blocks[3..6]);
    let current = ingress
        .latest_finalized_ordered_output(WAVELENGTH)
        .expect("current output should be computed");
    assert!(
        current.len() > previous.len(),
        "later mirrored state should extend finalized ordered output"
    );
    assert_monotonic(
        &previous,
        &current,
        "previous export -> later mirrored state",
    );

    for (index, previous_block) in previous.blocks.iter().enumerate() {
        assert_eq!(
            current.blocks.get(index),
            Some(previous_block),
            "previously exported block changed at ordered position {index}"
        );
    }
}

#[test]
fn recomputing_without_new_blocks_keeps_the_same_finalized_sequence() {
    let creator = validator(3);
    let blocks = sequential_blocks(&creator, 3);
    let mut ingress = live_ingress_single(&creator);

    ingest_batch(&mut ingress, &blocks);

    let first = ingress
        .latest_finalized_ordered_output(WAVELENGTH)
        .expect("first output should be computed");
    let second = ingress
        .latest_finalized_ordered_output(WAVELENGTH)
        .expect("recomputed output should be computed");

    assert_eq!(
        second.blocks, first.blocks,
        "recomputing unchanged mirrored state changed finalized ordering"
    );
    assert_eq!(
        second.anchor, first.anchor,
        "recomputing unchanged mirrored state changed the finality anchor"
    );
    assert_eq!(
        second.total_mirrored_blocks, first.total_mirrored_blocks,
        "recomputing unchanged state changed the mirrored block count"
    );

    assert!(second.preserves_prefix(&first));
    assert!(first.preserves_prefix(&second));
}

// ════════════════════════════════════════════════════════════════════════════════════════
// Multi-validator ordered-output checks
// ════════════════════════════════════════════════════════════════════════════════════════

#[test]
fn multi_validator_ordered_output_is_monotonic_as_a_wave_completes() {
    let creator_1 = validator(11);
    let creator_2 = validator(12);
    let creator_3 = validator(13);
    let creator_4 = validator(14);

    // The three validators supporting the leader hold 90% of the bonded
    // weight, so this scenario exercises stake-weighted finality rather
    // than relying on an equal-validator head count.
    let mut ingress = live_ingress_weighted(&[
        (creator_1.clone(), 100),
        (creator_2.clone(), 400),
        (creator_3.clone(), 300),
        (creator_4.clone(), 200),
    ]);

    let leader = make_block(&creator_1, 0, 1, &[]);

    let round1_v2 = make_block(&creator_2, 1, 2, &[leader.identity.clone()]);
    let round1_v3 = make_block(&creator_3, 1, 3, &[leader.identity.clone()]);
    let round1_v4 = make_block(&creator_4, 1, 4, &[leader.identity.clone()]);

    let round1_support = vec![
        round1_v2.identity.clone(),
        round1_v3.identity.clone(),
        round1_v4.identity.clone(),
    ];

    let round2_v2 = make_block(&creator_2, 2, 5, &round1_support);
    let round2_v3 = make_block(&creator_3, 2, 6, &round1_support);
    let round2_v4 = make_block(&creator_4, 2, 7, &round1_support);

    // The first batch has no round-2 support yet so if output is empty that is okay,
    // but completing the wave must not rewrite anything already exported.
    ingest_batch(
        &mut ingress,
        &[leader.clone(), round1_v2, round1_v3, round1_v4],
    );
    let partial = ingress
        .latest_finalized_ordered_output(WAVELENGTH)
        .expect("ordered output computation should succeed");

    // Batch 2: complete the wave with round 2.
    ingest_batch(&mut ingress, &[round2_v2, round2_v3, round2_v4]);
    let complete = ingress
        .latest_finalized_ordered_output(WAVELENGTH)
        .expect("ordered output computation should succeed");

    assert_monotonic(&partial, &complete, "partial wave -> complete wave");
    assert!(
        !complete.is_empty(),
        "expected the leader's wave to finalize once round 2 arrives"
    );
    assert!(
        complete
            .blocks
            .iter()
            .any(|id| id.content_hash == leader.identity.content_hash),
        "leader block should be part of the finalized output once its wave completes"
    );
}

#[test]
fn multi_validator_ordered_output_does_not_rewrite_finalized_prefix_on_unrelated_growth() {
    let creator_1 = validator(21);
    let creator_2 = validator(22);
    let creator_3 = validator(23);
    let creator_4 = validator(24);

    let mut ingress = live_ingress_multi(&[
        creator_1.clone(),
        creator_2.clone(),
        creator_3.clone(),
        creator_4.clone(),
    ]);

    let leader = make_block(&creator_1, 0, 1, &[]);
    let round1_v2 = make_block(&creator_2, 1, 2, &[leader.identity.clone()]);
    let round1_v3 = make_block(&creator_3, 1, 3, &[leader.identity.clone()]);
    let round1_v4 = make_block(&creator_4, 1, 4, &[leader.identity.clone()]);

    let round1_support = vec![
        round1_v2.identity.clone(),
        round1_v3.identity.clone(),
        round1_v4.identity.clone(),
    ];

    let round2_v2 = make_block(&creator_2, 2, 5, &round1_support);
    let round2_v3 = make_block(&creator_3, 2, 6, &round1_support);
    let round2_v4 = make_block(&creator_4, 2, 7, &round1_support);

    ingest_batch(
        &mut ingress,
        &[
            leader, round1_v2, round1_v3, round1_v4, round2_v2, round2_v3, round2_v4,
        ],
    );
    let after_wave_0 = ingress
        .latest_finalized_ordered_output(WAVELENGTH)
        .expect("ordered output computation should succeed");
    assert!(
        !after_wave_0.is_empty(),
        "wave 0 should have finalized at least the leader"
    );

    // This new block must not change already finalized output.
    let dangling = make_block(&creator_2, 3, 9, &round1_support);
    ingest_batch(&mut ingress, &[dangling]);

    let after_extra = ingress
        .latest_finalized_ordered_output(WAVELENGTH)
        .expect("ordered output computation should succeed");

    assert_monotonic(
        &after_wave_0,
        &after_extra,
        "wave 0 -> unrelated dangling growth",
    );
}
