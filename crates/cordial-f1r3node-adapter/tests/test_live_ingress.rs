use std::collections::{HashMap, HashSet};

use cordial_f1r3node_adapter::block_translation::{
    BlockMessage, Body, F1r3flyState, Header, Justification,
};
use cordial_f1r3node_adapter::grpc_ingest::BlocklaceAdapter;
use cordial_f1r3node_adapter::live_ingress::{LiveIngress, LiveIngressError, LiveIngressPhase};
use cordial_f1r3node_adapter::shard_conf::CasperShardConf;
use cordial_miners_core::Block;
use cordial_miners_core::crypto::{hash_content, sign};
use cordial_miners_core::execution::{BlockState, CordialBlockPayload};
use cordial_miners_core::types::{BlockContent, BlockIdentity, NodeId};

/// Number of rounds in one consensus wave which same as `ES_WAVELENGTH` in `snapshot.rs`.
const WAVELENGTH: u64 = 3;

#[test]
fn new_live_ingress_starts_in_defined_phase() {
    let ingress = LiveIngress::new(());
    assert_eq!(ingress.phase(), LiveIngressPhase::Defined);
}

#[test]
fn live_ingress_phase_can_progress_without_changing_adapter() {
    let mut ingress = LiveIngress::new(String::from("adapter"));

    ingress.mark_traced();
    assert_eq!(ingress.phase(), LiveIngressPhase::Traced);
    assert_eq!(ingress.adapter(), "adapter");

    ingress.mark_connected();
    assert_eq!(ingress.phase(), LiveIngressPhase::Connected);
    assert_eq!(ingress.into_inner(), "adapter");
}

#[test]
fn live_ingress_routes_valid_block_messages_through_grpc_ingest() {
    let signing_key = test_signing_key(7);
    let creator = test_public_key(&signing_key);
    let block_msg = build_test_block_message(&creator, &[], &signing_key, "secp256k1");

    let mut ingress = LiveIngress::new(RecordingAdapter::default());
    let block = ingress
        .ingest_block_message(&block_msg)
        .expect("live ingress should accept a valid block message");

    assert_eq!(ingress.phase(), LiveIngressPhase::Connected);
    assert_eq!(
        block.identity.content_hash,
        <[u8; 32]>::try_from(block_msg.block_hash.as_slice()).expect("valid 32-byte block hash")
    );
    assert_eq!(ingress.adapter().callback_count, 1);
    assert_eq!(ingress.adapter().received_blocks.len(), 1);
    assert_eq!(ingress.blocklace().dom().len(), 1);
    assert!(ingress.pending_blocks().is_empty());
}

#[test]
fn live_ingress_surfaces_adapter_rejections_after_mapping() {
    let signing_key = test_signing_key(9);
    let creator = test_public_key(&signing_key);
    let block_msg = build_test_block_message(&creator, &[], &signing_key, "secp256k1");

    let mut ingress = LiveIngress::new(RecordingAdapter::rejecting());
    let err = ingress
        .ingest_block_message(&block_msg)
        .expect_err("adapter rejection should be surfaced");

    match err {
        LiveIngressError::Adapter(inner) => {
            assert!(inner.to_string().contains("Adapter rejected block"));
        }
        other => panic!("expected adapter error, got {other:?}"),
    }

    assert_eq!(ingress.phase(), LiveIngressPhase::Defined);
    assert_eq!(ingress.adapter().callback_count, 1);
    assert!(ingress.adapter().received_blocks.is_empty());
    assert_eq!(ingress.blocklace().dom().len(), 0);
    assert!(ingress.pending_blocks().is_empty());
}

#[test]
fn live_ingress_buffers_out_of_order_blocks_until_predecessors_arrive() {
    let parent_signing_key = test_signing_key(11);
    let parent_creator = test_public_key(&parent_signing_key);
    let parent = build_test_block_message(&parent_creator, &[], &parent_signing_key, "secp256k1");

    let child_signing_key = test_signing_key(12);
    let child_creator = test_public_key(&child_signing_key);
    let child = build_test_block_message(
        &child_creator,
        &[(parent.block_hash.clone(), parent.sender.clone())],
        &child_signing_key,
        "secp256k1",
    );

    let mut ingress = LiveIngress::new(RecordingAdapter::default());

    ingress
        .ingest_block_message(&child)
        .expect("child block should be accepted into pending state");
    assert_eq!(ingress.blocklace().dom().len(), 0);
    assert_eq!(ingress.pending_blocks().len(), 1);

    ingress
        .ingest_block_message(&parent)
        .expect("parent block should release buffered child");
    assert_eq!(ingress.blocklace().dom().len(), 2);
    assert!(ingress.pending_blocks().is_empty());
}

#[test]
fn live_ingress_ignores_duplicate_blocks_in_mirror_state() {
    let signing_key = test_signing_key(13);
    let creator = test_public_key(&signing_key);
    let block_msg = build_test_block_message(&creator, &[], &signing_key, "secp256k1");

    let mut ingress = LiveIngress::new(RecordingAdapter::default());

    ingress
        .ingest_block_message(&block_msg)
        .expect("first block ingestion should succeed");
    ingress
        .ingest_block_message(&block_msg)
        .expect("duplicate block ingestion should not error");

    assert_eq!(ingress.blocklace().dom().len(), 1);
    assert!(ingress.pending_blocks().is_empty());
}

#[test]
fn live_ingress_exposes_snapshot_and_finality_over_mirrored_state() {
    let shard_conf = CasperShardConf {
        shard_name: "root".to_string(),
        max_number_of_parents: 16,
        fault_tolerance_threshold: 0.333,
        deploy_lifespan: 50,
        min_phlo_price: 1,
        ..CasperShardConf::default()
    };

    let signing_key_1 = test_signing_key(21);
    let creator_1 = test_public_key(&signing_key_1);
    let signing_key_2 = test_signing_key(22);
    let creator_2 = test_public_key(&signing_key_2);
    let signing_key_3 = test_signing_key(23);
    let creator_3 = test_public_key(&signing_key_3);
    let signing_key_4 = test_signing_key(24);
    let creator_4 = test_public_key(&signing_key_4);

    let mut bonds = HashMap::new();
    bonds.insert(NodeId(creator_1.clone()), 100);
    bonds.insert(NodeId(creator_2.clone()), 100);
    bonds.insert(NodeId(creator_3.clone()), 100);
    bonds.insert(NodeId(creator_4.clone()), 100);

    let mut ingress =
        LiveIngress::with_consensus_view(RecordingAdapter::default(), bonds, shard_conf, "root");

    let leader =
        build_test_block_message_with_state(&creator_1, &[], &signing_key_1, "secp256k1", 0, 1);

    let round1_v2 = build_test_block_message_with_state(
        &creator_2,
        &[(leader.block_hash.clone(), leader.sender.clone())],
        &signing_key_2,
        "secp256k1",
        1,
        2,
    );
    let round1_v3 = build_test_block_message_with_state(
        &creator_3,
        &[(leader.block_hash.clone(), leader.sender.clone())],
        &signing_key_3,
        "secp256k1",
        1,
        3,
    );
    let round1_v4 = build_test_block_message_with_state(
        &creator_4,
        &[(leader.block_hash.clone(), leader.sender.clone())],
        &signing_key_4,
        "secp256k1",
        1,
        4,
    );

    let round1_support = [
        (round1_v2.block_hash.clone(), round1_v2.sender.clone()),
        (round1_v3.block_hash.clone(), round1_v3.sender.clone()),
        (round1_v4.block_hash.clone(), round1_v4.sender.clone()),
    ];

    let round2_v2 = build_test_block_message_with_state(
        &creator_2,
        &round1_support,
        &signing_key_2,
        "secp256k1",
        2,
        5,
    );
    let round2_v3 = build_test_block_message_with_state(
        &creator_3,
        &round1_support,
        &signing_key_3,
        "secp256k1",
        2,
        6,
    );
    let round2_v4 = build_test_block_message_with_state(
        &creator_4,
        &round1_support,
        &signing_key_4,
        "secp256k1",
        2,
        7,
    );

    for block_msg in [
        &round2_v3, &round1_v2, &round2_v2, &leader, &round1_v4, &round2_v4, &round1_v3,
    ] {
        ingress
            .ingest_block_message(block_msg)
            .expect("live block wave should mirror successfully");
    }

    let snapshot = ingress
        .snapshot()
        .expect("mirrored wave should build a live snapshot");
    let last_finalized = ingress
        .last_finalized_block_hash()
        .expect("finality lookup should succeed");
    let ordered = ingress
        .ordered_finalized_blocks()
        .expect("weighted tau lookup should succeed");

    assert_eq!(snapshot.last_finalized_block, leader.block_hash);
    assert_eq!(last_finalized, Some(leader.block_hash.clone()));
    assert!(snapshot.dag.dag_set.contains(&leader.block_hash));
    assert!(
        snapshot
            .dag
            .finalized_blocks_set
            .contains(&leader.block_hash)
    );
    assert!(!ordered.is_empty());
    assert_eq!(ordered, snapshot.ordered_finalized_blocks);
    assert!(ordered.contains(&leader.block_hash));
    assert_eq!(snapshot.on_chain_state.shard_conf.shard_name, "root");
    assert_eq!(snapshot.on_chain_state.active_validators.len(), 4);
}

#[test]
fn latest_ordered_output_is_monotonic_across_batches() {
    let shard_conf = CasperShardConf {
        shard_name: "root".to_string(),
        max_number_of_parents: 16,
        fault_tolerance_threshold: 0.333,
        deploy_lifespan: 50,
        min_phlo_price: 1,
        ..CasperShardConf::default()
    };

    let signing_key = test_signing_key(41);
    let creator = test_public_key(&signing_key);

    let mut bonds = HashMap::new();
    bonds.insert(NodeId(creator.clone()), 100);

    let mut ingress =
        LiveIngress::with_consensus_view(RecordingAdapter::default(), bonds, shard_conf, "root");

    // Build 6 sequential blocks forming 2 waves (wavelength=3).
    // Each block references the previous one as predecessor.
    let mut blocks = Vec::new();
    let mut prev_hash = Vec::new();
    for i in 0..6u64 {
        let parents = if i == 0 {
            vec![]
        } else {
            vec![(prev_hash.clone(), creator.clone())]
        };
        let block_msg = build_test_block_message_with_state(
            &creator,
            &parents,
            &signing_key,
            "secp256k1",
            i,
            (i + 1) as u8,
        );
        prev_hash = block_msg.block_hash.clone();
        blocks.push(block_msg);
    }

    // Batch 1: ingest first 3 blocks (completes wave 0).
    for block_msg in &blocks[0..3] {
        ingress
            .ingest_block_message(block_msg)
            .expect("batch 1 block should mirror");
    }
    let first_output = ingress
        .latest_finalized_ordered_output(WAVELENGTH)
        .expect("first batch ordered output should be computed");

    // Batch 2: ingest remaining 3 blocks (completes wave 1).
    for block_msg in &blocks[3..6] {
        ingress
            .ingest_block_message(block_msg)
            .expect("batch 2 block should mirror");
    }
    let second_output = ingress
        .latest_finalized_ordered_output(WAVELENGTH)
        .expect("second batch ordered output should be computed");

    // The first batch's blocks must be a prefix of the second batch's blocks.
    assert!(
        second_output.blocks.starts_with(&first_output.blocks),
        "ordered output must be monotonic: first batch should be prefix of second. \
         first_hashes={:?}, second_hashes={:?}",
        first_output.block_hashes(),
        second_output.block_hashes(),
    );

    // Sanity: after 2 full waves, both wave leaders are in the output.
    assert!(
        first_output
            .blocks
            .iter()
            .any(|id| id.content_hash == blocks[0].block_hash.as_slice()),
        "genesis (wave 0 leader) should be finalized after wave 0"
    );
    assert!(
        second_output
            .blocks
            .iter()
            .any(|id| id.content_hash == blocks[3].block_hash.as_slice()),
        "block 3 (wave 1 leader) should be finalized after wave 1"
    );
}

#[test]
fn latest_ordered_output_rejects_same_round_fork() {
    let shard_conf = CasperShardConf {
        shard_name: "root".to_string(),
        max_number_of_parents: 16,
        fault_tolerance_threshold: 0.333,
        deploy_lifespan: 50,
        min_phlo_price: 1,
        ..CasperShardConf::default()
    };

    let signing_key = test_signing_key(41);
    let creator = test_public_key(&signing_key);

    let mut bonds = HashMap::new();
    bonds.insert(NodeId(creator.clone()), 100);

    let mut ingress =
        LiveIngress::with_consensus_view(RecordingAdapter::default(), bonds, shard_conf, "root");

    // Genesis block (round 0), no predecessors.
    let genesis =
        build_test_block_message_with_state(&creator, &[], &signing_key, "secp256k1", 0, 1);
    ingress
        .ingest_block_message(&genesis)
        .expect("genesis should mirror");

    // Two distinct blocks, same creator, same round (block_number = 2),
    // both built on genesis. Distinct `i` index (1 vs 2) should be enough
    // to produce distinct content hashes while landing in the same round —
    // this is the equivocation scenario.
    let fork_a = build_test_block_message_with_state(
        &creator,
        &[(genesis.block_hash.clone(), creator.clone())],
        &signing_key,
        "secp256k1",
        1,
        2,
    );
    let fork_b = build_test_block_message_with_state(
        &creator,
        &[(genesis.block_hash.clone(), creator.clone())],
        &signing_key,
        "secp256k1",
        2,
        2,
    );

    assert_ne!(
        fork_a.block_hash, fork_b.block_hash,
        "fork blocks must be distinct for this to actually be equivocation"
    );

    ingress
        .ingest_block_message(&fork_a)
        .expect("fork_a should mirror");
    ingress
        .ingest_block_message(&fork_b)
        .expect("fork_b should mirror");

    let output = ingress
        .latest_finalized_ordered_output(WAVELENGTH)
        .expect("ordered output should be computed during equivocation");

    // Equivocation must be terminal: no anchor, no ordered blocks — and
    // critically, this must NOT silently fall through to a weighted_tau
    // result that trusts the equivocating validator's frontier.
    assert!(
        output.anchor.is_none(),
        "anchor must be None when the sole bonded validator has equivocated, got {:?}",
        output.anchor_hash()
    );
    assert!(
        output.blocks.is_empty(),
        "blocks must be empty when the sole bonded validator has equivocated, got {:?}",
        output.block_hashes()
    );
}

#[test]
fn last_finalized_block_hash_and_latest_ordered_output_agree_during_fork() {
    let shard_conf = CasperShardConf {
        shard_name: "root".to_string(),
        max_number_of_parents: 16,
        fault_tolerance_threshold: 0.333,
        deploy_lifespan: 50,
        min_phlo_price: 1,
        ..CasperShardConf::default()
    };

    let signing_key = test_signing_key(41);
    let creator = test_public_key(&signing_key);

    let mut bonds = HashMap::new();
    bonds.insert(NodeId(creator.clone()), 100);

    let mut ingress =
        LiveIngress::with_consensus_view(RecordingAdapter::default(), bonds, shard_conf, "root");

    // Genesis block (round 0), no predecessors.
    let genesis =
        build_test_block_message_with_state(&creator, &[], &signing_key, "secp256k1", 0, 1);
    ingress
        .ingest_block_message(&genesis)
        .expect("genesis should mirror");

    // Two distinct blocks, same creator, same round (block_number = 2),
    // both built on genesis — this is the equivocation scenario.
    let fork_a = build_test_block_message_with_state(
        &creator,
        &[(genesis.block_hash.clone(), creator.clone())],
        &signing_key,
        "secp256k1",
        1,
        2,
    );
    let fork_b = build_test_block_message_with_state(
        &creator,
        &[(genesis.block_hash.clone(), creator.clone())],
        &signing_key,
        "secp256k1",
        2,
        2,
    );

    assert_ne!(
        fork_a.block_hash, fork_b.block_hash,
        "fork blocks must be distinct for this to actually be equivocation"
    );

    ingress
        .ingest_block_message(&fork_a)
        .expect("fork_a should mirror");
    ingress
        .ingest_block_message(&fork_b)
        .expect("fork_b should mirror");

    let last_finalized = ingress
        .last_finalized_block_hash()
        .expect("should not error");
    let ordered_output = ingress
        .latest_finalized_ordered_output(WAVELENGTH)
        .expect("ordered output finality lookup should succeed during equivocation");

    assert!(
        last_finalized.is_none(),
        "last_finalized_block_hash must be None during equivocation, got {:?}",
        last_finalized
    );
    assert!(
        ordered_output.anchor.is_none(),
        "anchor must be None during equivocation, got {:?}",
        ordered_output.anchor_hash()
    );
}

#[test]
fn latest_ordered_output_before_first_complete_wave() {
    let shard_conf = CasperShardConf {
        shard_name: "root".to_string(),
        max_number_of_parents: 16,
        fault_tolerance_threshold: 0.333,
        deploy_lifespan: 50,
        min_phlo_price: 1,
        ..CasperShardConf::default()
    };

    let signing_key = test_signing_key(41);
    let creator = test_public_key(&signing_key);

    let mut bonds = HashMap::new();
    bonds.insert(NodeId(creator.clone()), 100);

    let mut ingress =
        LiveIngress::with_consensus_view(RecordingAdapter::default(), bonds, shard_conf, "root");

    // With no blocks ingested at all, the output must be empty and must
    // not panic — this exercises the `leaders` non-empty but
    // `compute_all_depths` empty edge before any wave exists.
    let empty_output = ingress
        .latest_finalized_ordered_output(WAVELENGTH)
        .expect("ordered output should be computed before any blocks are ingested");
    assert!(empty_output.blocks.is_empty());
    assert!(empty_output.anchor.is_none());

    // Ingest only 2 of the 3 blocks needed to complete wave 0
    // (wavelength = 3, so rounds 0..=2 must all be present and singular).
    let genesis =
        build_test_block_message_with_state(&creator, &[], &signing_key, "secp256k1", 0, 1);
    ingress
        .ingest_block_message(&genesis)
        .expect("genesis should mirror");

    let round_two = build_test_block_message_with_state(
        &creator,
        &[(genesis.block_hash.clone(), creator.clone())],
        &signing_key,
        "secp256k1",
        1,
        2,
    );
    ingress
        .ingest_block_message(&round_two)
        .expect("round_two should mirror");

    // Wave 0 needs rounds 0, 1, 2 — round 2 (block_number 3) is missing.
    // This must not panic and should not fabricate a leader/blocks; it's
    // the genuine "no complete wave yet" case that the single-validator
    // fast path falls through on, deferring to weighted_tau.
    let partial_output = ingress
        .latest_finalized_ordered_output(WAVELENGTH)
        .expect("ordered output should be computed before the first complete wave");

    // We don't assert a specific non-empty/empty outcome here since
    // weighted_tau may or may not detect finality via self-ratification
    // depending on core's exact semantics — the point of this test is that
    // it completes without panicking and stays internally consistent:
    // an anchor implies at least one block, and vice versa.
    assert_eq!(
        partial_output.anchor.is_some(),
        !partial_output.blocks.is_empty(),
        "anchor and blocks must agree on whether anything is finalized yet: \
         anchor={:?}, blocks={:?}",
        partial_output.anchor_hash(),
        partial_output.block_hashes(),
    );
}

#[derive(Default)]
struct RecordingAdapter {
    received_blocks: Vec<Block>,
    callback_count: usize,
    reject: bool,
}

impl RecordingAdapter {
    fn rejecting() -> Self {
        Self {
            received_blocks: Vec::new(),
            callback_count: 0,
            reject: true,
        }
    }
}

impl BlocklaceAdapter<BlockIdentity> for RecordingAdapter {
    fn on_block(&mut self, block: Block) -> anyhow::Result<()> {
        self.callback_count += 1;

        if self.reject {
            return Err(anyhow::anyhow!("Adapter rejected block"));
        }

        self.received_blocks.push(block);
        Ok(())
    }
}

fn test_signing_key(seed: u8) -> Vec<u8> {
    let mut key = vec![0u8; 32];
    key[0] = seed;
    for (i, item) in key.iter_mut().enumerate().skip(1) {
        *item = ((seed as u16).wrapping_mul(i as u16 + 1)) as u8;
    }
    key
}

fn test_public_key(signing_key: &[u8]) -> Vec<u8> {
    use k256::ecdsa::SigningKey as SecpSigningKey;

    let sk = SecpSigningKey::from_slice(signing_key)
        .expect("failed to create secp256k1 signing key from seed");
    sk.verifying_key()
        .to_encoded_point(true)
        .as_bytes()
        .to_vec()
}

fn build_test_block_message(
    creator: &[u8],
    parents: &[(Vec<u8>, Vec<u8>)],
    signing_key: &[u8],
    sig_algorithm: &str,
) -> BlockMessage {
    build_test_block_message_with_state(creator, parents, signing_key, sig_algorithm, 0, 1)
}

fn build_test_block_message_with_state(
    creator: &[u8],
    parents: &[(Vec<u8>, Vec<u8>)],
    signing_key: &[u8],
    sig_algorithm: &str,
    block_number: u64,
    state_tag: u8,
) -> BlockMessage {
    let justifications: Vec<Justification> = parents
        .iter()
        .filter(|(hash, _)| hash.len() == 32)
        .map(|(hash, validator)| Justification {
            validator: validator.clone(),
            latest_block_hash: hash.clone(),
        })
        .collect();

    let payload = CordialBlockPayload {
        state: BlockState {
            pre_state_hash: vec![state_tag; 32],
            post_state_hash: vec![state_tag.wrapping_add(1); 32],
            bonds: vec![],
            block_number,
        },
        deploys: vec![],
        rejected_deploys: vec![],
        system_deploys: vec![],
    };
    let payload_bytes = payload.to_bytes();

    let mut predecessors = HashSet::new();
    for jus in &justifications {
        let mut hash_array = [0u8; 32];
        hash_array.copy_from_slice(&jus.latest_block_hash);
        predecessors.insert(BlockIdentity {
            content_hash: hash_array,
            creator: NodeId(jus.validator.clone()),
            signature: vec![],
        });
    }

    let content = BlockContent {
        payload: payload_bytes,
        predecessors,
    };

    let content_hash = hash_content(&content);
    let signature = sign(&content_hash, signing_key);

    BlockMessage {
        block_hash: content_hash.to_vec(),
        header: Header {
            parents_hash_list: parents.iter().map(|(hash, _)| hash.clone()).collect(),
            timestamp: 0,
            version: 1,
            extra_bytes: vec![],
        },
        body: Body {
            state: F1r3flyState {
                pre_state_hash: vec![state_tag; 32],
                post_state_hash: vec![state_tag.wrapping_add(1); 32],
                bonds: vec![],
                block_number: i64::try_from(block_number).expect("test block number should fit"),
            },
            deploys: vec![],
            rejected_deploys: vec![],
            system_deploys: vec![],
            extra_bytes: vec![],
        },
        justifications,
        sender: creator.to_vec(),
        seq_num: 0,
        sig: signature,
        sig_algorithm: sig_algorithm.to_string(),
        shard_id: "0".to_string(),
        extra_bytes: vec![],
    }
}
