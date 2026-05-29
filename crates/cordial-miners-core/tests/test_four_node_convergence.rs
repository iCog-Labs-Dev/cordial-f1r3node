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
            // Treat the payload as the intercepted transaction bytes for the
            // purposes of the convergence demonstration.
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
fn four_nodes_converge_on_same_final_leader_and_tau_output() {
    let mut bonds = HashMap::new();
    bonds.insert(node(1), 100);
    bonds.insert(node(2), 100);
    bonds.insert(node(3), 100);
    bonds.insert(node(4), 100);

    // Four simulated observers with the same bonded validator set.
    let observer_a = SimNode::new(node(70), bonds.clone(), simulation_validation_config());
    let observer_b = SimNode::new(node(71), bonds.clone(), simulation_validation_config());
    let observer_c = SimNode::new(node(72), bonds.clone(), simulation_validation_config());
    let observer_d = SimNode::new(node(73), bonds, simulation_validation_config());
    let mut network = SimNetwork::new(vec![observer_a, observer_b, observer_c, observer_d]);

    let wavelength = 3u64;
    let n = 4usize;
    let f = 1usize;

    // Wave 0:
    // round 0 leader
    let leader = create_block(1, 1, HashSet::new());

    // round 1 witnesses approve the leader
    let r1_v2 = create_block(2, 2, HashSet::from([leader.identity.clone()]));
    let r1_v3 = create_block(3, 3, HashSet::from([leader.identity.clone()]));
    let r1_v4 = create_block(4, 4, HashSet::from([leader.identity.clone()]));

    // round 2 witnesses super-ratify by observing the full round-1 support set
    let round1_preds = HashSet::from([
        r1_v2.identity.clone(),
        r1_v3.identity.clone(),
        r1_v4.identity.clone(),
    ]);
    let r2_v2 = create_block(2, 5, round1_preds.clone());
    let r2_v3 = create_block(3, 6, round1_preds.clone());
    let r2_v4 = create_block(4, 7, round1_preds);

    let all_blocks = [&leader, &r1_v2, &r1_v3, &r1_v4, &r2_v2, &r2_v3, &r2_v4];

    // Observer A receives everything in dependency order.
    for block in all_blocks {
        network.queue_delivery(node(70), block.clone());
    }

    // Observer B receives the same wave in mixed order and must buffer some
    // blocks until parents arrive.
    for block in [&r2_v3, &r1_v2, &r2_v2, &leader, &r1_v4, &r2_v4, &r1_v3] {
        network.queue_delivery(node(71), block.clone());
    }

    // Observer C is partitioned at first: it sees the leader and one witness,
    // then later receives the rest after healing.
    for block in [&leader, &r1_v2] {
        network.queue_delivery(node(72), block.clone());
    }

    // Observer D sees a different but still valid order.
    for block in [&leader, &r1_v4, &r2_v4, &r1_v3, &r2_v3, &r1_v2, &r2_v2] {
        network.queue_delivery(node(73), block.clone());
    }

    while network.deliver_next_to(&node(70)).is_some() {}
    while network.deliver_next_to(&node(71)).is_some() {}
    while network.deliver_next_to(&node(72)).is_some() {}
    while network.deliver_next_to(&node(73)).is_some() {}
    network.retry_all_buffers();

    let observer_c_before_heal = network.node(&node(72)).expect("observer C should exist");
    assert_eq!(
        observer_c_before_heal.latest_final_leader(wavelength, n, f, leader_node1),
        None
    );
    assert!(
        observer_c_before_heal
            .ordered_output(wavelength, n, f, leader_node1)
            .expect("observer C should compute tau before heal")
            .is_empty()
    );

    // Heal the partition for observer C.
    for block in [&r2_v4, &r1_v4, &r2_v2, &r2_v3, &r1_v3] {
        network.queue_delivery(node(72), block.clone());
    }

    while network.deliver_next_to(&node(72)).is_some() {}
    network.retry_all_buffers();

    let observers = [node(70), node(71), node(72), node(73)];
    let mut final_leaders = Vec::new();
    let mut tau_outputs = Vec::new();

    for observer_id in observers {
        let observer = network.node(&observer_id).expect("observer should exist");

        assert_eq!(observer.pending_len(), 0, "observer should have no buffered blocks left");
        for block in all_blocks {
            assert!(
                observer.knows_block(&block.identity),
                "observer {:?} should know block {:?}",
                observer_id,
                block.identity
            );
        }

        final_leaders.push(observer.latest_final_leader(wavelength, n, f, leader_node1));
        tau_outputs.push(
            observer
                .ordered_output(wavelength, n, f, leader_node1)
                .expect("observer should produce ordered output"),
        );
    }

    for final_leader in &final_leaders {
        assert_eq!(*final_leader, Some(leader.identity.clone()));
    }

    assert!(
        !tau_outputs[0].is_empty(),
        "a finalized leader wave should produce non-empty tau output"
    );
    assert_eq!(tau_outputs[0], tau_outputs[1]);
    assert_eq!(tau_outputs[1], tau_outputs[2]);
    assert_eq!(tau_outputs[2], tau_outputs[3]);
}

#[test]
fn four_node_convergence_records_buffering_before_catch_up() {
    let mut bonds = HashMap::new();
    bonds.insert(node(1), 100);
    bonds.insert(node(2), 100);
    bonds.insert(node(3), 100);
    bonds.insert(node(4), 100);

    let observer = SimNode::new(node(80), bonds, simulation_validation_config());
    let mut network = SimNetwork::new(vec![observer]);

    let leader = create_block(1, 1, HashSet::new());
    let witness = create_block(2, 2, HashSet::from([leader.identity.clone()]));

    network.queue_delivery(node(80), witness.clone());
    network.queue_delivery(node(80), leader.clone());

    assert_eq!(
        network.deliver_next_to(&node(80)),
        Some(DeliveryOutcome::Buffered)
    );
    assert_eq!(
        network.node(&node(80)).expect("observer should exist").pending_len(),
        1
    );

    assert_eq!(
        network.deliver_next_to(&node(80)),
        Some(DeliveryOutcome::Inserted)
    );
    network.retry_all_buffers();

    let observer = network.node(&node(80)).expect("observer should exist");
    assert_eq!(observer.pending_len(), 0);
    assert!(observer.knows_block(&leader.identity));
    assert!(observer.knows_block(&witness.identity));
}
