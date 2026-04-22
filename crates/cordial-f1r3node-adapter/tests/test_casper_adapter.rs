//! Tests for the Casper trait adapter (Phase 3.1 / 3.2).
//!
//! Exercises the [`CordialCasper`] / [`CordialMultiParentCasper`] trait
//! surface through [`CordialCasperAdapter`]. No f1r3node dependency — we
//! use the mirror types from [`crate::block_translation`] and the adapter's
//! own local traits.

use std::collections::{HashMap, HashSet};

use cordial_miners_core::Block;
use cordial_miners_core::crypto::hash_content;
use cordial_miners_core::execution::{
    BlockState, Bond as CmBond, CordialBlockPayload, DeployPoolConfig,
};
use cordial_miners_core::types::{BlockContent, BlockIdentity, NodeId};
use either::Either;

use cordial_f1r3node_adapter::block_translation::{DeployData, SignedDeployData, block_to_message};
use cordial_f1r3node_adapter::casper_adapter::{
    BlockError, CordialCasper, CordialCasperAdapter, CordialMultiParentCasper, DeployError,
    InvalidBlock, ValidBlock,
};
use cordial_f1r3node_adapter::shard_conf::CasperShardConf;

// ── Helpers ──────────────────────────────────────────────────────────────

fn node(b: u8) -> NodeId {
    NodeId(vec![b])
}

fn bonds(entries: &[(u8, u64)]) -> HashMap<NodeId, u64> {
    entries.iter().map(|(b, s)| (node(*b), *s)).collect()
}

fn default_shard_conf() -> CasperShardConf {
    CasperShardConf::from_cordial(&DeployPoolConfig::default(), "root")
}

fn make_block(
    creator: NodeId,
    payload: CordialBlockPayload,
    predecessors: HashSet<BlockIdentity>,
    sig_tag: u8,
) -> Block {
    let content = BlockContent {
        payload: payload.to_bytes(),
        predecessors,
    };
    Block {
        identity: BlockIdentity {
            content_hash: hash_content(&content),
            creator,
            signature: vec![sig_tag; 64],
        },
        content,
    }
}

fn simple_payload(block_number: u64) -> CordialBlockPayload {
    CordialBlockPayload {
        state: BlockState {
            pre_state_hash: vec![0u8; 32],
            post_state_hash: vec![block_number as u8; 32],
            bonds: vec![CmBond {
                validator: node(1),
                stake: 100,
            }],
            block_number,
        },
        deploys: vec![],
        rejected_deploys: vec![],
        system_deploys: vec![],
    }
}

fn sample_deploy(sig_byte: u8) -> SignedDeployData {
    SignedDeployData {
        data: DeployData {
            term: format!("tx-{sig_byte}"),
            time_stamp: 1000 + sig_byte as i64,
            phlo_price: 1,
            phlo_limit: 10_000,
            valid_after_block_number: 0,
            shard_id: "root".to_string(),
            expiration_timestamp: None,
        },
        pk: vec![sig_byte; 32],
        sig: vec![sig_byte; 64],
        sig_algorithm: "ed25519".to_string(),
    }
}

async fn insert_through_adapter(adapter: &CordialCasperAdapter, block: Block) {
    let mut bl = adapter.blocklace().lock().await;
    bl.insert(block).unwrap();
}

// ── Construction ─────────────────────────────────────────────────────────

#[tokio::test]
async fn adapter_constructs_with_defaults() {
    let adapter = CordialCasperAdapter::new(
        bonds(&[(1, 100)]),
        default_shard_conf(),
        "root",
        DeployPoolConfig::default(),
        None,
    );
    assert_eq!(adapter.get_version(), 0); // default casper_version
    assert!(adapter.get_approved_block().is_err());
}

#[tokio::test]
async fn approved_block_is_returned() {
    let bl = cordial_miners_core::blocklace::Blocklace::new();
    let _ = bl; // just for namespacing
    let genesis = make_block(node(1), simple_payload(0), HashSet::new(), 1);
    let genesis_msg = block_to_message(&genesis, "root").unwrap();

    let adapter = CordialCasperAdapter::new(
        bonds(&[(1, 100)]),
        default_shard_conf(),
        "root",
        DeployPoolConfig::default(),
        Some(genesis_msg.clone()),
    );
    let stored = adapter.get_approved_block().unwrap();
    assert_eq!(stored.sender, vec![1]);
}

// ── contains / dag_contains / buffer_contains ────────────────────────────

#[tokio::test]
async fn dag_contains_tracks_inserted_blocks() {
    let adapter = CordialCasperAdapter::new(
        bonds(&[(1, 100)]),
        default_shard_conf(),
        "root",
        DeployPoolConfig::default(),
        None,
    );
    let g = make_block(node(1), simple_payload(0), HashSet::new(), 1);
    let hash = g.identity.content_hash.to_vec();
    assert!(!adapter.dag_contains(&hash));

    insert_through_adapter(&adapter, g).await;
    assert!(adapter.dag_contains(&hash));
    assert!(adapter.contains(&hash));
    assert!(!adapter.buffer_contains(&hash));
}

// ── deploy() ─────────────────────────────────────────────────────────────

#[tokio::test]
async fn deploy_accepts_valid_signed_deploy() {
    let adapter = CordialCasperAdapter::new(
        bonds(&[(1, 100)]),
        default_shard_conf(),
        "root",
        DeployPoolConfig::default(),
        None,
    );
    let deploy = sample_deploy(1);
    let expected_sig = deploy.sig.clone();
    let res = adapter.deploy(deploy).unwrap();
    match res {
        Either::Right(deploy_id) => assert_eq!(deploy_id, expected_sig),
        Either::Left(e) => panic!("expected acceptance, got {e:?}"),
    }
}

#[tokio::test]
async fn deploy_rejects_duplicate_signature() {
    let adapter = CordialCasperAdapter::new(
        bonds(&[(1, 100)]),
        default_shard_conf(),
        "root",
        DeployPoolConfig::default(),
        None,
    );
    let d = sample_deploy(1);
    adapter.deploy(d.clone()).unwrap();
    let res = adapter.deploy(d).unwrap();
    match res {
        Either::Left(DeployError::PoolRejected(msg)) => {
            assert!(msg.contains("duplicate"));
        }
        other => panic!("expected PoolRejected duplicate, got {other:?}"),
    }
}

#[tokio::test]
async fn deploy_rejects_empty_signature() {
    let adapter = CordialCasperAdapter::new(
        bonds(&[(1, 100)]),
        default_shard_conf(),
        "root",
        DeployPoolConfig::default(),
        None,
    );
    let mut d = sample_deploy(1);
    d.sig = vec![];
    let res = adapter.deploy(d).unwrap();
    assert!(matches!(
        res,
        Either::Left(DeployError::SignatureVerificationFailed)
    ));
}

#[tokio::test]
async fn has_pending_deploys_reflects_pool_state() {
    let adapter = CordialCasperAdapter::new(
        bonds(&[(1, 100)]),
        default_shard_conf(),
        "root",
        DeployPoolConfig::default(),
        None,
    );
    assert!(!adapter.has_pending_deploys_in_storage().await.unwrap());
    adapter.deploy(sample_deploy(1)).unwrap();
    assert!(adapter.has_pending_deploys_in_storage().await.unwrap());
}

// ── estimator ────────────────────────────────────────────────────────────

#[tokio::test]
async fn estimator_returns_empty_when_no_blocks() {
    let adapter = CordialCasperAdapter::new(
        bonds(&[(1, 100)]),
        default_shard_conf(),
        "root",
        DeployPoolConfig::default(),
        None,
    );
    let tips = adapter.estimator().await.unwrap();
    assert!(tips.is_empty());
}

#[tokio::test]
async fn estimator_returns_tip_of_single_chain() {
    let adapter = CordialCasperAdapter::new(
        bonds(&[(1, 100)]),
        default_shard_conf(),
        "root",
        DeployPoolConfig::default(),
        None,
    );
    let g = make_block(node(1), simple_payload(0), HashSet::new(), 1);
    let g_hash = g.identity.content_hash.to_vec();
    insert_through_adapter(&adapter, g).await;

    let tips = adapter.estimator().await.unwrap();
    assert_eq!(tips.len(), 1);
    assert_eq!(tips[0], g_hash);
}

// ── get_snapshot ─────────────────────────────────────────────────────────

#[tokio::test]
async fn get_snapshot_succeeds_on_empty_blocklace() {
    let adapter = CordialCasperAdapter::new(
        bonds(&[(1, 100)]),
        default_shard_conf(),
        "root",
        DeployPoolConfig::default(),
        None,
    );
    let snap = adapter.get_snapshot().await.unwrap();
    assert!(snap.dag.dag_set.is_empty());
    assert_eq!(snap.on_chain_state.bonds_map[&vec![1u8]], 100);
}

// ── validate ─────────────────────────────────────────────────────────────

#[tokio::test]
async fn validate_accepts_well_formed_genesis() {
    let adapter = CordialCasperAdapter::new(
        bonds(&[(1, 100)]),
        default_shard_conf(),
        "root",
        DeployPoolConfig::default(),
        None,
    );
    let g = make_block(node(1), simple_payload(0), HashSet::new(), 1);
    let msg = block_to_message(&g, "root").unwrap();

    // Skip crypto checks: our test signature is just [1;64], not a real ed25519 sig.
    let adapter =
        adapter.with_validation_config(cordial_miners_core::consensus::ValidationConfig {
            check_content_hash: false,
            check_signature: false,
            check_sender: true,
            check_closure: true,
            check_chain_axiom: true,
            check_cordial: false,
        });

    let res = adapter.validate(&msg).await.unwrap();
    assert!(matches!(res, Either::Right(ValidBlock::Valid)));
}

#[tokio::test]
async fn validate_missing_predecessor_returns_missing_blocks() {
    let adapter = CordialCasperAdapter::new(
        bonds(&[(1, 100)]),
        default_shard_conf(),
        "root",
        DeployPoolConfig::default(),
        None,
    )
    .with_validation_config(cordial_miners_core::consensus::ValidationConfig {
        check_content_hash: false,
        check_signature: false,
        check_sender: true,
        check_closure: true,
        check_chain_axiom: true,
        check_cordial: false,
    });

    // Build a child block whose parent is NOT in the adapter's blocklace.
    let parent = make_block(node(1), simple_payload(0), HashSet::new(), 1);
    let mut preds = HashSet::new();
    preds.insert(parent.identity.clone());
    let child = make_block(node(1), simple_payload(1), preds, 2);
    let msg = block_to_message(&child, "root").unwrap();

    let res = adapter.validate(&msg).await.unwrap();
    assert!(matches!(res, Either::Left(BlockError::MissingBlocks)));
}

#[tokio::test]
async fn validate_unbonded_sender_returns_invalid_sender() {
    let adapter = CordialCasperAdapter::new(
        bonds(&[(2, 100)]), // only node 2 is bonded
        default_shard_conf(),
        "root",
        DeployPoolConfig::default(),
        None,
    )
    .with_validation_config(cordial_miners_core::consensus::ValidationConfig {
        check_content_hash: false,
        check_signature: false,
        check_sender: true,
        check_closure: true,
        check_chain_axiom: true,
        check_cordial: false,
    });
    let g = make_block(node(1), simple_payload(0), HashSet::new(), 1);
    let msg = block_to_message(&g, "root").unwrap();
    let res = adapter.validate(&msg).await.unwrap();
    match res {
        Either::Left(BlockError::Invalid(InvalidBlock::InvalidSender)) => {}
        other => panic!("expected InvalidSender, got {other:?}"),
    }
}

// ── handle_valid_block / handle_invalid_block ────────────────────────────

#[tokio::test]
async fn handle_valid_block_inserts_into_blocklace() {
    let adapter = CordialCasperAdapter::new(
        bonds(&[(1, 100)]),
        default_shard_conf(),
        "root",
        DeployPoolConfig::default(),
        None,
    );
    let g = make_block(node(1), simple_payload(0), HashSet::new(), 1);
    let hash = g.identity.content_hash.to_vec();
    let msg = block_to_message(&g, "root").unwrap();

    adapter.handle_valid_block(&msg).await.unwrap();
    assert!(adapter.dag_contains(&hash));
}

#[tokio::test]
async fn handle_invalid_block_does_not_insert() {
    let adapter = CordialCasperAdapter::new(
        bonds(&[(1, 100)]),
        default_shard_conf(),
        "root",
        DeployPoolConfig::default(),
        None,
    );
    let g = make_block(node(1), simple_payload(0), HashSet::new(), 1);
    let hash = g.identity.content_hash.to_vec();
    let msg = block_to_message(&g, "root").unwrap();

    adapter
        .handle_invalid_block(&msg, &InvalidBlock::InvalidSignature)
        .unwrap();
    assert!(!adapter.dag_contains(&hash));
}

// ── buffer operations ────────────────────────────────────────────────────

#[tokio::test]
async fn buffer_starts_empty() {
    let adapter = CordialCasperAdapter::new(
        bonds(&[(1, 100)]),
        default_shard_conf(),
        "root",
        DeployPoolConfig::default(),
        None,
    );
    assert!(adapter.get_all_from_buffer().unwrap().is_empty());
    assert!(
        adapter
            .get_dependency_free_from_buffer()
            .unwrap()
            .is_empty()
    );
}

// ── last_finalized_block ─────────────────────────────────────────────────

#[tokio::test]
async fn last_finalized_returns_err_when_none_finalized() {
    let adapter = CordialCasperAdapter::new(
        bonds(&[(1, 100)]),
        default_shard_conf(),
        "root",
        DeployPoolConfig::default(),
        None,
    );
    assert!(adapter.last_finalized_block().await.is_err());
}

#[tokio::test]
async fn last_finalized_returns_genesis_once_supermajority_supports_it() {
    let adapter = CordialCasperAdapter::new(
        bonds(&[(1, 100)]),
        default_shard_conf(),
        "root",
        DeployPoolConfig::default(),
        None,
    );
    // Single validator with 100% stake → genesis is immediately finalized.
    let g = make_block(node(1), simple_payload(0), HashSet::new(), 1);
    let g_hash = g.identity.content_hash.to_vec();
    insert_through_adapter(&adapter, g).await;

    let lfb_msg = adapter.last_finalized_block().await.unwrap();
    // `lfb_msg.block_hash` is the blocklace content_hash (not the
    // f1r3node-style Blake2b block hash).
    assert_eq!(lfb_msg.block_hash, g_hash);
    assert_eq!(lfb_msg.sender, vec![1]);
}

// ── normalized_initial_fault ─────────────────────────────────────────────

#[tokio::test]
async fn normalized_initial_fault_zero_when_no_equivocators() {
    let adapter = CordialCasperAdapter::new(
        bonds(&[(1, 100), (2, 200)]),
        default_shard_conf(),
        "root",
        DeployPoolConfig::default(),
        None,
    );
    let weights: HashMap<Vec<u8>, u64> = [(vec![1u8], 100u64), (vec![2u8], 200u64)]
        .into_iter()
        .collect();
    let fault = adapter.normalized_initial_fault(weights).unwrap();
    assert_eq!(fault, 0.0);
}

#[tokio::test]
async fn normalized_initial_fault_counts_equivocator_stake() {
    let adapter = CordialCasperAdapter::new(
        bonds(&[(1, 100), (2, 200)]),
        default_shard_conf(),
        "root",
        DeployPoolConfig::default(),
        None,
    );
    // Make node 1 an equivocator by inserting two incomparable blocks.
    // They must have distinct content to get distinct BlockIdentities.
    let mut p1 = simple_payload(0);
    p1.state.pre_state_hash = vec![0xaa; 32];
    let mut p2 = simple_payload(0);
    p2.state.pre_state_hash = vec![0xbb; 32];
    let g1 = make_block(node(1), p1, HashSet::new(), 1);
    let g2 = make_block(node(1), p2, HashSet::new(), 2);
    insert_through_adapter(&adapter, g1).await;
    insert_through_adapter(&adapter, g2).await;

    let weights: HashMap<Vec<u8>, u64> = [(vec![1u8], 100u64), (vec![2u8], 200u64)]
        .into_iter()
        .collect();
    let fault = adapter.normalized_initial_fault(weights).unwrap();
    // Node 1 is the equivocator with 100/300 of the stake.
    assert!((fault - (100.0_f32 / 300.0)).abs() < 1e-5);
}

// ── get_version ──────────────────────────────────────────────────────────

#[tokio::test]
async fn get_version_reads_from_shard_conf() {
    let mut shard_conf = default_shard_conf();
    shard_conf.casper_version = 42;
    let adapter = CordialCasperAdapter::new(
        bonds(&[(1, 100)]),
        shard_conf,
        "root",
        DeployPoolConfig::default(),
        None,
    );
    assert_eq!(adapter.get_version(), 42);
}
