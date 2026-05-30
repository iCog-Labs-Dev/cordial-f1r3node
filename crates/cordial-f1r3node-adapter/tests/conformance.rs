//! End-to-end Cordial Miners conformance scenarios through the adapter.
//!
//! These scenarios model the paper-level outcomes that must remain true when
//! Cordial Miners is driven through the f1r3node adapter boundary: honest
//! supermajorities finalize leaders, equivocations do not super-ratify, and
//! extending the DAG preserves the already emitted tau prefix.

use std::collections::{HashMap, HashSet};

use cordial_f1r3node_adapter::casper_adapter::{
    CordialCasper, CordialCasperAdapter, CordialMultiParentCasper,
};
use cordial_f1r3node_adapter::shard_conf::CasperShardConf;
use cordial_f1r3node_adapter::slashing::{F1r3SlashDeployFormatter, SlashDeployFormatter};
use cordial_miners_core::Block;
use cordial_miners_core::consensus::{
    CordialEvidencePool, EvidencePool, all_equivocations, weighted_super_ratifies,
};
use cordial_miners_core::crypto::CryptoVerifier;
use cordial_miners_core::execution::{BlockState, Bond, CordialBlockPayload, DeployPoolConfig};
use cordial_miners_core::types::{BlockContent, BlockIdentity, NodeId};

struct MockVerifier;

impl CryptoVerifier for MockVerifier {
    type Error = String;

    fn verify_block(
        &self,
        _content: &BlockContent,
        _sig: &[u8],
        _creator: &NodeId,
    ) -> Result<(), Self::Error> {
        Ok(())
    }
}

#[derive(Clone)]
struct ConformanceScenario {
    name: &'static str,
    bonds: HashMap<NodeId, u64>,
    network_blocks: Vec<Block>,
    expected_finalized_leader: Option<BlockIdentity>,
    expected_tau_prefix: Vec<BlockIdentity>,
}

struct HonestGraph {
    scenario: ConformanceScenario,
    extension_blocks: Vec<Block>,
}

fn node(id: u8) -> NodeId {
    NodeId(vec![id])
}

fn bonds(entries: &[(u8, u64)]) -> HashMap<NodeId, u64> {
    entries
        .iter()
        .map(|(id, stake)| (node(*id), *stake))
        .collect()
}

fn default_shard_conf() -> CasperShardConf {
    CasperShardConf::from_cordial(&DeployPoolConfig::default(), "root")
}

fn block(
    creator: u8,
    tag: u8,
    block_number: u64,
    predecessors: &[&Block],
    bonds: &HashMap<NodeId, u64>,
) -> Block {
    let predecessors = predecessors
        .iter()
        .map(|block| block.identity.clone())
        .collect::<HashSet<_>>();
    let content = BlockContent {
        payload: payload(block_number, bonds).to_bytes(),
        predecessors,
    };
    Block {
        identity: BlockIdentity {
            content_hash: tagged_hash(tag),
            creator: node(creator),
            signature: vec![tag; 64],
        },
        content,
    }
}

fn tagged_hash(tag: u8) -> [u8; 32] {
    let mut hash = [0u8; 32];
    hash[0] = tag;
    hash[31] = tag.wrapping_add(1);
    hash
}

fn payload(block_number: u64, bonds: &HashMap<NodeId, u64>) -> CordialBlockPayload {
    CordialBlockPayload {
        state: BlockState {
            pre_state_hash: vec![block_number.saturating_sub(1) as u8; 32],
            post_state_hash: vec![block_number as u8; 32],
            bonds: payload_bonds(bonds),
            block_number,
        },
        deploys: vec![],
        rejected_deploys: vec![],
        system_deploys: vec![],
    }
}

fn payload_bonds(bonds: &HashMap<NodeId, u64>) -> Vec<Bond> {
    let mut entries = bonds.iter().collect::<Vec<_>>();
    entries.sort_by_key(|(validator, _)| *validator);
    entries
        .into_iter()
        .map(|(validator, stake)| Bond {
            validator: validator.clone(),
            stake: *stake,
        })
        .collect()
}

fn adapter_for(bonds: &HashMap<NodeId, u64>) -> CordialCasperAdapter<MockVerifier> {
    CordialCasperAdapter::new_with_verifier(
        bonds.clone(),
        default_shard_conf(),
        "root",
        DeployPoolConfig::default(),
        None,
        MockVerifier,
    )
}

async fn feed_network_block(adapter: &CordialCasperAdapter<MockVerifier>, block: Block) {
    let hash = block.identity.content_hash.to_vec();
    {
        let mut blocklace = adapter.blocklace().lock().await;
        blocklace.insert(block, &adapter.verifier).unwrap();
    }
    assert!(
        adapter.dag_contains(&hash),
        "fed block must be visible through the adapter DAG"
    );
}

async fn feed_network_blocks(adapter: &CordialCasperAdapter<MockVerifier>, blocks: &[Block]) {
    for block in blocks {
        feed_network_block(adapter, block.clone()).await;
    }
}

async fn run_scenario(scenario: &ConformanceScenario) -> CordialCasperAdapter<MockVerifier> {
    let adapter = adapter_for(&scenario.bonds);
    feed_network_blocks(&adapter, &scenario.network_blocks).await;

    let snapshot = adapter.get_snapshot().await.unwrap();
    let expected_lfb = scenario
        .expected_finalized_leader
        .as_ref()
        .map(|id| id.content_hash.to_vec())
        .unwrap_or_default();
    let expected_tau = scenario
        .expected_tau_prefix
        .iter()
        .map(|id| id.content_hash.to_vec())
        .collect::<Vec<_>>();

    assert_eq!(
        snapshot.dag.last_finalized_block_hash, expected_lfb,
        "{}: adapter LFB must match expected final leader",
        scenario.name
    );
    assert_eq!(
        snapshot.ordered_finalized_blocks, expected_tau,
        "{}: adapter tau prefix must match expected output",
        scenario.name
    );

    if let Some(expected_leader) = &scenario.expected_finalized_leader {
        let lfb = adapter.last_finalized_block().await.unwrap();
        assert_eq!(lfb.block_hash, expected_leader.content_hash.to_vec());
    } else {
        assert!(adapter.last_finalized_block().await.is_err());
    }

    adapter
}

fn honest_majority_graph() -> HonestGraph {
    let bonds = bonds(&[(1, 1), (2, 1), (3, 1), (4, 1)]);

    let leader = block(1, 0x01, 0, &[], &bonds);

    let r1_v2 = block(2, 0x12, 1, &[&leader], &bonds);
    let r1_v3 = block(3, 0x13, 1, &[&leader], &bonds);
    let r1_v4 = block(4, 0x14, 1, &[&leader], &bonds);

    let r2_v2 = block(2, 0x22, 2, &[&r1_v2, &r1_v3, &r1_v4], &bonds);
    let r2_v3 = block(3, 0x23, 2, &[&r1_v2, &r1_v3, &r1_v4], &bonds);
    let r2_v4 = block(4, 0x24, 2, &[&r1_v2, &r1_v3, &r1_v4], &bonds);

    let wave1_leader = block(2, 0x31, 3, &[&r2_v2, &r2_v3, &r2_v4], &bonds);
    let w1_r1_v1 = block(1, 0x41, 4, &[&wave1_leader], &bonds);
    let w1_r1_v3 = block(3, 0x43, 4, &[&wave1_leader], &bonds);
    let w1_r1_v4 = block(4, 0x44, 4, &[&wave1_leader], &bonds);
    let w1_r2_v1 = block(1, 0x51, 5, &[&w1_r1_v1, &w1_r1_v3, &w1_r1_v4], &bonds);
    let w1_r2_v3 = block(3, 0x53, 5, &[&w1_r1_v1, &w1_r1_v3, &w1_r1_v4], &bonds);
    let w1_r2_v4 = block(4, 0x54, 5, &[&w1_r1_v1, &w1_r1_v3, &w1_r1_v4], &bonds);

    HonestGraph {
        scenario: ConformanceScenario {
            name: "honest majority finalizes wave-0 leader",
            bonds,
            network_blocks: vec![
                leader.clone(),
                r1_v2.clone(),
                r1_v3.clone(),
                r1_v4.clone(),
                r2_v2.clone(),
                r2_v3.clone(),
                r2_v4.clone(),
            ],
            expected_finalized_leader: Some(leader.identity.clone()),
            expected_tau_prefix: vec![leader.identity],
        },
        extension_blocks: vec![
            wave1_leader,
            w1_r1_v1,
            w1_r1_v3,
            w1_r1_v4,
            w1_r2_v1,
            w1_r2_v3,
            w1_r2_v4,
        ],
    }
}

fn equivocation_attack_scenario() -> ConformanceScenario {
    let bonds = bonds(&[(1, 1), (2, 1), (3, 1), (4, 1)]);

    let left = block(1, 0x01, 0, &[], &bonds);
    let right = block(1, 0x02, 0, &[], &bonds);
    let witness2 = block(2, 0x12, 1, &[&left, &right], &bonds);
    let witness3 = block(3, 0x13, 1, &[&left, &right], &bonds);
    let witness4 = block(4, 0x14, 1, &[&left, &right], &bonds);

    ConformanceScenario {
        name: "equivocation attack does not finalize",
        bonds,
        network_blocks: vec![left, right, witness2, witness3, witness4],
        expected_finalized_leader: None,
        expected_tau_prefix: vec![],
    }
}

fn collect_evidence(blocks: &[Block]) -> CordialEvidencePool {
    let verifier = MockVerifier;
    let mut blocklace = cordial_miners_core::Blocklace::new();
    for block in blocks {
        blocklace.insert(block.clone(), &verifier).unwrap();
    }

    let mut evidence_pool = CordialEvidencePool::new();
    for equivocation in all_equivocations(&blocklace) {
        let blocks = equivocation
            .blocks
            .iter()
            .filter_map(|id| blocklace.get(id))
            .collect::<Vec<_>>();
        evidence_pool.record_equivocation(equivocation.creator, equivocation.round, blocks);
    }
    evidence_pool
}

#[tokio::test]
async fn honest_majority_finalizes_expected_leader() {
    let graph = honest_majority_graph();

    run_scenario(&graph.scenario).await;
}

#[tokio::test]
async fn equivocation_attack_is_rejected_and_yields_slash_evidence() {
    let scenario = equivocation_attack_scenario();
    let adapter = run_scenario(&scenario).await;
    let blocklace = adapter.blocklace().lock().await;
    let left = &scenario.network_blocks[0];
    let right = &scenario.network_blocks[1];
    let witnesses = scenario.network_blocks[2..]
        .iter()
        .cloned()
        .collect::<HashSet<_>>();

    assert!(!weighted_super_ratifies(
        &blocklace,
        &witnesses,
        left,
        &scenario.bonds
    ));
    assert!(!weighted_super_ratifies(
        &blocklace,
        &witnesses,
        right,
        &scenario.bonds
    ));

    drop(blocklace);

    let evidence_pool = collect_evidence(&scenario.network_blocks);
    let evidence = evidence_pool.evidence_for(&node(1));
    assert_eq!(evidence.len(), 1);
    assert_eq!(evidence[0].round, 0);
    assert_eq!(evidence[0].blocks.len(), 2);

    let formatter = F1r3SlashDeployFormatter::new(node(2).0);
    let slash_deploys = formatter.to_slash_system_deploys(&evidence).unwrap();
    assert_eq!(slash_deploys.len(), 1);
    assert!(!slash_deploys[0].is_empty());
}

#[tokio::test]
async fn extending_dag_preserves_existing_tau_prefix() {
    let graph = honest_majority_graph();
    let adapter = run_scenario(&graph.scenario).await;
    let before = adapter.ordered_finalized_blocks().await.unwrap();

    feed_network_blocks(&adapter, &graph.extension_blocks).await;

    let after = adapter.ordered_finalized_blocks().await.unwrap();
    assert!(after.starts_with(&before));
    assert!(after.len() > before.len());
    assert_eq!(
        &after[..before.len()],
        before.as_slice(),
        "the previously emitted tau prefix must remain byte-for-byte invariant"
    );
}
