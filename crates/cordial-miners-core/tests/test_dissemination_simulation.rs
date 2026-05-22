use cordial_miners_core::simulation::dissemination::{DeliveryOutcome, SimNode};
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

