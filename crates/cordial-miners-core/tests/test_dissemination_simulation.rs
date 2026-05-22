use cordial_miners_core::simulation::dissemination::{DeliveryOutcome, SimNetwork, SimNode};
use cordial_miners_core::{Block, BlockContent, BlockIdentity, NodeId};
use cordial_miners_core::consensus::ValidationConfig;
use std::collections::{HashMap, HashSet};

fn node(id: u8) -> NodeId {
    NodeId(vec![id])
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

#[test]
fn out_of_order_delivery_is_buffered_then_resolved_after_parent_arrives() {
    let mut bonds = HashMap::new();
    bonds.insert(node(1), 100);
    bonds.insert(node(2), 100);

    let mut sim_node = SimNode::new(node(9), bonds, simulation_validation_config());

    let genesis = create_block(1, 1, HashSet::new());
    let child = create_block(2, 2, HashSet::from([genesis.identity.clone()]));

    let early_delivery = sim_node.receive_block(child.clone());
    assert_eq!(early_delivery, DeliveryOutcome::Buffered);
    assert_eq!(sim_node.pending_len(), 1);
    assert!(!sim_node.knows_block(&child.identity));

    let parent_delivery = sim_node.receive_block(genesis.clone());
    assert_eq!(parent_delivery, DeliveryOutcome::Inserted);
    assert!(sim_node.knows_block(&genesis.identity));

    sim_node.retry_buffered_blocks();

    assert_eq!(sim_node.pending_len(), 0);
    assert!(sim_node.knows_block(&child.identity));
}

#[test]
fn two_nodes_converge_after_receiving_the_same_blocks_in_different_orders() {
    let mut bonds = HashMap::new();
    bonds.insert(node(1), 100);
    bonds.insert(node(2), 100);

    let node_a = SimNode::new(node(10), bonds.clone(), simulation_validation_config());
    let node_b = SimNode::new(node(11), bonds, simulation_validation_config());
    let mut network = SimNetwork::new(vec![node_a, node_b]);

    let genesis = create_block(1, 1, HashSet::new());
    let child = create_block(2, 2, HashSet::from([genesis.identity.clone()]));

    network.queue_delivery(node(10), genesis.clone());
    network.queue_delivery(node(10), child.clone());
    network.queue_delivery(node(11), child.clone());
    network.queue_delivery(node(11), genesis.clone());
    assert_eq!(network.queued_delivery_count(), 4);

    assert_eq!(
        network.deliver_next_to(&node(10)),
        Some(DeliveryOutcome::Inserted)
    );
    assert_eq!(
        network.deliver_next_to(&node(10)),
        Some(DeliveryOutcome::Inserted)
    );

    assert_eq!(
        network.deliver_next_to(&node(11)),
        Some(DeliveryOutcome::Buffered)
    );
    assert_eq!(
        network
            .node(&node(11))
            .expect("node B should exist")
            .pending_len(),
        1
    );

    assert_eq!(
        network.deliver_next_to(&node(11)),
        Some(DeliveryOutcome::Inserted)
    );

    network.retry_all_buffers();

    let node_a = network.node(&node(10)).expect("node A should exist");
    let node_b = network.node(&node(11)).expect("node B should exist");

    assert!(node_a.knows_block(&genesis.identity));
    assert!(node_a.knows_block(&child.identity));
    assert!(node_b.knows_block(&genesis.identity));
    assert!(node_b.knows_block(&child.identity));
    assert_eq!(node_b.pending_len(), 0);
}
