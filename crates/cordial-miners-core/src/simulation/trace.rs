use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::path::Path;

use serde::Serialize;

use crate::consensus::ValidationConfig;
use crate::simulation::dissemination::{DeliveryOutcome, SimNetwork, SimNode};
use crate::{Block, BlockContent, BlockIdentity, NodeId};

#[derive(Debug, Clone, Serialize)]
pub struct TraceBlock {
    pub id: String,
    pub label: String,
    pub creator: String,
    pub round: u64,
    pub role: String,
    pub deps: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TraceNodeState {
    pub known: Vec<String>,
    pub buffered: Vec<String>,
    pub partitioned: bool,
    pub final_leader: Option<String>,
    pub tau: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TraceSnapshot {
    pub step: usize,
    pub latest_message: String,
    pub active_block: Option<String>,
    pub nodes: BTreeMap<String, TraceNodeState>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TraceEvent {
    pub kind: String,
    pub title: String,
    pub detail: String,
    pub node: Option<String>,
    pub block: Option<String>,
    pub outcome: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TraceDocument {
    pub scenario: String,
    pub wavelength: u64,
    pub n: usize,
    pub f: usize,
    pub blocks: Vec<TraceBlock>,
    pub events: Vec<TraceEvent>,
    pub snapshots: Vec<TraceSnapshot>,
}

fn node(id: u8) -> NodeId {
    NodeId(vec![id])
}

fn node_key(id: &NodeId) -> String {
    match id.0.first() {
        Some(70) => "A".to_string(),
        Some(71) => "B".to_string(),
        Some(72) => "C".to_string(),
        Some(73) => "D".to_string(),
        Some(other) => format!("N{other}"),
        None => "N?".to_string(),
    }
}

fn label_for_tag(tag: u8) -> String {
    match tag {
        1 => "L1".to_string(),
        2 => "W2".to_string(),
        3 => "W3".to_string(),
        4 => "W4".to_string(),
        5 => "S2".to_string(),
        6 => "S3".to_string(),
        7 => "S4".to_string(),
        other => format!("B{other}"),
    }
}

fn round_for_tag(tag: u8) -> u64 {
    match tag {
        1 => 0,
        2..=4 => 1,
        5..=7 => 2,
        _ => 0,
    }
}

fn role_for_tag(tag: u8) -> &'static str {
    match tag {
        1 => "leader",
        2..=4 => "witness",
        5..=7 => "super",
        _ => "block",
    }
}

fn create_block(creator_id: u8, tag: u8, predecessors: HashSet<BlockIdentity>) -> Block {
    let mut content_hash = [0u8; 32];
    content_hash[0] = creator_id;
    content_hash[1] = tag;

    Block {
        identity: BlockIdentity {
            content_hash,
            creator: node(creator_id),
            signature: vec![tag],
        },
        content: BlockContent {
            payload: vec![tag],
            predecessors,
        },
    }
}

fn simulation_validation_config() -> ValidationConfig {
    ValidationConfig {
        check_content_hash: false,
        check_signature: false,
        ..ValidationConfig::default()
    }
}

fn leader_node1(_wave: u64) -> Option<NodeId> {
    Some(node(1))
}

fn all_bonds() -> HashMap<NodeId, u64> {
    let mut bonds = HashMap::new();
    bonds.insert(node(1), 100);
    bonds.insert(node(2), 100);
    bonds.insert(node(3), 100);
    bonds.insert(node(4), 100);
    bonds
}

fn known_labels(node: &SimNode) -> Vec<String> {
    let mut labels: Vec<_> = node
        .blocklace
        .dom()
        .into_iter()
        .map(|identity| label_for_tag(identity.signature[0]))
        .collect();
    labels.sort();
    labels
}

fn buffered_labels(node: &SimNode) -> Vec<String> {
    let mut labels: Vec<_> = node
        .pending
        .buffered_blocks
        .iter()
        .map(|(_, block)| label_for_tag(block.identity.signature[0]))
        .collect();
    labels.sort();
    labels
}

fn final_leader_label(node: &SimNode, wavelength: u64, n: usize, f: usize) -> Option<String> {
    node.latest_final_leader(wavelength, n, f, leader_node1)
        .map(|identity| label_for_tag(identity.signature[0]))
}

fn tau_labels(node: &SimNode, wavelength: u64, n: usize, f: usize) -> Vec<String> {
    match node.ordered_output(wavelength, n, f, leader_node1) {
        Ok(ids) => ids
            .into_iter()
            .map(|identity| label_for_tag(identity.signature[0]))
            .collect(),
        Err(_) => Vec::new(),
    }
}

fn snapshot_for_network(
    network: &SimNetwork,
    partitioned: &HashSet<String>,
    step: usize,
    latest_message: String,
    active_block: Option<String>,
    wavelength: u64,
    n: usize,
    f: usize,
) -> TraceSnapshot {
    let mut nodes = BTreeMap::new();

    for (id, node) in &network.nodes {
        let key = node_key(id);
        nodes.insert(
            key.clone(),
            TraceNodeState {
                known: known_labels(node),
                buffered: buffered_labels(node),
                partitioned: partitioned.contains(&key),
                final_leader: final_leader_label(node, wavelength, n, f),
                tau: tau_labels(node, wavelength, n, f),
            },
        );
    }

    TraceSnapshot {
        step,
        latest_message,
        active_block,
        nodes,
    }
}

fn deliver_all_pending_for(network: &mut SimNetwork, recipient: &NodeId) -> Vec<DeliveryOutcome> {
    let mut outcomes = Vec::new();
    while let Some(outcome) = network.deliver_next_to(recipient) {
        outcomes.push(outcome);
    }
    outcomes
}

pub fn four_node_convergence_trace() -> TraceDocument {
    let bonds = all_bonds();
    let observer_a = SimNode::new(node(70), bonds.clone(), simulation_validation_config());
    let observer_b = SimNode::new(node(71), bonds.clone(), simulation_validation_config());
    let observer_c = SimNode::new(node(72), bonds.clone(), simulation_validation_config());
    let observer_d = SimNode::new(node(73), bonds, simulation_validation_config());
    let mut network = SimNetwork::new(vec![observer_a, observer_b, observer_c, observer_d]);

    let wavelength = 3u64;
    let n = 4usize;
    let f = 1usize;

    let leader = create_block(1, 1, HashSet::new());
    let r1_v2 = create_block(2, 2, HashSet::from([leader.identity.clone()]));
    let r1_v3 = create_block(3, 3, HashSet::from([leader.identity.clone()]));
    let r1_v4 = create_block(4, 4, HashSet::from([leader.identity.clone()]));

    let round1_preds = HashSet::from([
        r1_v2.identity.clone(),
        r1_v3.identity.clone(),
        r1_v4.identity.clone(),
    ]);
    let r2_v2 = create_block(2, 5, round1_preds.clone());
    let r2_v3 = create_block(3, 6, round1_preds.clone());
    let r2_v4 = create_block(4, 7, round1_preds);

    let ordered_blocks = [&leader, &r1_v2, &r1_v3, &r1_v4, &r2_v2, &r2_v3, &r2_v4];
    let trace_blocks: Vec<TraceBlock> = ordered_blocks
        .iter()
        .map(|block| TraceBlock {
            id: label_for_tag(block.identity.signature[0]),
            label: label_for_tag(block.identity.signature[0]),
            creator: format!("V{}", block.identity.creator.0[0]),
            round: round_for_tag(block.identity.signature[0]),
            role: role_for_tag(block.identity.signature[0]).to_string(),
            deps: block
                .content
                .predecessors
                .iter()
                .map(|identity| label_for_tag(identity.signature[0]))
                .collect(),
        })
        .collect();

    for block in ordered_blocks {
        network.queue_delivery(node(70), block.clone());
    }
    for block in [&r2_v3, &r1_v2, &r2_v2, &leader, &r1_v4, &r2_v4, &r1_v3] {
        network.queue_delivery(node(71), block.clone());
    }
    for block in [&leader, &r1_v2] {
        network.queue_delivery(node(72), block.clone());
    }
    for block in [&leader, &r1_v4, &r2_v4, &r1_v3, &r2_v3, &r1_v2, &r2_v2] {
        network.queue_delivery(node(73), block.clone());
    }

    let mut partitioned = HashSet::new();
    let mut events = Vec::new();
    let mut snapshots = Vec::new();

    snapshots.push(snapshot_for_network(
        &network,
        &partitioned,
        0,
        "Replay loaded. Observer C will start in a partitioned state.".to_string(),
        None,
        wavelength,
        n,
        f,
    ));

    partitioned.insert("C".to_string());
    events.push(TraceEvent {
        kind: "partition".to_string(),
        title: "Observer C partitioned".to_string(),
        detail: "Observer C is temporarily cut off and can only see a partial view of the wave.".to_string(),
        node: Some("C".to_string()),
        block: None,
        outcome: None,
    });
    snapshots.push(snapshot_for_network(
        &network,
        &partitioned,
        1,
        "Observer C is partitioned and will lag behind the rest.".to_string(),
        None,
        wavelength,
        n,
        f,
    ));

    let delivery_plan: Vec<(&str, Vec<&Block>)> = vec![
        ("A", vec![&leader, &r1_v2, &r1_v3, &r1_v4, &r2_v2, &r2_v3, &r2_v4]),
        ("B", vec![&r2_v3, &r1_v2, &r2_v2, &leader, &r1_v4, &r2_v4, &r1_v3]),
        ("C", vec![&leader, &r1_v2]),
        ("D", vec![&leader, &r1_v4, &r2_v4, &r1_v3, &r2_v3, &r1_v2, &r2_v2]),
    ];

    let recipient_ids: HashMap<&str, NodeId> = HashMap::from([
        ("A", node(70)),
        ("B", node(71)),
        ("C", node(72)),
        ("D", node(73)),
    ]);

    for (node_name, blocks_for_node) in delivery_plan {
        let recipient = recipient_ids
            .get(node_name)
            .expect("recipient should exist")
            .clone();
        let outcomes = deliver_all_pending_for(&mut network, &recipient);
        for (idx, outcome) in outcomes.into_iter().enumerate() {
            let block = blocks_for_node[idx];
            let block_label = label_for_tag(block.identity.signature[0]);
            let (detail, outcome_text) = match outcome {
                DeliveryOutcome::Inserted => (
                    format!("{} accepted {} into its local blocklace.", node_name, block_label),
                    "inserted".to_string(),
                ),
                DeliveryOutcome::Buffered => (
                    format!(
                        "{} buffered {} because one or more predecessors were still missing.",
                        node_name, block_label
                    ),
                    "buffered".to_string(),
                ),
                DeliveryOutcome::Rejected(errors) => (
                    format!(
                        "{} rejected {} with {} validation errors.",
                        node_name,
                        block_label,
                        errors.len()
                    ),
                    "rejected".to_string(),
                ),
            };

            events.push(TraceEvent {
                kind: "deliver".to_string(),
                title: format!("Observer {} receives {}", node_name, block_label),
                detail: detail.clone(),
                node: Some(node_name.to_string()),
                block: Some(block_label.clone()),
                outcome: Some(outcome_text),
            });

            snapshots.push(snapshot_for_network(
                &network,
                &partitioned,
                snapshots.len(),
                detail,
                Some(block_label),
                wavelength,
                n,
                f,
            ));
        }
    }

    network.retry_all_buffers();
    snapshots.push(snapshot_for_network(
        &network,
        &partitioned,
        snapshots.len(),
        "Buffered deliveries were retried after the first delivery pass.".to_string(),
        None,
        wavelength,
        n,
        f,
    ));

    partitioned.remove("C");
    events.push(TraceEvent {
        kind: "heal".to_string(),
        title: "Observer C heals".to_string(),
        detail: "Observer C reconnects and can now receive the missing witness blocks.".to_string(),
        node: Some("C".to_string()),
        block: None,
        outcome: None,
    });
    snapshots.push(snapshot_for_network(
        &network,
        &partitioned,
        snapshots.len(),
        "Observer C healed and can now catch up on the missing wave evidence.".to_string(),
        None,
        wavelength,
        n,
        f,
    ));

    for block in [&r2_v4, &r1_v4, &r2_v2, &r2_v3, &r1_v3] {
        network.queue_delivery(node(72), block.clone());
    }
    let heal_outcomes = deliver_all_pending_for(&mut network, &node(72));
    let heal_blocks = [&r2_v4, &r1_v4, &r2_v2, &r2_v3, &r1_v3];

    for (idx, outcome) in heal_outcomes.into_iter().enumerate() {
        let block = heal_blocks[idx];
        let block_label = label_for_tag(block.identity.signature[0]);
        let (detail, outcome_text) = match outcome {
            DeliveryOutcome::Inserted => (
                format!("C accepted {} after the partition healed.", block_label),
                "inserted".to_string(),
            ),
            DeliveryOutcome::Buffered => (
                format!(
                    "C buffered {} after heal because some predecessors were still missing.",
                    block_label
                ),
                "buffered".to_string(),
            ),
            DeliveryOutcome::Rejected(errors) => (
                format!("C rejected {} with {} validation errors.", block_label, errors.len()),
                "rejected".to_string(),
            ),
        };

        events.push(TraceEvent {
            kind: "deliver".to_string(),
            title: format!("Observer C receives {}", block_label),
            detail: detail.clone(),
            node: Some("C".to_string()),
            block: Some(block_label.clone()),
            outcome: Some(outcome_text),
        });

        snapshots.push(snapshot_for_network(
            &network,
            &partitioned,
            snapshots.len(),
            detail,
            Some(block_label),
            wavelength,
            n,
            f,
        ));
    }

    network.retry_all_buffers();
    snapshots.push(snapshot_for_network(
        &network,
        &partitioned,
        snapshots.len(),
        "All buffers were retried again. The observers now share the same final leader and tau output."
            .to_string(),
        None,
        wavelength,
        n,
        f,
    ));

    TraceDocument {
        scenario: "four-node convergence".to_string(),
        wavelength,
        n,
        f,
        blocks: trace_blocks,
        events,
        snapshots,
    }
}

pub fn write_four_node_trace_js(path: impl AsRef<Path>) -> std::io::Result<()> {
    let trace = four_node_convergence_trace();
    let json = serde_json::to_string_pretty(&trace)
        .expect("four-node convergence trace should serialize to JSON");
    let path = path.as_ref();

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let script = format!("window.__FOUR_NODE_TRACE__ = {};\n", json);
    fs::write(path, script)
}
