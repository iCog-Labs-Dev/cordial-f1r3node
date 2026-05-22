use cordial_miners_core::consensus::ProposalError;
use cordial_miners_core::consensus::ValidationConfig;
use cordial_miners_core::simulation::dissemination::{DeliveryOutcome, SimNetwork, SimNode};
use cordial_miners_core::{Block, BlockContent, BlockIdentity, NodeId};
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
    bonds.insert(node(1), 100); // Include a bond for the block creator to ensure blocks are considered valid
    bonds.insert(node(2), 100); // Include a bond for the block creator to ensure blocks are considered valid

    let node_a = SimNode::new(node(10), bonds.clone(), simulation_validation_config()); // Create a separate bonds map for node A to ensure it has the same bond information as node B
    let node_b = SimNode::new(node(11), bonds, simulation_validation_config()); // Create a separate bonds map for node B to ensure it has the same bond information as node A
    let mut network = SimNetwork::new(vec![node_a, node_b]);

    let genesis = create_block(1, 1, HashSet::new()); // Create a genesis block with a bond for the creator to ensure it's considered valid
    let child = create_block(2, 2, HashSet::from([genesis.identity.clone()])); // Create a child block that references the genesis block

    network.queue_delivery(node(10), genesis.clone()); // Queue the genesis block for delivery to node A
    network.queue_delivery(node(10), child.clone()); // Queue the child block for delivery to node A
    network.queue_delivery(node(11), child.clone()); // Queue the child block for delivery to node B before the genesis block to simulate out-of-order delivery
    network.queue_delivery(node(11), genesis.clone()); // Queue the genesis block for delivery to node B after the child block to simulate out-of-order delivery
    // in the queue for node B, the child block will be attempted for delivery before the genesis block, which should result in it being buffered until the genesis block is delivered and processed
    assert_eq!(network.queued_delivery_count(), 4); // Ensure all blocks are queued for delivery

    assert_eq!(
        network.deliver_next_to(&node(10)), // Deliver the genesis block to node A, which should be inserted successfully
        Some(DeliveryOutcome::Inserted)
    );
    assert_eq!(
        network.deliver_next_to(&node(10)), // Deliver the child block to node A, which should be inserted successfully since the genesis block is already known
        Some(DeliveryOutcome::Inserted)
    );

    assert_eq!(
        network.deliver_next_to(&node(11)),
        Some(DeliveryOutcome::Buffered) // Deliver the child block to node B, which should be buffered since the genesis block is not yet known
    );
    assert_eq!(
        network
            .node(&node(11))
            .expect("node B should exist")
            .pending_len(), // Check that the child block is in the pending buffer of node B
        1
    );

    assert_eq!(
        network.deliver_next_to(&node(11)),
        Some(DeliveryOutcome::Inserted) // Deliver the genesis block to node B, which should be inserted successfully and trigger a retry of the buffered child block
    );

    network.retry_all_buffers(); // Retry all buffered blocks in the network, which should result in the child block being inserted into node B since its parent (the genesis block) is now known

    let node_a = network.node(&node(10)).expect("node A should exist"); // Retrieve node A from the network to check its known blocks
    let node_b = network.node(&node(11)).expect("node B should exist"); // Retrieve node B from the network to check its known blocks

    assert!(node_a.knows_block(&genesis.identity)); // Check that node A knows the genesis block
    assert!(node_a.knows_block(&child.identity)); // Check that node A knows the child block
    assert!(node_b.knows_block(&genesis.identity)); // Check that node B knows the genesis block
    assert!(node_b.knows_block(&child.identity)); // Check that node B knows the child block after the genesis block was delivered and the buffered child block was retried
    assert_eq!(node_b.pending_len(), 0); // Check that node B's pending buffer is empty after the buffered child block was successfully inserted
}

#[test]
fn proposal_construction_converges_after_nodes_catch_up_on_visible_tips() {
    let mut bonds = HashMap::new();
    bonds.insert(node(1), 100); // Include a bond for the block creator to ensure blocks are considered valid
    bonds.insert(node(2), 100); // Include a bond for the block creator to ensure blocks are considered valid
    bonds.insert(node(3), 100); // Include a bond for the block creator to ensure blocks are considered valid
    bonds.insert(node(4), 100); // Include a bond for the block creator to ensure blocks are considered valid

    let node_a = SimNode::new(node(20), bonds.clone(), simulation_validation_config()); // Create a separate bonds map for node A to ensure it has the same bond information as node B
    let node_b = SimNode::new(node(21), bonds, simulation_validation_config()); // Create a separate bonds map for node B to ensure it has the same bond information as node A
    let mut network = SimNetwork::new(vec![node_a, node_b]); // Create a simulation network with both nodes

    let tip1 = create_block(1, 1, HashSet::new()); // Create a tip block with a bond for the creator to ensure it's considered valid
    let tip2 = create_block(2, 2, HashSet::new()); // Create another tip block with a bond for the creator to ensure it's considered valid
    let tip3 = create_block(3, 3, HashSet::new()); // Create a third tip block with a bond for the creator to ensure it's considered valid

    for block in [&tip1, &tip2, &tip3] {
        network.queue_delivery(node(20), block.clone()); // Queue all three tip blocks for delivery to node A to ensure it has all the visible tips needed for proposal construction
    }
    for block in [&tip1, &tip2] {
        network.queue_delivery(node(21), block.clone()); // Queue only two of the tip blocks for delivery to node B to simulate it being slightly behind in terms of visible tips needed for proposal construction
    }

    assert_eq!(
        network.deliver_next_to(&node(20)),
        Some(DeliveryOutcome::Inserted) // Deliver the first tip block to node A, which should be inserted successfully
    );
    assert_eq!(
        network.deliver_next_to(&node(20)),
        Some(DeliveryOutcome::Inserted) // Deliver the second tip block to node A, which should be inserted successfully
    );
    assert_eq!(
        network.deliver_next_to(&node(20)),
        Some(DeliveryOutcome::Inserted) // Deliver the third tip block to node A, which should be inserted successfully
    );

    assert_eq!(
        network.deliver_next_to(&node(21)),
        Some(DeliveryOutcome::Inserted) // Deliver the first tip block to node B, which should be inserted successfully
    );
    assert_eq!(
        network.deliver_next_to(&node(21)),
        Some(DeliveryOutcome::Inserted) // Deliver the second tip block to node B, which should be inserted successfully
    );

    let payload = vec![9, 9];

    let candidate_a = network // Have node A attempt to build a block candidate using the three tip blocks it knows, which should succeed since it has all the required visible tips
        .node(&node(20))
        .expect("node A should exist")
        .build_block_candidate(payload.clone())
        .expect("node A should have enough visible tips to propose");

    let candidate_b_before = network // Have node B attempt to build a block candidate using the two tip blocks it knows, which should fail since it doesn't have all the required visible tips (it is missing tip3) and thus doesn't have enough acknowledgements to propose
        .node(&node(21))
        .expect("node B should exist")
        .build_block_candidate(payload.clone());

    assert!(matches!(
        candidate_b_before,
        Err(ProposalError::InsufficientAcknowledgements {
            // Check that the error is specifically about insufficient acknowledgements, which indicates that node B correctly identified that it doesn't have enough visible tips to propose
            observed: 2,
            required: 3,
        })
    ));

    network.queue_delivery(node(21), tip3.clone()); // Queue the missing tip block for delivery to node B to simulate it catching up on the visible tips needed for proposal construction
    assert_eq!(
        network.deliver_next_to(&node(21)),
        Some(DeliveryOutcome::Inserted)
    );

    let candidate_b_after = network // Have node B attempt to build a block candidate using the three tip blocks it now knows, which should succeed since it has all the required visible tips
        .node(&node(21))
        .expect("node B should exist")
        .build_block_candidate(payload)
        .expect("node B should propose after catching up");

    assert_eq!(candidate_a.payload, candidate_b_after.payload); // Check that the payloads of the two candidates are the same, which indicates that both nodes constructed their candidates using the same set of visible tips and thus converged on the same proposal content
    assert_eq!(candidate_a.predecessors, candidate_b_after.predecessors); // Check that the predecessors of the two candidates are the same, which indicates that both nodes constructed their candidates using the same set of visible tips and thus converged on the same proposal content
}
