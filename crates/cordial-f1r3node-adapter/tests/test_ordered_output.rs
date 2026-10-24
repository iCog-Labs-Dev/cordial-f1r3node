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
use cordial_f1r3node_adapter::shared_ordered_output::ReadOrderedOutput;
use cordial_f1r3node_adapter::snapshot::SnapshotError;
use cordial_miners_core::Block;
use cordial_miners_core::consensus::{
    OrderingCache, WeightedTauSuffixContext, compute_all_depths, latest_weighted_final_leader,
    weighted_tau, weighted_tau_suffix_with_cache, xsort,
};
use cordial_miners_core::crypto::block_hash;
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
        por_authorizations: vec![],
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
            content_hash: block_hash(creator, &content),
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

fn weighted_wave_zero(validators: &[NodeId; 4], first_tag: u8) -> (Vec<Block>, Vec<Block>) {
    let leader = make_block(&validators[0], 0, first_tag, &[]);
    let round_one: Vec<Block> = validators[1..]
        .iter()
        .enumerate()
        .map(|(index, creator)| {
            make_block(
                creator,
                1,
                first_tag + u8::try_from(index + 1).expect("test tag should fit"),
                std::slice::from_ref(&leader.identity),
            )
        })
        .collect();
    let round_one_ids: Vec<_> = round_one
        .iter()
        .map(|block| block.identity.clone())
        .collect();
    let round_two: Vec<Block> = validators[1..]
        .iter()
        .enumerate()
        .map(|(index, creator)| {
            make_block(
                creator,
                2,
                first_tag + u8::try_from(index + 4).expect("test tag should fit"),
                &round_one_ids,
            )
        })
        .collect();

    let mut wave = vec![leader];
    wave.extend(round_one);
    wave.extend(round_two.clone());
    (wave, round_two)
}

fn weighted_wave_one(
    validators: &[NodeId; 4],
    previous_round: &[Block],
    first_tag: u8,
) -> Vec<Block> {
    let previous_round_ids: Vec<_> = previous_round
        .iter()
        .map(|block| block.identity.clone())
        .collect();
    let leader = make_block(&validators[1], 3, first_tag, &previous_round_ids);
    let followers = [
        validators[0].clone(),
        validators[2].clone(),
        validators[3].clone(),
    ];
    let round_one: Vec<Block> = followers
        .iter()
        .enumerate()
        .map(|(index, creator)| {
            make_block(
                creator,
                4,
                first_tag + u8::try_from(index + 1).expect("test tag should fit"),
                std::slice::from_ref(&leader.identity),
            )
        })
        .collect();
    let round_one_ids: Vec<_> = round_one
        .iter()
        .map(|block| block.identity.clone())
        .collect();
    let round_two: Vec<Block> = followers
        .iter()
        .enumerate()
        .map(|(index, creator)| {
            make_block(
                creator,
                5,
                first_tag + u8::try_from(index + 4).expect("test tag should fit"),
                &round_one_ids,
            )
        })
        .collect();

    let mut wave = vec![leader];
    wave.extend(round_one);
    wave.extend(round_two);
    wave
}

fn live_ingress_single(creator: &NodeId) -> LiveIngress<TestAdapter> {
    let mut bonds = HashMap::new();
    bonds.insert(creator.clone(), 100);

    LiveIngress::with_consensus_view(TestAdapter, bonds, shard_conf(), "root")
}

fn live_ingress_multi(creators: &[NodeId]) -> LiveIngress<TestAdapter> {
    let mut bonds = HashMap::new();
    for creator in creators {
        bonds.insert(creator.clone(), 100);
    }

    LiveIngress::with_consensus_view(TestAdapter, bonds, shard_conf(), "root")
}

fn live_ingress_weighted(validators: &[(NodeId, u64)]) -> LiveIngress<TestAdapter> {
    let bonds = validators.iter().cloned().collect();

    LiveIngress::with_consensus_view(TestAdapter, bonds, shard_conf(), "root")
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

fn assert_matches_full_weighted_tau(
    ingress: &LiveIngress<TestAdapter>,
    output: &OrderedFinalizedOutput,
    bonds: &HashMap<NodeId, u64>,
    wavelength: u64,
    validators: &[NodeId],
) {
    let (expected_blocks, expected_anchor) =
        independent_full_weighted_ordering(ingress, bonds, wavelength, validators);
    assert_eq!(output.blocks, expected_blocks);
    assert_eq!(output.anchor, expected_anchor);
}

fn independent_full_weighted_ordering(
    ingress: &LiveIngress<TestAdapter>,
    bonds: &HashMap<NodeId, u64>,
    wavelength: u64,
    validators: &[NodeId],
) -> (Vec<BlockIdentity>, Option<BlockIdentity>) {
    let mut ordered_validators = validators.to_vec();
    ordered_validators.sort();
    let leader_of_wave = |wave: u64| {
        let index = usize::try_from(wave).ok()? % ordered_validators.len();
        Some(ordered_validators[index].clone())
    };

    let expected_blocks = weighted_tau(ingress.blocklace(), wavelength, bonds, leader_of_wave)
        .expect("independent full weighted tau should compute");
    let expected_anchor =
        latest_weighted_final_leader(ingress.blocklace(), wavelength, bonds, leader_of_wave)
            .map(|anchor| anchor.consensus_identity());
    (expected_blocks, expected_anchor)
}

#[test]
fn unchanged_ordered_output_poll_reuses_the_published_result() {
    let creator = validator(1);
    let blocks = sequential_blocks(&creator, 6);
    let mut ingress = live_ingress_single(&creator);
    ingest_batch(&mut ingress, &blocks);

    let first = ingress
        .latest_finalized_ordered_output(WAVELENGTH)
        .expect("first ordered output should be computed");
    let second = ingress
        .latest_finalized_ordered_output(WAVELENGTH)
        .expect("unchanged poll should return the published output");

    assert_eq!(second, first);
    assert_eq!(
        second.computed_at_ns, first.computed_at_ns,
        "an unchanged poll should reuse the prior output rather than recomputing it"
    );
}

#[test]
fn duplicate_ingestion_does_not_invalidate_or_recompute_ordered_output() {
    let validators = [validator(91), validator(92), validator(93), validator(94)];
    let mut ingress = live_ingress_weighted(
        &validators
            .iter()
            .cloned()
            .map(|id| (id, 100))
            .collect::<Vec<_>>(),
    );
    let (wave_zero, _) = weighted_wave_zero(&validators, 100);
    ingest_batch(&mut ingress, &wave_zero);
    let before = ingress
        .latest_finalized_ordered_output(WAVELENGTH)
        .expect("initial ordered output should compute");
    let generation_before = ingress.blocklace().generation();

    let duplicate = ingress
        .ingest_trusted_block(wave_zero[0].clone())
        .expect("duplicate ingestion should be accepted as a no-op");
    assert_eq!(
        duplicate.disposition,
        cordial_f1r3node_adapter::live_ingress::MirrorDisposition::Duplicate
    );
    assert_eq!(ingress.blocklace().generation(), generation_before);

    let after = ingress
        .latest_finalized_ordered_output(WAVELENGTH)
        .expect("duplicate ingestion must leave ordering available");
    assert_eq!(after, before);
    assert_eq!(
        after.computed_at_ns, before.computed_at_ns,
        "duplicate ingestion should keep the published result without recomputing"
    );
}

#[test]
fn unchanged_empty_ordered_output_poll_reuses_the_published_result() {
    let mut ingress = live_ingress_single(&validator(1));

    let first = ingress
        .latest_finalized_ordered_output(WAVELENGTH)
        .expect("empty ordered output should be computed");
    let second = ingress
        .latest_finalized_ordered_output(WAVELENGTH)
        .expect("unchanged empty poll should reuse the published output");

    assert!(first.anchor.is_none());
    assert!(first.blocks.is_empty());
    assert_eq!(second, first);
}

#[test]
fn ordered_output_recomputes_after_a_blocklace_or_weight_change() {
    let creator = validator(1);
    let blocks = sequential_blocks(&creator, 7);
    let mut ingress = live_ingress_single(&creator);
    ingest_batch(&mut ingress, &blocks[..6]);

    let before_new_block = ingress
        .latest_finalized_ordered_output(WAVELENGTH)
        .expect("initial ordered output should be computed");
    ingress
        .ingest_trusted_block(blocks[6].clone())
        .expect("new block should enter the mirror");
    let after_new_block = ingress
        .latest_finalized_ordered_output(WAVELENGTH)
        .expect("blocklace change should trigger ordering");
    assert_eq!(after_new_block.total_mirrored_blocks, 7);
    assert_ne!(after_new_block, before_new_block);

    ingress.set_bonds(HashMap::from([(creator, 101)]));
    let after_weight_change = ingress
        .latest_finalized_ordered_output(WAVELENGTH)
        .expect("weight change should trigger ordering");

    assert_eq!(after_weight_change.blocks, after_new_block.blocks);
    assert_eq!(after_weight_change.anchor, after_new_block.anchor);
    assert_ne!(
        after_weight_change.computed_at_ns, after_new_block.computed_at_ns,
        "a changed weight snapshot must not return the previously published output"
    );
}

#[test]
fn ordered_output_recomputes_when_validator_membership_changes() {
    let creator = validator(1);
    let blocks = sequential_blocks(&creator, 6);
    let mut ingress = live_ingress_single(&creator);
    ingest_batch(&mut ingress, &blocks);

    let before = ingress
        .latest_finalized_ordered_output(WAVELENGTH)
        .expect("ordered output should be computed");
    ingress.set_bonds(HashMap::from([(validator(2), 100)]));

    let error = ingress
        .latest_finalized_ordered_output(WAVELENGTH)
        .expect_err("new membership must not reuse the old validator's output");
    assert_eq!(error, SnapshotError::OrderedOutputPrefixViolation);
    assert_eq!(
        ingress.ordered_output_reader().latest(),
        Some(&before),
        "a rejected membership-change output must leave the prior publication intact"
    );
    let changed_bonds = HashMap::from([(validator(2), 100)]);
    let expected_blocks = weighted_tau(ingress.blocklace(), WAVELENGTH, &changed_bonds, |_| {
        Some(validator(2))
    })
    .expect("independent full ordering should compute for changed membership");
    let expected_anchor =
        latest_weighted_final_leader(ingress.blocklace(), WAVELENGTH, &changed_bonds, |_| {
            Some(validator(2))
        });
    assert!(expected_blocks.is_empty());
    assert!(expected_anchor.is_none());
}

#[test]
fn ordered_output_recomputes_when_wavelength_changes() {
    let creator = validator(1);
    let blocks = sequential_blocks(&creator, 6);
    let mut ingress = live_ingress_single(&creator);
    ingest_batch(&mut ingress, &blocks);

    let first = ingress
        .latest_finalized_ordered_output(WAVELENGTH)
        .expect("ordered output should be computed");
    let bonds = HashMap::from([(creator.clone(), 100)]);
    let depths = compute_all_depths(ingress.blocklace());
    let new_wavelength = WAVELENGTH + 1;
    let latest_wave = depths.values().copied().max().unwrap_or_default() / new_wavelength;
    let full_anchor = (0..=latest_wave).rev().find_map(|wave| {
        let first_round = wave * new_wavelength;
        let last_round = first_round + new_wavelength - 1;
        let leaders: Vec<_> = (first_round..=last_round)
            .map(|round| {
                let mut round_blocks = depths
                    .iter()
                    .filter(|(id, depth)| **depth == round && id.creator == creator)
                    .map(|(id, _)| id.clone());
                let leader = round_blocks.next();
                (leader, round_blocks.next().is_none())
            })
            .collect();
        if leaders
            .iter()
            .all(|(leader, unique)| leader.is_some() && *unique)
        {
            leaders.first().and_then(|(leader, _)| leader.clone())
        } else {
            None
        }
    });
    let full_blocks = full_anchor
        .as_ref()
        .map(|anchor| {
            let observed = ingress
                .blocklace()
                .observe(anchor)
                .into_iter()
                .filter_map(|id| ingress.blocklace().get(&id))
                .collect();
            xsort(&observed).expect("full single-validator observation order should compute")
        })
        .unwrap_or_default();
    let candidate = OrderedFinalizedOutput::new(
        full_blocks,
        full_anchor,
        new_wavelength,
        bonds.len(),
        ingress.blocklace().dom().len(),
    );
    assert!(
        !candidate.preserves_prefix(&first),
        "new wavelength should produce a different finalized prefix in this fixture"
    );
    assert_eq!(
        ingress.latest_finalized_ordered_output(WAVELENGTH + 1),
        Err(SnapshotError::OrderedOutputPrefixViolation)
    );
    assert_eq!(ingress.ordered_output_reader().latest(), Some(&first));
}

#[test]
fn verified_prefix_extension_matches_full_weighted_tau() {
    let validators = [validator(31), validator(32), validator(33), validator(34)];
    let bonds: HashMap<NodeId, u64> = validators
        .iter()
        .cloned()
        .map(|validator| (validator, 100))
        .collect();
    let mut ingress = live_ingress_weighted(
        &validators
            .iter()
            .map(|validator| (validator.clone(), 100))
            .collect::<Vec<_>>(),
    );

    let wave_0_leader = make_block(&validators[0], 0, 1, &[]);
    let wave_0_round_1: Vec<Block> = validators[1..]
        .iter()
        .enumerate()
        .map(|(index, validator)| {
            make_block(
                validator,
                1,
                u8::try_from(index + 2).expect("test tag should fit"),
                std::slice::from_ref(&wave_0_leader.identity),
            )
        })
        .collect();
    let wave_0_round_1_ids: Vec<_> = wave_0_round_1
        .iter()
        .map(|block| block.identity.clone())
        .collect();
    let wave_0_round_2: Vec<Block> = validators[1..]
        .iter()
        .enumerate()
        .map(|(index, validator)| {
            make_block(
                validator,
                2,
                u8::try_from(index + 5).expect("test tag should fit"),
                &wave_0_round_1_ids,
            )
        })
        .collect();

    let mut wave_0 = vec![wave_0_leader];
    wave_0.extend(wave_0_round_1);
    wave_0.extend(wave_0_round_2.clone());
    ingest_batch(&mut ingress, &wave_0);
    let previous = ingress
        .latest_finalized_ordered_output(WAVELENGTH)
        .expect("wave 0 output should be computed");
    assert_eq!(
        previous.anchor.as_ref(),
        Some(&wave_0[0].identity.consensus_identity())
    );

    let wave_1_leader = make_block(
        &validators[1],
        3,
        8,
        &wave_0_round_2
            .iter()
            .map(|block| block.identity.clone())
            .collect::<Vec<_>>(),
    );
    let wave_1_round_1: Vec<Block> = [
        validators[0].clone(),
        validators[2].clone(),
        validators[3].clone(),
    ]
    .iter()
    .enumerate()
    .map(|(index, validator)| {
        make_block(
            validator,
            4,
            u8::try_from(index + 9).expect("test tag should fit"),
            std::slice::from_ref(&wave_1_leader.identity),
        )
    })
    .collect();
    let wave_1_round_1_ids: Vec<_> = wave_1_round_1
        .iter()
        .map(|block| block.identity.clone())
        .collect();
    let wave_1_round_2: Vec<Block> = [
        validators[0].clone(),
        validators[2].clone(),
        validators[3].clone(),
    ]
    .iter()
    .enumerate()
    .map(|(index, validator)| {
        make_block(
            validator,
            5,
            u8::try_from(index + 12).expect("test tag should fit"),
            &wave_1_round_1_ids,
        )
    })
    .collect();

    let mut wave_1 = vec![wave_1_leader];
    wave_1.extend(wave_1_round_1);
    wave_1.extend(wave_1_round_2);
    ingest_batch(&mut ingress, &wave_1);

    let incremental = ingress
        .latest_finalized_ordered_output(WAVELENGTH)
        .expect("verified later-wave extension should be computed");
    let mut ordered_validators = validators.to_vec();
    ordered_validators.sort();
    let leader_of_wave = |wave: u64| {
        let index = usize::try_from(wave).ok()? % ordered_validators.len();
        Some(ordered_validators[index].clone())
    };
    let expected_blocks = weighted_tau(ingress.blocklace(), WAVELENGTH, &bonds, leader_of_wave)
        .expect("independent full weighted tau should compute");
    let expected_anchor =
        latest_weighted_final_leader(ingress.blocklace(), WAVELENGTH, &bonds, leader_of_wave);

    let mut cache = OrderingCache::default();
    let verified_suffix = weighted_tau_suffix_with_cache(
        ingress.blocklace(),
        WeightedTauSuffixContext {
            wavelength: WAVELENGTH,
            bonds: &bonds,
            leader_selection_id: 0,
            leader_selection: leader_of_wave,
        },
        previous
            .anchor
            .as_ref()
            .expect("first wave should have an anchor"),
        &previous.blocks,
        &mut cache,
    )
    .expect("verified suffix calculation should not fail");

    assert!(
        verified_suffix.is_some(),
        "signature-independent anchor should validate against its signed DAG identity"
    );
    assert_eq!(incremental.blocks, expected_blocks);
    assert_eq!(
        verified_suffix
            .expect("eligible extension should produce a suffix")
            .0,
        expected_blocks
    );
    assert_eq!(
        incremental.anchor,
        expected_anchor.map(|anchor| anchor.consensus_identity())
    );
    assert!(incremental.preserves_prefix(&previous));
}

#[test]
fn fork_before_finalization_does_not_change_output_from_full_recomputation() {
    let validators = [validator(41), validator(42), validator(43), validator(44)];
    let bonds: HashMap<_, _> = validators.iter().cloned().map(|id| (id, 100)).collect();
    let mut ingress = live_ingress_weighted(
        &validators
            .iter()
            .cloned()
            .map(|id| (id, 100))
            .collect::<Vec<_>>(),
    );

    let (wave_zero, round_two) = weighted_wave_zero(&validators, 20);
    ingest_batch(&mut ingress, &wave_zero);
    let prior = ingress
        .latest_finalized_ordered_output(WAVELENGTH)
        .expect("wave zero should finalize");

    let mut wave_one = weighted_wave_one(&validators, &round_two, 30);
    let competing_leader = make_block(
        &validators[1],
        3,
        39,
        &round_two
            .iter()
            .map(|block| block.identity.clone())
            .collect::<Vec<_>>(),
    );
    wave_one.push(competing_leader);
    ingest_batch(&mut ingress, &wave_one);

    let current = ingress
        .latest_finalized_ordered_output(WAVELENGTH)
        .expect("forked DAG should still produce its full finalized ordering");
    assert_matches_full_weighted_tau(&ingress, &current, &bonds, WAVELENGTH, &validators);
    assert_monotonic(
        &prior,
        &current,
        "wave-zero output after wave-one leader fork",
    );
    assert_eq!(
        current.anchor, prior.anchor,
        "the equivocated wave-one leader must not be selected as final"
    );
}

#[test]
fn late_equivocating_leader_cannot_rewrite_a_published_prefix() {
    let validators = [validator(51), validator(52), validator(53), validator(54)];
    let bonds: HashMap<_, _> = validators.iter().cloned().map(|id| (id, 100)).collect();
    let mut ingress = live_ingress_weighted(
        &validators
            .iter()
            .cloned()
            .map(|id| (id, 100))
            .collect::<Vec<_>>(),
    );

    let (wave_zero, round_two) = weighted_wave_zero(&validators, 40);
    let wave_one = weighted_wave_one(&validators, &round_two, 50);
    ingest_batch(&mut ingress, &wave_zero);
    ingest_batch(&mut ingress, &wave_one);
    let published = ingress
        .latest_finalized_ordered_output(WAVELENGTH)
        .expect("both waves should finalize before the late fork");
    assert!(
        published
            .anchor
            .as_ref()
            .is_some_and(|anchor| { anchor.same_consensus_identity(&wave_one[0].identity) }),
        "wave one should be the published finality anchor"
    );

    let late_fork = make_block(
        &validators[1],
        3,
        59,
        &round_two
            .iter()
            .map(|block| block.identity.clone())
            .collect::<Vec<_>>(),
    );
    ingress
        .ingest_trusted_block(late_fork)
        .expect("late competing leader should be ingested");

    let full_blocks = weighted_tau(ingress.blocklace(), WAVELENGTH, &bonds, |wave| {
        let mut ordered = validators.to_vec();
        ordered.sort();
        Some(ordered[usize::try_from(wave).ok()? % ordered.len()].clone())
    })
    .expect("full ordering should still compute after the late fork");
    assert!(
        !full_blocks.starts_with(&published.blocks),
        "late equivocation should invalidate the previously selected wave-one output"
    );
    assert_eq!(
        ingress.latest_finalized_ordered_output(WAVELENGTH),
        Err(SnapshotError::OrderedOutputPrefixViolation),
        "the append-only publisher must reject a full recomputation that rewrites finality"
    );
    assert_eq!(
        ingress.ordered_output_reader().latest(),
        Some(&published),
        "rejected late-fork output must not replace the published prefix"
    );
}

#[test]
fn restart_with_stale_same_size_prefix_falls_back_to_full_ordering() {
    let validators = [validator(61), validator(62), validator(63), validator(64)];
    let bonds: HashMap<_, _> = validators.iter().cloned().map(|id| (id, 100)).collect();
    let configured = validators
        .iter()
        .cloned()
        .map(|id| (id, 100))
        .collect::<Vec<_>>();
    let (old_wave, _) = weighted_wave_zero(&validators, 60);
    let (replacement_wave, _) = weighted_wave_zero(&validators, 70);

    let mut old_ingress = live_ingress_weighted(&configured);
    ingest_batch(&mut old_ingress, &old_wave);
    let stale_output = old_ingress
        .latest_finalized_ordered_output(WAVELENGTH)
        .expect("old mirror should produce an output");

    let mut restarted_ingress = live_ingress_weighted(&configured);
    ingest_batch(&mut restarted_ingress, &replacement_wave);
    assert_eq!(
        old_ingress.blocklace().generation(),
        restarted_ingress.blocklace().generation(),
        "restart mirrors should have matching generation numbers"
    );
    assert_eq!(
        old_ingress.blocklace().dom().len(),
        restarted_ingress.blocklace().dom().len(),
        "restart mirrors should have matching sizes"
    );

    let mut cache = OrderingCache::default();
    let stale_extension = weighted_tau_suffix_with_cache(
        restarted_ingress.blocklace(),
        WeightedTauSuffixContext {
            wavelength: WAVELENGTH,
            bonds: &bonds,
            leader_selection_id: 0,
            leader_selection: |wave| {
                let mut ordered = validators.to_vec();
                ordered.sort();
                Some(ordered[usize::try_from(wave).ok()? % ordered.len()].clone())
            },
        },
        stale_output
            .anchor
            .as_ref()
            .expect("old mirror should have a finality anchor"),
        &stale_output.blocks,
        &mut cache,
    )
    .expect("stale-state detection should not fail");
    assert!(
        stale_extension.is_none(),
        "a saved prefix from another same-size mirror must not be reused"
    );

    let restarted_output = restarted_ingress
        .latest_finalized_ordered_output(WAVELENGTH)
        .expect("restart without prefix state should compute from scratch");
    assert_matches_full_weighted_tau(
        &restarted_ingress,
        &restarted_output,
        &bonds,
        WAVELENGTH,
        &validators,
    );
    assert!(restarted_output.anchor.is_some());
}

#[test]
fn weight_and_wavelength_changes_match_independent_full_ordering() {
    let validators = [validator(81), validator(82), validator(83), validator(84)];
    let initial_bonds: HashMap<_, _> = validators.iter().cloned().map(|id| (id, 100)).collect();
    let mut ingress = live_ingress_weighted(
        &validators
            .iter()
            .cloned()
            .map(|id| (id, 100))
            .collect::<Vec<_>>(),
    );
    let (wave_zero, round_two) = weighted_wave_zero(&validators, 80);
    let wave_one = weighted_wave_one(&validators, &round_two, 90);
    ingest_batch(&mut ingress, &wave_zero);
    ingest_batch(&mut ingress, &wave_one);
    let mut previously_published = ingress
        .latest_finalized_ordered_output(WAVELENGTH)
        .expect("initial output should compute");
    assert_matches_full_weighted_tau(
        &ingress,
        &previously_published,
        &initial_bonds,
        WAVELENGTH,
        &validators,
    );

    let changed_bonds: HashMap<_, _> = [
        (validators[0].clone(), 1_000),
        (validators[1].clone(), 100),
        (validators[2].clone(), 100),
        (validators[3].clone(), 100),
    ]
    .into_iter()
    .collect();
    ingress.set_bonds(changed_bonds.clone());
    let (full_blocks, full_anchor) =
        independent_full_weighted_ordering(&ingress, &changed_bonds, WAVELENGTH, &validators);
    let expected = OrderedFinalizedOutput::new(
        full_blocks,
        full_anchor,
        WAVELENGTH,
        changed_bonds.len(),
        ingress.blocklace().dom().len(),
    );
    if expected.preserves_prefix(&previously_published) {
        let actual = ingress
            .latest_finalized_ordered_output(WAVELENGTH)
            .expect("weight update preserving prefix should publish");
        assert_eq!(actual.blocks, expected.blocks);
        assert_eq!(actual.anchor, expected.anchor);
        previously_published = actual;
    } else {
        assert_eq!(
            ingress.latest_finalized_ordered_output(WAVELENGTH),
            Err(SnapshotError::OrderedOutputPrefixViolation)
        );
        assert_eq!(
            ingress.ordered_output_reader().latest(),
            Some(&previously_published),
            "weight changes must not rewrite a previously published prefix"
        );
    }

    let changed_wavelength = WAVELENGTH + 1;
    let (full_blocks, full_anchor) = independent_full_weighted_ordering(
        &ingress,
        &changed_bonds,
        changed_wavelength,
        &validators,
    );
    let expected = OrderedFinalizedOutput::new(
        full_blocks,
        full_anchor,
        changed_wavelength,
        changed_bonds.len(),
        ingress.blocklace().dom().len(),
    );
    if expected.preserves_prefix(&previously_published) {
        let actual = ingress
            .latest_finalized_ordered_output(changed_wavelength)
            .expect("wavelength update preserving prefix should publish");
        assert_eq!(actual.blocks, expected.blocks);
        assert_eq!(actual.anchor, expected.anchor);
    } else {
        assert_eq!(
            ingress.latest_finalized_ordered_output(changed_wavelength),
            Err(SnapshotError::OrderedOutputPrefixViolation)
        );
        assert_eq!(
            ingress.ordered_output_reader().latest(),
            Some(&previously_published),
            "wavelength changes must not rewrite a previously published prefix"
        );
    }
}

#[test]
fn empty_output_has_no_anchor_and_matches_full_recomputation() {
    let validators = [validator(71), validator(72), validator(73), validator(74)];
    let bonds: HashMap<_, _> = validators.iter().cloned().map(|id| (id, 100)).collect();
    let mut ingress = live_ingress_weighted(
        &validators
            .iter()
            .cloned()
            .map(|id| (id, 100))
            .collect::<Vec<_>>(),
    );

    let empty = ingress
        .latest_finalized_ordered_output(WAVELENGTH)
        .expect("empty mirror should compute an empty finalized output");
    assert!(empty.blocks.is_empty());
    assert!(empty.anchor.is_none());
    assert_matches_full_weighted_tau(&ingress, &empty, &bonds, WAVELENGTH, &validators);
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

    let round1_v2 = make_block(&creator_2, 1, 2, std::slice::from_ref(&leader.identity));
    let round1_v3 = make_block(&creator_3, 1, 3, std::slice::from_ref(&leader.identity));
    let round1_v4 = make_block(&creator_4, 1, 4, std::slice::from_ref(&leader.identity));

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
    let round1_v2 = make_block(&creator_2, 1, 2, std::slice::from_ref(&leader.identity));
    let round1_v3 = make_block(&creator_3, 1, 3, std::slice::from_ref(&leader.identity));
    let round1_v4 = make_block(&creator_4, 1, 4, std::slice::from_ref(&leader.identity));

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
