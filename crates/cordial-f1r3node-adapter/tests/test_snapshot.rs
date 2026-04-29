//! Tests for `snapshot` — constructing `CasperSnapshot` from a blocklace.

use std::collections::{HashMap, HashSet};

use cordial_miners_core::Block;
use cordial_miners_core::blocklace::Blocklace;
use cordial_miners_core::crypto::hash_content;
use cordial_miners_core::execution::{
    BlockState, Bond as CmBond, CordialBlockPayload, Deploy, ProcessedDeploy, SignedDeploy,
};
use cordial_miners_core::types::{BlockContent, BlockIdentity, NodeId};

use cordial_f1r3node_adapter::snapshot::{CasperShardConf, SnapshotError, build_snapshot};
use cordial_miners_core::crypto::CryptoVerifier;

struct MockVerifier;

impl CryptoVerifier for MockVerifier {
    type Error = String;
    fn verify_block(
        &self,
        _content: &BlockContent,
        _sig: &[u8],
        _creator: &NodeId,
    ) -> Result<(), Self::Error> {
        Ok(()) // Always allow in tests
    }
}

// ── Helpers ──────────────────────────────────────────────────────────────

fn node(b: u8) -> NodeId {
    NodeId(vec![b])
}

fn bonds(entries: &[(u8, u64)]) -> HashMap<NodeId, u64> {
    entries.iter().map(|(b, s)| (node(*b), *s)).collect()
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

fn simple_payload(block_number: u64, bonds: Vec<CmBond>) -> CordialBlockPayload {
    CordialBlockPayload {
        state: BlockState {
            pre_state_hash: vec![0x00; 32],
            post_state_hash: vec![block_number as u8; 32],
            bonds,
            block_number,
        },
        deploys: vec![],
        rejected_deploys: vec![],
        system_deploys: vec![],
    }
}

fn default_shard_conf() -> CasperShardConf {
    CasperShardConf {
        fault_tolerance_threshold: 0.1,
        shard_name: "root".to_string(),
        max_number_of_parents: 16,
        max_parent_depth: None,
        deploy_lifespan: 50,
        min_phlo_price: 1,
    }
}

// ── Empty blocklace ──────────────────────────────────────────────────────

#[test]
fn empty_blocklace_produces_empty_snapshot() {
    let bl = Blocklace::new();
    let b = bonds(&[(1, 100)]);
    let snap = build_snapshot(&bl, &b, default_shard_conf(), "root").unwrap();

    assert!(snap.dag.dag_set.is_empty());
    assert!(snap.dag.latest_messages_map.is_empty());
    assert!(snap.dag.child_map.is_empty());
    assert!(snap.dag.height_map.is_empty());
    assert!(snap.dag.last_finalized_block_hash.is_empty());
    assert!(snap.last_finalized_block.is_empty());
    assert!(snap.lca.is_empty());
    assert!(snap.tips.is_empty());
    assert!(snap.parents.is_empty());
    assert!(snap.justifications.is_empty());
    assert_eq!(snap.max_block_num, 0);
    assert!(snap.deploys_in_scope.is_empty());
}

// ── dag_set and block_number_map ─────────────────────────────────────────

#[test]
fn dag_set_contains_all_block_hashes() {
    let mut bl = Blocklace::new();
    let v1 = node(1);
    let g = make_block(v1.clone(), simple_payload(0, vec![]), HashSet::new(), 1);
    bl.insert(g.clone(), &MockVerifier).unwrap();

    let mut preds = HashSet::new();
    preds.insert(g.identity.clone());
    let b2 = make_block(v1.clone(), simple_payload(1, vec![]), preds, 2);
    bl.insert(b2.clone(), &MockVerifier).unwrap();

    let bonds_map = bonds(&[(1, 100)]);
    let snap = build_snapshot(&bl, &bonds_map, default_shard_conf(), "root").unwrap();

    assert_eq!(snap.dag.dag_set.len(), 2);
    assert!(
        snap.dag
            .dag_set
            .contains(g.identity.content_hash.as_slice())
    );
    assert!(
        snap.dag
            .dag_set
            .contains(b2.identity.content_hash.as_slice())
    );

    // block_number_map should reflect per-block block_number
    assert_eq!(
        snap.dag.block_number_map[&g.identity.content_hash.to_vec()],
        0
    );
    assert_eq!(
        snap.dag.block_number_map[&b2.identity.content_hash.to_vec()],
        1
    );
}

#[test]
fn height_map_groups_blocks_by_block_number() {
    let mut bl = Blocklace::new();
    let v1 = node(1);
    let v2 = node(2);

    // Two genesis blocks at block_number 0 from different validators.
    // We distinguish them via the pre_state_hash so their content hashes
    // differ (the blocklace is keyed by content_hash, not creator).
    let mut p1 = simple_payload(0, vec![]);
    p1.state.pre_state_hash = vec![0xaa; 32];
    let mut p2 = simple_payload(0, vec![]);
    p2.state.pre_state_hash = vec![0xbb; 32];

    let g1 = make_block(v1, p1, HashSet::new(), 1);
    let g2 = make_block(v2, p2, HashSet::new(), 2);
    bl.insert(g1.clone(), &MockVerifier).unwrap();
    bl.insert(g2.clone(), &MockVerifier).unwrap();

    let bonds_map = bonds(&[(1, 100), (2, 100)]);
    let snap = build_snapshot(&bl, &bonds_map, default_shard_conf(), "root").unwrap();

    let at_height_0 = snap.dag.height_map.get(&0).unwrap();
    assert_eq!(at_height_0.len(), 2);
}

// ── child_map (inverted predecessor relation) ────────────────────────────

#[test]
fn child_map_inverts_predecessor_relation() {
    let mut bl = Blocklace::new();
    let v1 = node(1);

    let g = make_block(v1.clone(), simple_payload(0, vec![]), HashSet::new(), 1);
    bl.insert(g.clone(), &MockVerifier).unwrap();

    let mut preds = HashSet::new();
    preds.insert(g.identity.clone());
    let b2 = make_block(v1.clone(), simple_payload(1, vec![]), preds, 2);
    bl.insert(b2.clone(), &MockVerifier).unwrap();

    let bonds_map = bonds(&[(1, 100)]);
    let snap = build_snapshot(&bl, &bonds_map, default_shard_conf(), "root").unwrap();

    // g should have b2 as a child
    let children_of_g = snap
        .dag
        .child_map
        .get(g.identity.content_hash.as_slice())
        .unwrap();
    assert!(children_of_g.contains(&b2.identity.content_hash.to_vec()));

    // b2 has no children
    assert!(
        !snap
            .dag
            .child_map
            .contains_key(b2.identity.content_hash.as_slice())
    );
}

// ── latest_messages_map ──────────────────────────────────────────────────

#[test]
fn latest_messages_map_tracks_each_validator_tip() {
    let mut bl = Blocklace::new();
    let v1 = node(1);
    let v2 = node(2);

    let g = make_block(v1.clone(), simple_payload(0, vec![]), HashSet::new(), 1);
    bl.insert(g.clone(), &MockVerifier).unwrap();

    // v1 extends
    let mut preds_a = HashSet::new();
    preds_a.insert(g.identity.clone());
    let b1a = make_block(v1.clone(), simple_payload(1, vec![]), preds_a, 2);
    bl.insert(b1a.clone(), &MockVerifier).unwrap();

    // v2 builds on g
    let mut preds_b = HashSet::new();
    preds_b.insert(g.identity.clone());
    let b2a = make_block(v2.clone(), simple_payload(1, vec![]), preds_b, 3);
    bl.insert(b2a.clone(), &MockVerifier).unwrap();

    let bonds_map = bonds(&[(1, 100), (2, 100)]);
    let snap = build_snapshot(&bl, &bonds_map, default_shard_conf(), "root").unwrap();

    assert_eq!(snap.dag.latest_messages_map.len(), 2);
    assert_eq!(
        snap.dag.latest_messages_map[&vec![1u8]],
        b1a.identity.content_hash.to_vec()
    );
    assert_eq!(
        snap.dag.latest_messages_map[&vec![2u8]],
        b2a.identity.content_hash.to_vec()
    );
}

#[test]
fn latest_messages_map_excludes_equivocators() {
    let mut bl = Blocklace::new();
    let v1 = node(1);

    // v1 creates TWO incomparable genesis blocks — equivocation
    let g1 = make_block(v1.clone(), simple_payload(0, vec![]), HashSet::new(), 1);
    let g2 = make_block(v1.clone(), simple_payload(0, vec![]), HashSet::new(), 2);
    bl.insert(g1, &MockVerifier).unwrap();
    bl.insert(g2, &MockVerifier).unwrap();

    let bonds_map = bonds(&[(1, 100)]);
    let snap = build_snapshot(&bl, &bonds_map, default_shard_conf(), "root").unwrap();

    // v1 is an equivocator → should not appear in latest_messages_map
    assert!(snap.dag.latest_messages_map.is_empty());
}

// ── Finality fields ──────────────────────────────────────────────────────

#[test]
fn last_finalized_block_matches_finality_detector() {
    let mut bl = Blocklace::new();
    let v1 = node(1);
    let v2 = node(2);

    // v1 genesis, v2 builds on it → g finalizes under (v1, v2) both supporting
    let g = make_block(v1.clone(), simple_payload(0, vec![]), HashSet::new(), 1);
    bl.insert(g.clone(), &MockVerifier).unwrap();
    let mut preds = HashSet::new();
    preds.insert(g.identity.clone());
    let b2 = make_block(v2.clone(), simple_payload(1, vec![]), preds, 2);
    bl.insert(b2.clone(), &MockVerifier).unwrap();

    let bonds_map = bonds(&[(1, 100), (2, 100)]);
    let snap = build_snapshot(&bl, &bonds_map, default_shard_conf(), "root").unwrap();

    // Last finalized block should be non-empty (some block is finalized)
    assert!(!snap.dag.last_finalized_block_hash.is_empty());
    assert_eq!(
        snap.last_finalized_block,
        snap.dag.last_finalized_block_hash
    );

    // The LFB must be a real block in the lace
    assert!(snap.dag.dag_set.contains(&snap.last_finalized_block));

    // finalized_blocks_set includes the LFB and its ancestors
    assert!(
        snap.dag
            .finalized_blocks_set
            .contains(&snap.last_finalized_block)
    );
}

#[test]
fn no_finality_when_single_validator_has_no_supermajority() {
    // With 3 independent genesis blocks from 3 validators, none of them are
    // in the ancestry of > 2/3 of the validators' tips, so no LFB.
    let mut bl = Blocklace::new();
    let g1 = make_block(node(1), simple_payload(0, vec![]), HashSet::new(), 1);
    let g2 = make_block(node(2), simple_payload(0, vec![]), HashSet::new(), 2);
    let g3 = make_block(node(3), simple_payload(0, vec![]), HashSet::new(), 3);
    bl.insert(g1, &MockVerifier).unwrap();
    bl.insert(g2, &MockVerifier).unwrap();
    bl.insert(g3, &MockVerifier).unwrap();

    let bonds_map = bonds(&[(1, 100), (2, 100), (3, 100)]);
    let snap = build_snapshot(&bl, &bonds_map, default_shard_conf(), "root").unwrap();

    assert!(snap.dag.last_finalized_block_hash.is_empty());
    assert!(snap.last_finalized_block.is_empty());
    assert!(snap.dag.finalized_blocks_set.is_empty());
}

// ── Tips / LCA / parents ─────────────────────────────────────────────────

#[test]
fn tips_and_parents_reflect_fork_choice() {
    let mut bl = Blocklace::new();
    let v1 = node(1);

    let g = make_block(v1.clone(), simple_payload(0, vec![]), HashSet::new(), 1);
    bl.insert(g.clone(), &MockVerifier).unwrap();

    let bonds_map = bonds(&[(1, 100)]);
    let snap = build_snapshot(&bl, &bonds_map, default_shard_conf(), "root").unwrap();

    // Single validator, single block → single tip = g
    assert_eq!(snap.tips.len(), 1);
    assert_eq!(snap.tips[0], g.identity.content_hash.to_vec());
    assert_eq!(snap.lca, g.identity.content_hash.to_vec());

    // Parents should contain one BlockMessage with the same hash
    assert_eq!(snap.parents.len(), 1);
    assert_eq!(snap.parents[0].block_hash, g.identity.content_hash.to_vec());
    assert_eq!(snap.parents[0].sender, vec![1]);
}

// ── Justifications ───────────────────────────────────────────────────────

#[test]
fn justifications_contain_one_entry_per_validator_tip() {
    let mut bl = Blocklace::new();
    let v1 = node(1);
    let v2 = node(2);

    let g = make_block(v1.clone(), simple_payload(0, vec![]), HashSet::new(), 1);
    bl.insert(g.clone(), &MockVerifier).unwrap();

    let mut preds = HashSet::new();
    preds.insert(g.identity.clone());
    let b2 = make_block(v2.clone(), simple_payload(1, vec![]), preds, 2);
    bl.insert(b2.clone(), &MockVerifier).unwrap();

    let bonds_map = bonds(&[(1, 100), (2, 100)]);
    let snap = build_snapshot(&bl, &bonds_map, default_shard_conf(), "root").unwrap();

    assert_eq!(snap.justifications.len(), 2);
    let validators: HashSet<Vec<u8>> = snap
        .justifications
        .iter()
        .map(|j| j.validator.clone())
        .collect();
    assert!(validators.contains(&vec![1u8]));
    assert!(validators.contains(&vec![2u8]));
}

// ── max_block_num and max_seq_nums ───────────────────────────────────────

#[test]
fn max_block_num_reflects_highest_block_number_in_lace() {
    let mut bl = Blocklace::new();
    let v1 = node(1);

    let g = make_block(v1.clone(), simple_payload(0, vec![]), HashSet::new(), 1);
    bl.insert(g.clone(), &MockVerifier).unwrap();

    let mut preds1 = HashSet::new();
    preds1.insert(g.identity.clone());
    let b1 = make_block(v1.clone(), simple_payload(1, vec![]), preds1, 2);
    bl.insert(b1.clone(), &MockVerifier).unwrap();

    let mut preds2 = HashSet::new();
    preds2.insert(b1.identity.clone());
    let b2 = make_block(v1.clone(), simple_payload(5, vec![]), preds2, 3);
    bl.insert(b2, &MockVerifier).unwrap();

    let bonds_map = bonds(&[(1, 100)]);
    let snap = build_snapshot(&bl, &bonds_map, default_shard_conf(), "root").unwrap();

    assert_eq!(snap.max_block_num, 5);
}

#[test]
fn max_seq_nums_counts_blocks_per_validator() {
    let mut bl = Blocklace::new();
    let v1 = node(1);
    let v2 = node(2);

    let g = make_block(v1.clone(), simple_payload(0, vec![]), HashSet::new(), 1);
    bl.insert(g.clone(), &MockVerifier).unwrap();

    let mut p1 = HashSet::new();
    p1.insert(g.identity.clone());
    let b1 = make_block(v1.clone(), simple_payload(1, vec![]), p1, 2);
    bl.insert(b1.clone(), &MockVerifier).unwrap();

    let mut p2 = HashSet::new();
    p2.insert(b1.identity.clone());
    let b2 = make_block(v2.clone(), simple_payload(2, vec![]), p2, 3);
    bl.insert(b2, &MockVerifier).unwrap();

    let bonds_map = bonds(&[(1, 100), (2, 100)]);
    let snap = build_snapshot(&bl, &bonds_map, default_shard_conf(), "root").unwrap();

    // v1 has 2 blocks, v2 has 1
    assert_eq!(snap.max_seq_nums[&vec![1u8]], 2);
    assert_eq!(snap.max_seq_nums[&vec![2u8]], 1);
}

// ── on_chain_state ───────────────────────────────────────────────────────

#[test]
fn on_chain_state_populates_bonds_and_active_validators() {
    let bl = Blocklace::new();
    let bonds_map = bonds(&[(1, 100), (2, 200)]);
    let snap = build_snapshot(&bl, &bonds_map, default_shard_conf(), "root").unwrap();

    assert_eq!(snap.on_chain_state.bonds_map.len(), 2);
    assert_eq!(snap.on_chain_state.bonds_map[&vec![1u8]], 100);
    assert_eq!(snap.on_chain_state.bonds_map[&vec![2u8]], 200);

    assert_eq!(snap.on_chain_state.active_validators.len(), 2);
    assert!(snap.on_chain_state.active_validators.contains(&vec![1u8]));
    assert!(snap.on_chain_state.active_validators.contains(&vec![2u8]));
}

#[test]
fn equivocator_excluded_from_active_validators() {
    let mut bl = Blocklace::new();
    let v1 = node(1);

    // v1 equivocates
    let g1 = make_block(v1.clone(), simple_payload(0, vec![]), HashSet::new(), 1);
    let g2 = make_block(v1.clone(), simple_payload(0, vec![]), HashSet::new(), 2);
    bl.insert(g1, &MockVerifier).unwrap();
    bl.insert(g2, &MockVerifier).unwrap();

    let bonds_map = bonds(&[(1, 100), (2, 100)]);
    let snap = build_snapshot(&bl, &bonds_map, default_shard_conf(), "root").unwrap();

    // v1 is still in bonds_map (bond exists on-chain)
    assert_eq!(snap.on_chain_state.bonds_map.len(), 2);
    // but only v2 is active
    assert_eq!(snap.on_chain_state.active_validators.len(), 1);
    assert_eq!(snap.on_chain_state.active_validators[0], vec![2u8]);
}

// ── deploys_in_scope ─────────────────────────────────────────────────────

#[test]
fn deploys_in_scope_collects_from_tip_ancestry() {
    let mut bl = Blocklace::new();
    let v1 = node(1);

    let deploy_sig = vec![0x42; 64];
    let signed = SignedDeploy {
        deploy: Deploy {
            term: b"tx".to_vec(),
            timestamp: 1000,
            phlo_price: 1,
            phlo_limit: 100,
            valid_after_block_number: 0,
            shard_id: "root".to_string(),
        },
        deployer: vec![0x01; 32],
        signature: deploy_sig.clone(),
    };
    let processed = ProcessedDeploy {
        deploy: signed,
        cost: 10,
        is_failed: false,
    };

    let mut genesis_payload = simple_payload(0, vec![]);
    genesis_payload.deploys = vec![processed];
    let g = make_block(v1.clone(), genesis_payload, HashSet::new(), 1);
    bl.insert(g.clone(), &MockVerifier).unwrap();

    let bonds_map = bonds(&[(1, 100)]);
    let snap = build_snapshot(&bl, &bonds_map, default_shard_conf(), "root").unwrap();

    assert!(snap.deploys_in_scope.contains(&deploy_sig));
}

// ── Error paths ──────────────────────────────────────────────────────────

#[test]
fn undecodable_payload_produces_error() {
    let mut bl = Blocklace::new();
    // Block with invalid (non-bincode) payload bytes
    let content = BlockContent {
        payload: vec![0xff, 0xff, 0xff], // not a valid CordialBlockPayload
        predecessors: HashSet::new(),
    };
    let block = Block {
        identity: BlockIdentity {
            content_hash: hash_content(&content),
            creator: node(1),
            signature: vec![0x00; 64],
        },
        content,
    };
    bl.insert(block, &MockVerifier).unwrap();

    let bonds_map = bonds(&[(1, 100)]);
    let err = build_snapshot(&bl, &bonds_map, default_shard_conf(), "root").unwrap_err();
    assert!(matches!(err, SnapshotError::PayloadDecodeFailed { .. }));
}
