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

fn leader_node1(_wave: u64) -> Option<NodeId> {
    Some(node(1))
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

#[test]
fn finality_and_tau_converge_after_out_of_order_wave_delivery() {
    let mut bonds = HashMap::new();
    bonds.insert(node(1), 100); // Include a bond for the block creator to ensure blocks are considered valid
    bonds.insert(node(2), 100);
    bonds.insert(node(3), 100);
    bonds.insert(node(4), 100);

    let node_a = SimNode::new(node(30), bonds.clone(), simulation_validation_config()); // Create a separate bonds map for node A to ensure it has the same bond information as node B
    let node_b = SimNode::new(node(31), bonds, simulation_validation_config()); // Create a separate bonds map for node B to ensure it has the same bond information as node A
    let mut network = SimNetwork::new(vec![node_a, node_b]);

    let wavelength = 3u64; // Set the wavelength to 3 so that the leader selection will rotate every 3 waves, which allows us to test finality and tau convergence across multiple waves with the same leader and then a wave with a different leader
    let n = 4usize; // Set n to 4 to allow for a small number of nodes while still enabling finality with f=1
    let f = 1usize; // Set f to 1 to allow for finality with n=4 while still enabling some tolerance for out-of-order delivery and buffering, here f represents the maximum number of faulty nodes that can be tolerated while still achieving finality, and setting it to 1 allows us to test that the protocol can still achieve finality even if one node receives blocks in a different order and has to buffer them until it can process them in the correct order

    let leader = create_block(1, 1, HashSet::new()); // Create a leader block for the first wave with a bond for the creator to ensure it's considered valid, this will be the only block in the first wave and will be the parent of all subsequent blocks in the second wave, which allows us to test that both nodes can achieve finality on this leader block even if they receive the subsequent blocks in different orders

    let r1_v2 = create_block(2, 2, HashSet::from([leader.identity.clone()])); // Create a block for the second wave that references the leader block, with a bond for the creator to ensure it's considered valid, this will be one of the blocks in the second wave and will be used to test that both nodes can achieve finality on the leader block even if they receive the blocks in different orders
    let r1_v3 = create_block(3, 3, HashSet::from([leader.identity.clone()])); // Create another block for the second wave that references the leader block, with a bond for the creator to ensure it's considered valid, this will be another block in the second wave and will be used to test that both nodes can achieve finality on the leader block even if they receive the blocks in different orders
    let r1_v4 = create_block(4, 4, HashSet::from([leader.identity.clone()])); // Create a third block for the second wave that references the leader block, with a bond for the creator to ensure it's considered valid, this will be another block in the second wave and will be used to test that both nodes can achieve finality on the leader block even if they receive the blocks in different orders

    let round1_preds = HashSet::from([
        // Create a set of predecessors for the third wave blocks that includes all the blocks from the second wave, which allows us to test that both nodes can achieve finality on the leader block and then produce the same tau even if they receive the blocks in different orders
        r1_v2.identity.clone(),
        r1_v3.identity.clone(),
        r1_v4.identity.clone(),
    ]);
    let r2_v2 = create_block(2, 5, round1_preds.clone()); // Create a block for the third wave that references all the blocks from the second wave, with a bond for the creator to ensure it's considered valid, this will be one of the blocks in the third wave and will be used to test that both nodes can produce the same tau even if they receive the blocks in different orders
    let r2_v3 = create_block(3, 6, round1_preds.clone()); // Create another block for the third wave that references all the blocks from the second wave, with a bond for the creator to ensure it's considered valid, this will be another block in the third wave and will be used to test that both nodes can produce the same tau even if they receive the blocks in different orders
    let r2_v4 = create_block(4, 7, round1_preds); // Create a third block for the third wave that references all the blocks from the second wave, with a bond for the creator to ensure it's considered valid, this will be another block in the third wave and will be used to test that both nodes can produce the same tau even if they receive the blocks in different orders

    for block in [&leader, &r1_v2, &r1_v3, &r1_v4, &r2_v2, &r2_v3, &r2_v4] {
        // Queue all the blocks for delivery to node A in order of their creation, which simulates node A receiving the blocks in the order they were created and thus being able to process them without buffering
        network.queue_delivery(node(30), block.clone());
    }

    for block in [&r2_v3, &r1_v2, &r2_v2, &leader, &r1_v4, &r2_v4, &r1_v3] {
        // Queue all the blocks for delivery to node B in a different order that simulates it receiving the blocks in a more jumbled order, which will require it to buffer some blocks until it can process them in the correct order based on their dependencies, this tests that even with out-of-order delivery and buffering, node B can still achieve finality on the leader block and produce the same tau as node A once it has processed all the blocks
        network.queue_delivery(node(31), block.clone());
    }

    while network.deliver_next_to(&node(30)).is_some() {} // Deliver all the blocks to node A, which should be processed in order without buffering since they were queued in creation order
    while network.deliver_next_to(&node(31)).is_some() {} // Deliver all the blocks to node B, which should require buffering for some blocks until their dependencies are processed, but should ultimately result in all blocks being processed once their dependencies are met
    network.retry_all_buffers(); // Retry all buffered blocks in the network to ensure that any blocks that were buffered due to out-of-order delivery are now processed once their dependencies have been met, this is especially important for node B which received the blocks in a jumbled order and thus likely had to buffer several blocks until it could process them in the correct order

    let node_a = network.node(&node(30)).expect("node A should exist"); // Retrieve node A from the network to check its finality and tau output
    let node_b = network.node(&node(31)).expect("node B should exist"); // Retrieve node B from the network to check its finality and tau output

    let final_a = node_a.latest_final_leader(wavelength, n, f, leader_node1); // Check the latest final leader for node A, which should be the leader block since both nodes should have achieved finality on it despite the out-of-order delivery and buffering
    let final_b = node_b.latest_final_leader(wavelength, n, f, leader_node1); // Check the latest final leader for node B, which should also be the leader block since both nodes should have achieved finality on it despite the out-of-order delivery and buffering
    assert_eq!(final_a, Some(leader.identity.clone())); // Assert that the final leader for node A is the leader block, which indicates that node A successfully achieved finality on the leader block despite the out-of-order delivery and buffering of subsequent blocks
    assert_eq!(final_b, Some(leader.identity.clone())); // Assert that the final leader for node B is also the leader block, which indicates that node B successfully achieved finality on the leader block despite the out-of-order delivery and buffering of subsequent blocks

    // Check the tau output for both nodes, which should be the same and should reflect the same set of finalized blocks and their dependencies, thus demonstrating that both nodes converged on the same view of finality and the same tau output despite receiving the blocks in different orders and having to buffer some blocks until they could process them in the correct order
    let tau_a = node_a
        .ordered_output(wavelength, n, f, leader_node1)
        .expect("node A should produce ordered output");
    let tau_b = node_b
        .ordered_output(wavelength, n, f, leader_node1)
        .expect("node B should produce ordered output");

    // Assert that the tau output for both nodes is not empty, which indicates that they were able to produce a tau output based on the finalized blocks and their dependencies, and that the tau output reflects the same view of finality and the same set of blocks despite the out-of-order delivery and buffering
    assert!(
        !tau_a.is_empty(),
        "finalized wave should produce non-empty tau"
    );
    assert_eq!(tau_a, tau_b);
}

#[test]
fn equivocating_leader_attempt_is_rejected_and_honest_wave_still_converges() {
    // Standard 4-validator setting with n = 4, f = 1 so a single honest wave
    // can still finalize its leader after one adversarial attempt.
    let mut bonds = HashMap::new();
    bonds.insert(node(1), 100);
    bonds.insert(node(2), 100);
    bonds.insert(node(3), 100);
    bonds.insert(node(4), 100);

    // Node A will see only the honest path.
    // Node B will see the honest leader first, then a conflicting leader block.
    let node_a = SimNode::new(node(40), bonds.clone(), simulation_validation_config());
    let node_b = SimNode::new(node(41), bonds, simulation_validation_config());
    let mut network = SimNetwork::new(vec![node_a, node_b]);

    let wavelength = 3u64;
    let n = 4usize;
    let f = 1usize;

    // Honest leader block for wave 0.
    let leader = create_block(1, 1, HashSet::new());
    // Conflicting block by the same creator. Since node B will already know
    // `leader`, this second block should be rejected as an equivocation.
    let equivocated_leader = create_block(1, 99, HashSet::new());

    // Round-1 witness blocks that ratify the honest leader.
    let r1_v2 = create_block(2, 2, HashSet::from([leader.identity.clone()]));
    let r1_v3 = create_block(3, 3, HashSet::from([leader.identity.clone()]));
    let r1_v4 = create_block(4, 4, HashSet::from([leader.identity.clone()]));

    // Round-2 witness blocks that super-ratify the wave by observing all
    // round-1 witnesses.
    let round1_preds = HashSet::from([
        r1_v2.identity.clone(),
        r1_v3.identity.clone(),
        r1_v4.identity.clone(),
    ]);
    let r2_v2 = create_block(2, 5, round1_preds.clone());
    let r2_v3 = create_block(3, 6, round1_preds.clone());
    let r2_v4 = create_block(4, 7, round1_preds);

    // Node A receives the honest wave in dependency order.
    for block in [&leader, &r1_v2, &r1_v3, &r1_v4, &r2_v2, &r2_v3, &r2_v4] {
        network.queue_delivery(node(40), block.clone());
    }

    // Node B receives the honest leader first, then the conflicting leader
    // attempt, then the rest of the honest wave in mixed order.
    network.queue_delivery(node(41), leader.clone());
    network.queue_delivery(node(41), equivocated_leader.clone());
    for block in [&r2_v3, &r1_v2, &r2_v2, &r1_v4, &r2_v4, &r1_v3] {
        network.queue_delivery(node(41), block.clone());
    }

    // Node A processes the honest path completely.
    while network.deliver_next_to(&node(40)).is_some() {}

    // Node B first accepts the honest leader.
    assert_eq!(
        network.deliver_next_to(&node(41)),
        Some(DeliveryOutcome::Inserted)
    );

    // Node B then sees a second incomparable block by the same creator and
    // must reject it as an equivocation instead of buffering or inserting it.
    let equivocation_outcome = network
        .deliver_next_to(&node(41))
        .expect("equivocating leader attempt should be delivered");
    assert!(
        matches!(equivocation_outcome, DeliveryOutcome::Rejected(errors)
        if errors.iter().any(|error| matches!(
            error,
            cordial_miners_core::consensus::InvalidBlock::Equivocation { .. }
        ))),
        "equivocating leader should be rejected by the receiving node"
    );

    // Deliver the remaining honest witness blocks to node B, then let any
    // out-of-order dependencies resolve from the buffer.
    while network.deliver_next_to(&node(41)).is_some() {}
    network.retry_all_buffers();

    let node_a = network.node(&node(40)).expect("node A should exist");
    let node_b = network.node(&node(41)).expect("node B should exist");

    // The rejected equivocation must never enter node B's local DAG.
    assert!(!node_b.knows_block(&equivocated_leader.identity));

    // Despite the adversarial attempt, both nodes should still finalize the
    // same honest leader.
    let final_a = node_a.latest_final_leader(wavelength, n, f, leader_node1);
    let final_b = node_b.latest_final_leader(wavelength, n, f, leader_node1);
    assert_eq!(final_a, Some(leader.identity.clone()));
    assert_eq!(final_b, Some(leader.identity.clone()));

    // And once delivery stabilizes, both nodes should compute the same ordered
    // output sequence from that finalized view.
    let tau_a = node_a
        .ordered_output(wavelength, n, f, leader_node1)
        .expect("node A should produce ordered output");
    let tau_b = node_b
        .ordered_output(wavelength, n, f, leader_node1)
        .expect("node B should produce ordered output");

    assert_eq!(tau_a, tau_b);
}

#[test]
fn partitioned_node_catches_up_on_finality_and_tau_after_heal() {
    // Standard 4-validator setting with n = 4, f = 1 so a single honest wave
    let mut bonds = HashMap::new();
    bonds.insert(node(1), 100);
    bonds.insert(node(2), 100);
    bonds.insert(node(3), 100);
    bonds.insert(node(4), 100);

    // Node A will see the full wave and finalize the leader.
    // Node B will see only part of the wave due to a network partition, then heal and receive the rest of the wave, but not necessarily in dependency order.
    let node_a = SimNode::new(node(50), bonds.clone(), simulation_validation_config());
    let node_b = SimNode::new(node(51), bonds, simulation_validation_config());
    let mut network = SimNetwork::new(vec![node_a, node_b]);

    // Set parameters such that a single honest wave can finalize its leader and produce non-empty tau, even if one node receives the blocks in a different order and has to buffer them until it can process them in the correct order.
    let wavelength = 3u64;
    let n = 4usize;
    let f = 1usize;

    // Honest wave: leader, round-1 witnesses, then round-2 super-ratifiers.
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

    // Node A sees the full wave and can finalize normally.
    for block in [&leader, &r1_v2, &r1_v3, &r1_v4, &r2_v2, &r2_v3, &r2_v4] {
        network.queue_delivery(node(50), block.clone());
    }

    // During the partition, node B sees only the leader plus one witness.
    // That is not enough to super-ratify the wave or to compute non-empty tau.
    for block in [&leader, &r1_v2] {
        network.queue_delivery(node(51), block.clone());
    }

    // Deliver what node B sees during the partition, then let any out-of-order dependencies resolve from the buffer.
    while network.deliver_next_to(&node(50)).is_some() {}
    while network.deliver_next_to(&node(51)).is_some() {}
    network.retry_all_buffers();

    let node_b_before_heal = network.node(&node(51)).expect("node B should exist");
    assert_eq!(
        node_b_before_heal.latest_final_leader(wavelength, n, f, leader_node1),
        None
    );
    assert!(
        node_b_before_heal
            .ordered_output(wavelength, n, f, leader_node1)
            .expect("partitioned node should still compute tau")
            .is_empty(),
        "partitioned node should not order anything before enough witnesses arrive"
    );

    // Partition heals: node B receives the missing witness and round-2 blocks,
    // but not necessarily in dependency order.
    for block in [&r2_v4, &r1_v4, &r2_v2, &r2_v3, &r1_v3] {
        network.queue_delivery(node(51), block.clone());
    }

    // Deliver the remaining blocks to node B, then let any out-of-order dependencies resolve from the buffer.
    while network.deliver_next_to(&node(51)).is_some() {}
    network.retry_all_buffers();

    // After healing, node B should have the same finalized leader and the same tau output as node A, demonstrating that it successfully caught up on the wave despite the initial partition and out-of-order delivery.
    let node_a = network.node(&node(50)).expect("node A should exist");
    let node_b = network.node(&node(51)).expect("node B should exist");

    // Both nodes should finalize the same leader block, demonstrating that node B was able to catch up on the wave and achieve finality on the same leader as node A despite the initial partition and out-of-order delivery.
    let final_a: Option<BlockIdentity> = node_a.latest_final_leader(wavelength, n, f, leader_node1);
    let final_b = node_b.latest_final_leader(wavelength, n, f, leader_node1);
    assert_eq!(final_a, Some(leader.identity.clone()));
    assert_eq!(final_b, Some(leader.identity.clone()));

    let tau_a = node_a
        .ordered_output(wavelength, n, f, leader_node1)
        .expect("node A should produce ordered output");
    let tau_b = node_b
        .ordered_output(wavelength, n, f, leader_node1)
        .expect("node B should produce ordered output after healing");

    assert_eq!(tau_a, tau_b); // Both nodes should produce the same tau output, demonstrating that node B successfully caught up on the wave and converged on the same view of finality and the same tau output as node A despite the initial partition and out-of-order delivery.
}
