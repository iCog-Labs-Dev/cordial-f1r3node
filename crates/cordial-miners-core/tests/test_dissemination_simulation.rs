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
    // Minimal 2-validator setting: validator 1 creates the parent, validator 2
    // creates a child that depends on it.
    let mut bonds = HashMap::new();
    bonds.insert(node(1), 100);
    bonds.insert(node(2), 100);

    // Single simulated node receiving blocks from the network.
    let mut sim_node = SimNode::new(node(9), bonds, simulation_validation_config());

    let genesis = create_block(1, 1, HashSet::new());
    let child = create_block(2, 2, HashSet::from([genesis.identity.clone()]));

    // The child arrives before its parent, so it cannot be inserted yet and
    // must be buffered.
    let early_delivery = sim_node.receive_block(child.clone());
    assert_eq!(early_delivery, DeliveryOutcome::Buffered);
    assert_eq!(sim_node.pending_len(), 1);
    assert!(!sim_node.knows_block(&child.identity));

    // Once the parent arrives, it is inserted normally.
    let parent_delivery = sim_node.receive_block(genesis.clone());
    assert_eq!(parent_delivery, DeliveryOutcome::Inserted);
    assert!(sim_node.knows_block(&genesis.identity));

    // Retrying the buffer should now resolve the child as well.
    sim_node.retry_buffered_blocks();

    assert_eq!(sim_node.pending_len(), 0);
    assert!(sim_node.knows_block(&child.identity));
}

#[test]
fn two_nodes_converge_after_receiving_the_same_blocks_in_different_orders() {
    // Two bonded creators produce a tiny parent/child chain.
    let mut bonds = HashMap::new();
    bonds.insert(node(1), 100);
    bonds.insert(node(2), 100);

    // Node A and node B start from empty local views.
    let node_a = SimNode::new(node(10), bonds.clone(), simulation_validation_config());
    let node_b = SimNode::new(node(11), bonds, simulation_validation_config());
    let mut network = SimNetwork::new(vec![node_a, node_b]);

    let genesis = create_block(1, 1, HashSet::new());
    let child = create_block(2, 2, HashSet::from([genesis.identity.clone()]));

    // Node A receives the chain in order.
    network.queue_delivery(node(10), genesis.clone());
    network.queue_delivery(node(10), child.clone());
    // Node B sees the same chain out of order and must buffer the child first.
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

    // Node B cannot yet insert the child because it does not know the parent.
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

    // Once the parent arrives, node B can process it and later resolve the child.
    assert_eq!(
        network.deliver_next_to(&node(11)),
        Some(DeliveryOutcome::Inserted)
    );

    network.retry_all_buffers();

    let node_a = network.node(&node(10)).expect("node A should exist");
    let node_b = network.node(&node(11)).expect("node B should exist");

    // After catch-up, both nodes should know the same chain and the buffer
    // should be empty again.
    assert!(node_a.knows_block(&genesis.identity));
    assert!(node_a.knows_block(&child.identity));
    assert!(node_b.knows_block(&genesis.identity));
    assert!(node_b.knows_block(&child.identity));
    assert_eq!(node_b.pending_len(), 0);
}

#[test]
fn proposal_construction_converges_after_nodes_catch_up_on_visible_tips() {
    // Four bonded validators so the unweighted acknowledgement threshold is 3.
    let mut bonds = HashMap::new();
    bonds.insert(node(1), 100);
    bonds.insert(node(2), 100);
    bonds.insert(node(3), 100);
    bonds.insert(node(4), 100);

    // Node A will have a complete local view. Node B will initially miss one tip.
    let node_a = SimNode::new(node(20), bonds.clone(), simulation_validation_config());
    let node_b = SimNode::new(node(21), bonds, simulation_validation_config());
    let mut network = SimNetwork::new(vec![node_a, node_b]);

    let tip1 = create_block(1, 1, HashSet::new());
    let tip2 = create_block(2, 2, HashSet::new());
    let tip3 = create_block(3, 3, HashSet::new());

    // Node A sees all three visible tips.
    for block in [&tip1, &tip2, &tip3] {
        network.queue_delivery(node(20), block.clone());
    }
    // Node B starts behind and sees only two of them.
    for block in [&tip1, &tip2] {
        network.queue_delivery(node(21), block.clone());
    }

    assert_eq!(
        network.deliver_next_to(&node(20)),
        Some(DeliveryOutcome::Inserted)
    );
    assert_eq!(
        network.deliver_next_to(&node(20)),
        Some(DeliveryOutcome::Inserted)
    );
    assert_eq!(
        network.deliver_next_to(&node(20)),
        Some(DeliveryOutcome::Inserted)
    );

    assert_eq!(
        network.deliver_next_to(&node(21)),
        Some(DeliveryOutcome::Inserted)
    );
    assert_eq!(
        network.deliver_next_to(&node(21)),
        Some(DeliveryOutcome::Inserted)
    );

    let payload = vec![9, 9];

    // Node A can already build a proposal because it has enough visible tips.
    let candidate_a = network
        .node(&node(20))
        .expect("node A should exist")
        .build_block_candidate(payload.clone())
        .expect("node A should have enough visible tips to propose");

    // Node B cannot yet build a proposal because it is still missing one tip.
    let candidate_b_before = network
        .node(&node(21))
        .expect("node B should exist")
        .build_block_candidate(payload.clone());

    assert!(matches!(
        candidate_b_before,
        Err(ProposalError::InsufficientAcknowledgements {
            observed: 2,
            required: 3,
        })
    ));

    // Once the missing tip arrives, node B should catch up to the same view.
    network.queue_delivery(node(21), tip3.clone());
    assert_eq!(
        network.deliver_next_to(&node(21)),
        Some(DeliveryOutcome::Inserted)
    );

    let candidate_b_after = network
        .node(&node(21))
        .expect("node B should exist")
        .build_block_candidate(payload)
        .expect("node B should propose after catching up");

    // After catch-up, both nodes should construct the same proposal.
    assert_eq!(candidate_a.payload, candidate_b_after.payload);
    assert_eq!(candidate_a.predecessors, candidate_b_after.predecessors);
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

#[test]
fn unbonded_sender_injection_is_rejected_and_honest_nodes_still_converge() {
    let mut bonds = HashMap::new();
    bonds.insert(node(1), 100);
    bonds.insert(node(2), 100);
    bonds.insert(node(3), 100);
    bonds.insert(node(4), 100);

    let node_a = SimNode::new(node(60), bonds.clone(), simulation_validation_config());
    let node_b = SimNode::new(node(61), bonds, simulation_validation_config());
    let mut network = SimNetwork::new(vec![node_a, node_b]);

    let wavelength = 3u64;
    let n = 4usize;
    let f = 1usize;

    // Honest wave that should finalize normally.
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

    // Block by an unbonded creator. It should be rejected immediately by node B.
    let injected = create_block(9, 99, HashSet::from([leader.identity.clone()]));

    for block in [&leader, &r1_v2, &r1_v3, &r1_v4, &r2_v2, &r2_v3, &r2_v4] {
        network.queue_delivery(node(60), block.clone());
    }

    network.queue_delivery(node(61), leader.clone());
    network.queue_delivery(node(61), injected.clone());
    for block in [&r1_v2, &r1_v3, &r1_v4, &r2_v2, &r2_v3, &r2_v4] {
        network.queue_delivery(node(61), block.clone());
    }

    while network.deliver_next_to(&node(60)).is_some() {}

    assert_eq!(
        network.deliver_next_to(&node(61)),
        Some(DeliveryOutcome::Inserted)
    );

    let injected_outcome = network
        .deliver_next_to(&node(61))
        .expect("unbonded block should be delivered");
    assert!(
        matches!(injected_outcome, DeliveryOutcome::Rejected(errors)
        if errors.iter().any(|error| matches!(
            error,
            cordial_miners_core::consensus::InvalidBlock::UnknownSender { .. }
        ))),
        "unbonded sender should be rejected by the receiving node"
    );

    while network.deliver_next_to(&node(61)).is_some() {}
    network.retry_all_buffers();

    let node_a = network.node(&node(60)).expect("node A should exist");
    let node_b = network.node(&node(61)).expect("node B should exist");

    assert!(!node_b.knows_block(&injected.identity));

    let final_a = node_a.latest_final_leader(wavelength, n, f, leader_node1);
    let final_b = node_b.latest_final_leader(wavelength, n, f, leader_node1);
    assert_eq!(final_a, Some(leader.identity.clone()));
    assert_eq!(final_b, Some(leader.identity.clone()));

    let tau_a = node_a
        .ordered_output(wavelength, n, f, leader_node1)
        .expect("node A should produce ordered output");
    let tau_b = node_b
        .ordered_output(wavelength, n, f, leader_node1)
        .expect("node B should produce ordered output");

    assert_eq!(tau_a, tau_b);
}

#[test]
fn duplicate_delivery_after_convergence_does_not_change_finality_or_tau() {
    let mut bonds = HashMap::new();
    bonds.insert(node(1), 100);
    bonds.insert(node(2), 100);
    bonds.insert(node(3), 100);
    bonds.insert(node(4), 100);

    let node_a = SimNode::new(node(70), bonds.clone(), simulation_validation_config());
    let node_b = SimNode::new(node(71), bonds, simulation_validation_config());
    let mut network = SimNetwork::new(vec![node_a, node_b]);

    let wavelength = 3u64;
    let n = 4usize;
    let f = 1usize;

    // Honest wave that should finalize leader 1.
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

    for recipient in [node(70), node(71)] {
        for block in [&leader, &r1_v2, &r1_v3, &r1_v4, &r2_v2, &r2_v3, &r2_v4] {
            network.queue_delivery(recipient.clone(), block.clone());
        }
    }

    while network.deliver_next_to(&node(70)).is_some() {}
    while network.deliver_next_to(&node(71)).is_some() {}
    network.retry_all_buffers();

    let node_b = network.node(&node(71)).expect("node B should exist");
    let final_before = node_b.latest_final_leader(wavelength, n, f, leader_node1);
    let tau_before = node_b
        .ordered_output(wavelength, n, f, leader_node1)
        .expect("node B should produce ordered output");

    assert_eq!(final_before, Some(leader.identity.clone()));
    assert!(!tau_before.is_empty());

    // The network replays already-known honest blocks to node B after it has
    // already converged. They should be treated as harmless duplicates.
    for block in [&leader, &r1_v2, &r2_v3] {
        network.queue_delivery(node(71), block.clone());
    }

    assert_eq!(
        network.deliver_next_to(&node(71)),
        Some(DeliveryOutcome::Inserted)
    );
    assert_eq!(
        network.deliver_next_to(&node(71)),
        Some(DeliveryOutcome::Inserted)
    );
    assert_eq!(
        network.deliver_next_to(&node(71)),
        Some(DeliveryOutcome::Inserted)
    );
    network.retry_all_buffers();

    let node_b = network.node(&node(71)).expect("node B should still exist");
    let final_after = node_b.latest_final_leader(wavelength, n, f, leader_node1);
    let tau_after = node_b
        .ordered_output(wavelength, n, f, leader_node1)
        .expect("node B should still produce ordered output");

    assert_eq!(node_b.pending_len(), 0);
    assert_eq!(final_before, final_after);
    assert_eq!(tau_before, tau_after);
}

#[test]
fn weighted_path_can_remain_empty_after_unweighted_convergence() {
    // Four active validators build an honest wave, but a fifth bonded validator
    // holds most of the stake and never appears in the blocklace. This should
    // allow unweighted convergence while keeping the weighted path empty.
    let mut bonds = HashMap::new();
    bonds.insert(node(1), 1);
    bonds.insert(node(2), 1);
    bonds.insert(node(3), 1);
    bonds.insert(node(4), 1);
    bonds.insert(node(9), 100);

    let node_a = SimNode::new(node(80), bonds.clone(), simulation_validation_config());
    let node_b = SimNode::new(node(81), bonds, simulation_validation_config());
    let mut network = SimNetwork::new(vec![node_a, node_b]);

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

    for recipient in [node(80), node(81)] {
        network.queue_delivery(recipient.clone(), leader.clone());
        network.queue_delivery(recipient.clone(), r2_v3.clone());
        network.queue_delivery(recipient.clone(), r1_v2.clone());
        network.queue_delivery(recipient.clone(), r2_v2.clone());
        network.queue_delivery(recipient.clone(), r1_v4.clone());
        network.queue_delivery(recipient.clone(), r2_v4.clone());
        network.queue_delivery(recipient, r1_v3.clone());
    }

    while network.deliver_next_to(&node(80)).is_some() {}
    while network.deliver_next_to(&node(81)).is_some() {}
    network.retry_all_buffers();

    let node_a = network.node(&node(80)).expect("node A should exist");
    let node_b = network.node(&node(81)).expect("node B should exist");

    // Unweighted path converges normally on the honest leader and a non-empty tau.
    assert_eq!(
        node_a.latest_final_leader(wavelength, n, f, leader_node1),
        Some(leader.identity.clone())
    );
    assert_eq!(
        node_b.latest_final_leader(wavelength, n, f, leader_node1),
        Some(leader.identity.clone())
    );

    let unweighted_tau_a = node_a
        .ordered_output(wavelength, n, f, leader_node1)
        .expect("node A should produce unweighted tau");
    let unweighted_tau_b = node_b
        .ordered_output(wavelength, n, f, leader_node1)
        .expect("node B should produce unweighted tau");
    assert!(!unweighted_tau_a.is_empty());
    assert_eq!(unweighted_tau_a, unweighted_tau_b);

    // Weighted path stays empty because the high-stake validator never contributes.
    assert_eq!(
        node_a.latest_weighted_final_leader(wavelength, leader_node1),
        None
    );
    assert_eq!(
        node_b.latest_weighted_final_leader(wavelength, leader_node1),
        None
    );

    let weighted_tau_a = node_a
        .weighted_ordered_output(wavelength, leader_node1)
        .expect("node A should compute weighted tau");
    let weighted_tau_b = node_b
        .weighted_ordered_output(wavelength, leader_node1)
        .expect("node B should compute weighted tau");
    assert!(weighted_tau_a.is_empty());
    assert_eq!(weighted_tau_a, weighted_tau_b);
}

#[test]
fn weighted_finality_and_tau_converge_after_high_stake_participants_catch_up() {
    // Stake is skewed toward validators 2 and 3, so the weighted path depends
    // on them participating in the witness rounds.
    let mut bonds = HashMap::new();
    bonds.insert(node(1), 1);
    bonds.insert(node(2), 45);
    bonds.insert(node(3), 45);
    bonds.insert(node(4), 9);

    let node_a = SimNode::new(node(90), bonds.clone(), simulation_validation_config());
    let node_b = SimNode::new(node(91), bonds, simulation_validation_config());
    let mut network = SimNetwork::new(vec![node_a, node_b]);

    let wavelength = 3u64;

    // Honest wave led by node 1. Weighted finality should depend on the
    // participation of the high-stake validators 2 and 3.
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

    // Node A sees the complete wave and should achieve weighted finality.
    for block in [&leader, &r1_v2, &r1_v3, &r1_v4, &r2_v2, &r2_v3, &r2_v4] {
        network.queue_delivery(node(90), block.clone());
    }

    // Node B is temporarily missing the high-stake validator 3 path, so the
    // weighted path should stay empty until the partition heals.
    for block in [&leader, &r1_v2, &r1_v4, &r2_v2, &r2_v4] {
        network.queue_delivery(node(91), block.clone());
    }

    while network.deliver_next_to(&node(90)).is_some() {}
    while network.deliver_next_to(&node(91)).is_some() {}
    network.retry_all_buffers();

    let node_b_before_heal = network.node(&node(91)).expect("node B should exist");

    assert_eq!(
        network
            .node(&node(90))
            .expect("node A should exist")
            .latest_weighted_final_leader(wavelength, leader_node1),
        Some(leader.identity.clone())
    );
    assert_eq!(
        node_b_before_heal.latest_weighted_final_leader(wavelength, leader_node1),
        None
    );
    assert!(
        node_b_before_heal
            .weighted_ordered_output(wavelength, leader_node1)
            .expect("partitioned node should still compute weighted tau")
            .is_empty(),
        "without the missing high-stake path, weighted tau should stay empty"
    );

    // Heal the partition by delivering the missing high-stake validator 3 path.
    for block in [&r1_v3, &r2_v3] {
        network.queue_delivery(node(91), block.clone());
    }

    while network.deliver_next_to(&node(91)).is_some() {}
    network.retry_all_buffers();

    let node_a = network.node(&node(90)).expect("node A should exist");
    let node_b = network
        .node(&node(91))
        .expect("node B should exist after heal");

    let weighted_final_a = node_a.latest_weighted_final_leader(wavelength, leader_node1);
    let weighted_final_b = node_b.latest_weighted_final_leader(wavelength, leader_node1);
    assert_eq!(weighted_final_a, Some(leader.identity.clone()));
    assert_eq!(weighted_final_b, Some(leader.identity.clone()));

    let weighted_tau_a = node_a
        .weighted_ordered_output(wavelength, leader_node1)
        .expect("node A should produce weighted tau");
    let weighted_tau_b = node_b
        .weighted_ordered_output(wavelength, leader_node1)
        .expect("node B should produce weighted tau after healing");

    assert!(!weighted_tau_a.is_empty());
    assert_eq!(weighted_tau_a, weighted_tau_b);
}
