//! Adversarial simulation tests for Cordial Miners.
//!
//! These tests exercise the protocol under bad network behaviour and Byzantine
//! validators: delayed delivery, reverse and randomised delivery order,
//! equivocation, and temporary partitions that later heal.
//!
//! The organising principle is that **safety and liveness are asserted
//! separately**. Safety — no two honest nodes commit conflicting orders — must
//! hold at every step of every schedule, including schedules where nothing can
//! make progress. Liveness is only asserted once the adversary stops: after
//! delays expire, partitions heal, and the backlog drains.
//!
//! Every schedule here is deterministic. Randomised runs use a seeded
//! generator and report their seed on failure, so a red test is reproducible.

use cordial_miners_core::consensus::{InvalidBlock, is_supermajority};
use cordial_miners_core::simulation::adversary::{
    AdversarialNetwork, BlockFactory, ConsensusParams, DeliveryOrder, SafetyViolation,
    check_prefix_consistency, es_quorum_size, max_byzantine_faults, within_es_fault_bound,
};
use cordial_miners_core::simulation::dissemination::DeliveryOutcome;
use cordial_miners_core::{Block, BlockIdentity, NodeId};
use std::collections::HashSet;

const WAVELENGTH: u64 = 3;

fn node(id: u8) -> NodeId {
    NodeId(vec![id])
}

/// Validator 1 leads every wave, matching the existing simulation tests.
fn leader_v1(_wave: u64) -> Option<NodeId> {
    Some(node(1))
}

/// Build a dense `rounds`-deep DAG: every validator produces one block per
/// round, referencing every block of the previous round.
///
/// As in the existing simulation tests, blocks are minted up-front by the test
/// rather than derived from each node's live view. That keeps the block set
/// fixed across scenarios so that differences in outcome are attributable to
/// the *schedule* alone, which is what these tests vary.
fn build_rounds(
    factory: &mut BlockFactory,
    validators: &[NodeId],
    rounds: usize,
) -> Vec<Vec<Block>> {
    let mut dag: Vec<Vec<Block>> = Vec::with_capacity(rounds);

    for round in 0..rounds {
        let predecessors: HashSet<BlockIdentity> = if round == 0 {
            HashSet::new()
        } else {
            dag[round - 1]
                .iter()
                .map(|block| block.identity.clone())
                .collect()
        };

        let blocks = validators
            .iter()
            .map(|validator| factory.block(validator, predecessors.clone()))
            .collect();
        dag.push(blocks);
    }

    dag
}

fn params(n: usize, f: usize) -> ConsensusParams {
    ConsensusParams {
        wavelength: WAVELENGTH,
        n,
        f,
    }
}

/// Reference run: the same DAG delivered promptly and in order.
///
/// Adversarial schedules are compared against this to show that a hostile
/// network changes *when* nodes converge, never *what* they converge on.
fn reference_output(n: usize, f: usize, rounds: usize) -> Vec<BlockIdentity> {
    let mut network = AdversarialNetwork::equal_stake(n);
    let validators = network.validators().to_vec();
    let mut factory = BlockFactory::new();

    for round in build_rounds(&mut factory, &validators, rounds) {
        for block in &round {
            network.broadcast(block);
        }
        network.settle();
    }

    network
        .ordered_output(&validators[0], params(n, f), leader_v1)
        .expect("reference run should order cleanly")
}

// ---------------------------------------------------------------------------
// ES-mode fault bound
// ---------------------------------------------------------------------------

/// The ES bound encoded in the harness must agree with the quorum rule the
/// consensus code actually enforces.
///
/// `is_supermajority` accepts a block set with strictly more than `(n + f) / 2`
/// distinct creators. For that quorum to be reachable from honest validators
/// alone we need `q <= n - f`, which is exactly `f < n / 3`.
#[test]
fn es_fault_bound_agrees_with_the_enforced_quorum_rule() {
    let mut factory = BlockFactory::new();

    for n in 1..=21usize {
        let f = max_byzantine_faults(n);

        assert!(
            within_es_fault_bound(n, f),
            "n={n}: f={f} should be inside the ES bound"
        );
        assert!(
            !within_es_fault_bound(n, f + 1),
            "n={n}: f={} should be outside the ES bound",
            f + 1
        );

        let quorum = es_quorum_size(n, f);
        assert!(
            quorum <= n - f,
            "n={n}, f={f}: quorum {quorum} must be reachable from the {} honest validators",
            n - f
        );

        // A set of `quorum` distinct creators is a supermajority; one fewer is not.
        let creators: Vec<NodeId> = (1..=n).map(|i| node(i as u8)).collect();
        let at_quorum: HashSet<Block> = creators
            .iter()
            .take(quorum)
            .map(|creator| factory.block(creator, HashSet::new()))
            .collect();
        let below_quorum: HashSet<Block> = creators
            .iter()
            .take(quorum - 1)
            .map(|creator| factory.block(creator, HashSet::new()))
            .collect();

        assert!(
            is_supermajority(&at_quorum, n, f),
            "n={n}, f={f}: {quorum} distinct creators should form a supermajority"
        );
        assert!(
            !is_supermajority(&below_quorum, n, f),
            "n={n}, f={f}: {} distinct creators should not form a supermajority",
            quorum - 1
        );
    }
}

#[test]
fn es_fault_bound_rejects_one_third_or_more_faults() {
    // f < n/3 — the classic bound. Exactly n/3 is already too many.
    assert!(!within_es_fault_bound(3, 1));
    assert!(!within_es_fault_bound(6, 2));
    assert!(!within_es_fault_bound(9, 3));

    assert!(within_es_fault_bound(4, 1));
    assert!(within_es_fault_bound(7, 2));
    assert!(within_es_fault_bound(10, 3));

    assert_eq!(max_byzantine_faults(4), 1);
    assert_eq!(max_byzantine_faults(7), 2);
    assert_eq!(max_byzantine_faults(10), 3);
}

/// At the ES bound, a network that eventually delivers must still converge.
#[test]
fn networks_at_the_es_fault_bound_still_converge() {
    for n in [4usize, 7] {
        let f = max_byzantine_faults(n);
        assert!(within_es_fault_bound(n, f));

        let mut network = AdversarialNetwork::equal_stake(n);
        let validators = network.validators().to_vec();
        let mut factory = BlockFactory::new();

        // The last `f` validators are silent — the cheapest Byzantine
        // behaviour, and the one that most directly stresses the quorum. They
        // produce nothing at all, so the honest DAG never references them.
        let honest: Vec<NodeId> = validators[..n - f].to_vec();

        for round in build_rounds(&mut factory, &honest, 6) {
            for block in &round {
                network.broadcast(block);
            }
            network.settle();
            network
                .check_safety(params(n, f), leader_v1)
                .unwrap_or_else(|violation| panic!("n={n}, f={f}: {violation}"));
        }

        assert!(
            network.has_converged(params(n, f), leader_v1),
            "n={n}, f={f}: {} honest validators should reach a common order",
            n - f
        );
    }
}

// ---------------------------------------------------------------------------
// Delayed and out-of-order delivery
// ---------------------------------------------------------------------------

/// A lagging node never disagrees with the rest — it is only behind.
#[test]
fn delayed_delivery_keeps_safety_and_converges_once_the_backlog_drains() {
    let (n, f) = (4usize, 1usize);
    let mut network = AdversarialNetwork::equal_stake(n);
    let validators = network.validators().to_vec();
    let mut factory = BlockFactory::new();

    // Everything addressed to validator 4 is held far beyond the run.
    let laggard = validators[3].clone();
    network.delay_node(&laggard, 1_000);

    for round in build_rounds(&mut factory, &validators, 6) {
        for block in &round {
            network.broadcast(block);
        }
        network.settle();
        network.advance(1);

        network
            .check_safety(params(n, f), leader_v1)
            .unwrap_or_else(|violation| {
                panic!("safety broke while delivery was delayed: {violation}")
            });
    }

    // Liveness is delayed for the laggard, but never at the cost of safety.
    let stalled = network
        .ordered_output(&laggard, params(n, f), leader_v1)
        .expect("laggard should still order cleanly");
    assert!(
        stalled.is_empty(),
        "the delayed node should not have committed anything yet"
    );
    assert!(
        network.inflight_len() > 0,
        "the delayed node's backlog should still be in flight"
    );

    // Delivery resumes: the backlog drains and the laggard catches up exactly.
    network.deliver_everything();

    network
        .check_safety(params(n, f), leader_v1)
        .unwrap_or_else(|violation| panic!("safety broke after catch-up: {violation}"));
    assert_eq!(network.inflight_len(), 0);
    assert!(
        network.has_converged(params(n, f), leader_v1),
        "all validators should converge once the backlog drains"
    );

    let caught_up = network
        .ordered_output(&laggard, params(n, f), leader_v1)
        .expect("laggard should order cleanly after catch-up");
    assert_eq!(
        caught_up,
        reference_output(n, f, 6),
        "catching up must reproduce the prompt-delivery order exactly"
    );
}

/// Strict reverse delivery: every child arrives before its parent, so each node
/// buffers almost the entire DAG before anything can be inserted.
#[test]
fn reverse_order_delivery_converges_on_the_same_order() {
    let (n, f) = (4usize, 1usize);
    let mut network =
        AdversarialNetwork::equal_stake(n).with_delivery_order(DeliveryOrder::Reverse);
    let validators = network.validators().to_vec();
    let mut factory = BlockFactory::new();

    // Queue the whole DAG before delivering anything, so reversal spans rounds
    // rather than just reordering within one.
    for round in build_rounds(&mut factory, &validators, 6) {
        for block in &round {
            network.broadcast(block);
        }
    }
    network.settle();

    network
        .check_safety(params(n, f), leader_v1)
        .unwrap_or_else(|violation| panic!("safety broke under reverse delivery: {violation}"));

    for validator in &validators {
        assert_eq!(
            network
                .node(validator)
                .expect("validator should exist")
                .pending_len(),
            0,
            "reverse delivery should leave no block permanently buffered"
        );
    }

    assert!(network.has_converged(params(n, f), leader_v1));
    assert_eq!(
        network
            .ordered_output(&validators[0], params(n, f), leader_v1)
            .expect("should order cleanly"),
        reference_output(n, f, 6),
        "reverse delivery must not change the committed order"
    );
}

/// Randomised delivery schedules, each reproducible from its seed.
#[test]
fn randomised_delivery_schedules_all_converge_on_the_same_order() {
    let (n, f) = (4usize, 1usize);
    let expected = reference_output(n, f, 6);

    for seed in 0..12u64 {
        let mut network = AdversarialNetwork::equal_stake(n)
            .with_seed(seed)
            .with_delivery_order(DeliveryOrder::Shuffled);
        let validators = network.validators().to_vec();
        let mut factory = BlockFactory::new();

        for round in build_rounds(&mut factory, &validators, 6) {
            for block in &round {
                network.broadcast(block);
            }
            network.settle();

            network
                .check_safety(params(n, f), leader_v1)
                .unwrap_or_else(|violation| panic!("seed {seed}: safety broke: {violation}"));
        }

        assert!(
            network.has_converged(params(n, f), leader_v1),
            "seed {seed}: validators failed to converge"
        );
        assert_eq!(
            network
                .ordered_output(&validators[0], params(n, f), leader_v1)
                .expect("should order cleanly"),
            expected,
            "seed {seed}: randomised delivery changed the committed order"
        );
    }
}

// ---------------------------------------------------------------------------
// Equivocation
// ---------------------------------------------------------------------------

/// The chain axiom keeps a second, conflicting branch out of the blocklace.
#[test]
fn equivocating_branch_is_rejected_and_never_enters_the_blocklace() {
    let (n, f) = (4usize, 1usize);
    let mut network = AdversarialNetwork::equal_stake(n);
    let validators = network.validators().to_vec();
    let mut factory = BlockFactory::new();

    let equivocator = validators[0].clone();
    let (branch_a, branch_b) = factory.equivocating_pair(&equivocator, HashSet::new());
    assert_ne!(branch_a.identity, branch_b.identity);

    let victim = validators[1].clone();
    network.send_to(&branch_a, std::slice::from_ref(&victim));
    let first = network.deliver_ready();
    assert_eq!(first, vec![(victim.clone(), DeliveryOutcome::Inserted)]);

    network.send_to(&branch_b, std::slice::from_ref(&victim));
    let second = network.deliver_ready();
    assert_eq!(second.len(), 1);
    match &second[0].1 {
        DeliveryOutcome::Rejected(errors) => assert!(
            errors
                .iter()
                .any(|error| matches!(error, InvalidBlock::Equivocation { .. })),
            "the conflicting branch should be rejected as an equivocation, got {errors:?}"
        ),
        other => panic!("expected the second branch to be rejected, got {other:?}"),
    }

    let blocklace = network.blocklace(&victim).expect("victim should exist");
    assert!(blocklace.satisfies_chain_axiom(&equivocator));
    assert!(blocklace.content(&branch_a.identity).is_some());
    assert!(
        blocklace.content(&branch_b.identity).is_none(),
        "the rejected branch must not be reachable in the blocklace"
    );

    network
        .check_safety(params(n, f), leader_v1)
        .unwrap_or_else(|violation| panic!("safety broke after equivocation: {violation}"));
}

/// Regression: a block that arrives more than one round ahead of its causal
/// history must be buffered, never discarded as an equivocation.
///
/// The chain-axiom check establishes comparability by tracing a path through
/// the block's predecessors in the local blocklace. While those predecessors
/// are missing the path cannot be traced, so an honest block used to be
/// reported as an equivocation alongside the missing-predecessor error — and
/// because the buffer only retains blocks whose errors are *all* missing
/// predecessors, it was dropped permanently and never retried.
///
/// Delivery orders that skip a round are routine under delay, reordering, and
/// partition healing, so this cost the network convergence outright.
#[test]
fn block_arriving_ahead_of_its_history_is_buffered_not_discarded() {
    let (n, f) = (4usize, 1usize);
    let mut network = AdversarialNetwork::equal_stake(n);
    let validators = network.validators().to_vec();
    let mut factory = BlockFactory::new();
    let dag = build_rounds(&mut factory, &validators, 3);

    let creator = validators[0].clone();
    let observer = validators[1].clone();

    // The observer knows the creator's round-0 block, and nothing else.
    let round_zero = dag[0]
        .iter()
        .find(|block| block.identity.creator == creator)
        .expect("creator should have a round-0 block")
        .clone();
    network.send_to(&round_zero, std::slice::from_ref(&observer));
    assert_eq!(
        network.deliver_ready(),
        vec![(observer.clone(), DeliveryOutcome::Inserted)]
    );

    // The creator's round-2 block now arrives, skipping round 1 entirely. Its
    // predecessors are missing, so the chain axiom is not yet decidable.
    let round_two = dag[2]
        .iter()
        .find(|block| block.identity.creator == creator)
        .expect("creator should have a round-2 block")
        .clone();
    network.send_to(&round_two, std::slice::from_ref(&observer));

    let outcome = network.deliver_ready();
    assert_eq!(
        outcome,
        vec![(observer.clone(), DeliveryOutcome::Buffered)],
        "a block ahead of its history must be buffered for retry, not rejected"
    );

    // Once the skipped history lands, the buffered block resolves on its own.
    for block in dag[0].iter().chain(dag[1].iter()) {
        network.send_to(block, std::slice::from_ref(&observer));
    }
    network.settle();

    let node = network.node(&observer).expect("observer should exist");
    assert_eq!(node.pending_len(), 0);
    assert!(
        node.knows_block(&round_two.identity),
        "the buffered block should be inserted once its history arrives"
    );

    network
        .check_safety(params(n, f), leader_v1)
        .unwrap_or_else(|violation| panic!("safety broke: {violation}"));
}

/// An equivocating leader splitting the network cannot get two conflicting
/// orders finalised.
///
/// Validator 1 leads every wave. It equivocates at the wave-1 leader round,
/// showing one branch to `{1, 2}` and a conflicting branch to `{3, 4}`, and the
/// honest validators build on whichever branch they saw. Neither half is a
/// supermajority, so the attack can at most stall wave 1 — it must never
/// produce two different committed orders.
#[test]
fn equivocating_leader_cannot_finalise_two_conflicting_orders() {
    let (n, f) = (4usize, 1usize);
    let mut network = AdversarialNetwork::equal_stake(n);
    let validators = network.validators().to_vec();
    let mut factory = BlockFactory::new();

    let leader = validators[0].clone();
    let left_half = vec![validators[0].clone(), validators[1].clone()];
    let right_half = vec![validators[2].clone(), validators[3].clone()];

    // Rounds 0..=2 are honest: wave 0 finalises normally.
    let base = build_rounds(&mut factory, &validators, 3);
    for round in &base {
        for block in round {
            network.broadcast(block);
        }
        network.settle();
    }

    let wave_zero_output = network
        .ordered_output(&validators[0], params(n, f), leader_v1)
        .expect("wave 0 should order cleanly");
    assert!(
        !wave_zero_output.is_empty(),
        "wave 0 should finalise before the attack starts"
    );

    // Round 3 is the wave-1 leader round. The leader equivocates.
    let round_two: HashSet<BlockIdentity> =
        base[2].iter().map(|block| block.identity.clone()).collect();
    let (branch_left, branch_right) = factory.equivocating_pair(&leader, round_two.clone());
    network.send_to(&branch_left, &left_half);
    network.send_to(&branch_right, &right_half);

    // The honest validators produce their round-3 blocks as usual.
    let honest_round_three: Vec<Block> = validators[1..]
        .iter()
        .map(|validator| factory.block(validator, round_two.clone()))
        .collect();
    for block in &honest_round_three {
        network.broadcast(block);
    }
    network.settle();

    network
        .check_safety(params(n, f), leader_v1)
        .unwrap_or_else(|violation| {
            panic!("safety broke while the leader equivocated: {violation}")
        });

    // Rounds 4 and 5: each half extends the branch it saw.
    let mut previous_left: HashSet<BlockIdentity> = honest_round_three
        .iter()
        .map(|block| block.identity.clone())
        .collect();
    let mut previous_right = previous_left.clone();
    previous_left.insert(branch_left.identity.clone());
    previous_right.insert(branch_right.identity.clone());

    for _ in 4..6 {
        let left_blocks: Vec<Block> = [validators[1].clone()]
            .iter()
            .map(|validator| factory.block(validator, previous_left.clone()))
            .collect();
        let right_blocks: Vec<Block> = validators[2..]
            .iter()
            .map(|validator| factory.block(validator, previous_right.clone()))
            .collect();

        for block in &left_blocks {
            network.send_to(block, &left_half);
        }
        for block in &right_blocks {
            network.send_to(block, &right_half);
        }
        network.settle();

        network
            .check_safety(params(n, f), leader_v1)
            .unwrap_or_else(|violation| {
                panic!("safety broke while halves extended conflicting branches: {violation}")
            });

        previous_left = left_blocks
            .iter()
            .map(|block| block.identity.clone())
            .collect();
        previous_right = right_blocks
            .iter()
            .map(|block| block.identity.clone())
            .collect();
    }

    // No half reached a supermajority, so no node may have committed anything
    // beyond the honestly finalised wave-0 prefix.
    let outputs = network.all_ordered_outputs(params(n, f), leader_v1);
    check_prefix_consistency(&outputs)
        .unwrap_or_else(|violation| panic!("equivocation split the committed order: {violation}"));

    for (id, output) in &outputs {
        assert_eq!(
            common_prefix(output, &wave_zero_output),
            wave_zero_output.len(),
            "{id:?} should still extend the wave-0 prefix"
        );
        assert!(
            !(output.contains(&branch_left.identity) && output.contains(&branch_right.identity)),
            "{id:?} committed both equivocating branches"
        );
    }

    // Both branches reach every node once the split heals. The chain axiom
    // admits only one of them, and the committed order is unaffected either way.
    network.send_to(&branch_left, &right_half);
    network.send_to(&branch_right, &left_half);
    network.deliver_everything();

    network
        .check_safety(params(n, f), leader_v1)
        .unwrap_or_else(|violation| panic!("safety broke after the split healed: {violation}"));

    for validator in &validators {
        let blocklace = network
            .blocklace(validator)
            .expect("validator should exist");
        assert!(
            blocklace.satisfies_chain_axiom(&leader),
            "{validator:?} accepted both equivocating branches"
        );
    }

    let healed = network.all_ordered_outputs(params(n, f), leader_v1);
    for (id, output) in &healed {
        assert_eq!(
            common_prefix(output, &wave_zero_output),
            wave_zero_output.len(),
            "{id:?} lost the wave-0 prefix after healing"
        );
    }
}

fn common_prefix(left: &[BlockIdentity], right: &[BlockIdentity]) -> usize {
    left.iter()
        .zip(right.iter())
        .take_while(|(a, b)| a == b)
        .count()
}

// ---------------------------------------------------------------------------
// Partitions
// ---------------------------------------------------------------------------

/// A partition with no quorum on either side stalls progress without ever
/// breaking safety, and the network converges once it heals.
#[test]
fn partition_stalls_progress_then_converges_after_healing() {
    let (n, f) = (4usize, 1usize);
    let mut network = AdversarialNetwork::equal_stake(n);
    let validators = network.validators().to_vec();
    let mut factory = BlockFactory::new();
    let dag = build_rounds(&mut factory, &validators, 6);

    // Rounds 0..=2: healthy network, wave 0 finalises.
    for round in dag.iter().take(3) {
        for block in round {
            network.broadcast(block);
        }
        network.settle();
    }

    let before_partition = network.all_ordered_outputs(params(n, f), leader_v1);
    let committed_before = before_partition[&validators[0]].clone();
    assert!(!committed_before.is_empty());

    // Split 2 / 2. The ES quorum is 3 distinct creators, so neither side can
    // finalise anything on its own.
    assert_eq!(es_quorum_size(n, f), 3);
    network.partition(vec![
        vec![validators[0].clone(), validators[1].clone()],
        vec![validators[2].clone(), validators[3].clone()],
    ]);
    assert!(network.is_partitioned());

    for round in dag.iter().skip(3) {
        for block in round {
            network.broadcast(block);
        }
        network.settle();

        network
            .check_safety(params(n, f), leader_v1)
            .unwrap_or_else(|violation| panic!("safety broke during the partition: {violation}"));
    }

    // Liveness is gone while the partition holds: nothing new is committed.
    let during_partition = network.all_ordered_outputs(params(n, f), leader_v1);
    for (id, output) in &during_partition {
        assert_eq!(
            output, &before_partition[id],
            "{id:?} committed new blocks despite having no quorum"
        );
    }
    assert!(
        network.inflight_len() > 0,
        "the partition should be holding cross-group blocks"
    );

    // Heal, deliver the backlog, and the network converges.
    network.heal();
    assert!(!network.is_partitioned());
    network.deliver_everything();

    network
        .check_safety(params(n, f), leader_v1)
        .unwrap_or_else(|violation| panic!("safety broke after healing: {violation}"));
    assert_eq!(network.inflight_len(), 0);
    assert!(
        network.has_converged(params(n, f), leader_v1),
        "the network should converge once the partition heals"
    );

    let healed = network
        .ordered_output(&validators[0], params(n, f), leader_v1)
        .expect("should order cleanly after healing");
    assert!(
        healed.len() > committed_before.len(),
        "healing should let the network make progress again"
    );
    assert_eq!(
        healed,
        reference_output(n, f, 6),
        "a healed partition must reproduce the prompt-delivery order"
    );
}

/// A partition that leaves a quorum on one side lets that side keep going,
/// and the minority still catches up to exactly the same order.
#[test]
fn partition_with_a_quorum_majority_keeps_progressing_and_the_minority_catches_up() {
    let (n, f) = (4usize, 1usize);
    let mut network = AdversarialNetwork::equal_stake(n);
    let validators = network.validators().to_vec();
    let mut factory = BlockFactory::new();
    let dag = build_rounds(&mut factory, &validators, 6);

    let minority = validators[3].clone();
    network.partition(vec![
        vec![
            validators[0].clone(),
            validators[1].clone(),
            validators[2].clone(),
        ],
        vec![minority.clone()],
    ]);

    for round in &dag {
        for block in round {
            network.broadcast(block);
        }
        network.settle();

        network
            .check_safety(params(n, f), leader_v1)
            .unwrap_or_else(|violation| {
                panic!("safety broke during the majority partition: {violation}")
            });
    }

    // The isolated node still holds its own blocks but cannot order anything.
    let isolated = network
        .ordered_output(&minority, params(n, f), leader_v1)
        .expect("isolated node should order cleanly");
    assert!(
        isolated.is_empty(),
        "an isolated validator should not commit on its own"
    );

    network.heal();
    network.deliver_everything();

    network
        .check_safety(params(n, f), leader_v1)
        .unwrap_or_else(|violation| panic!("safety broke after healing: {violation}"));
    assert!(network.has_converged(params(n, f), leader_v1));
    assert_eq!(
        network
            .ordered_output(&minority, params(n, f), leader_v1)
            .expect("minority should order cleanly after healing"),
        reference_output(n, f, 6),
        "the minority must catch up to the majority's order exactly"
    );
}

/// Partitions combined with a randomised delivery schedule.
#[test]
fn partition_healing_converges_under_randomised_delivery() {
    let (n, f) = (4usize, 1usize);
    let expected = reference_output(n, f, 6);

    for seed in 0..8u64 {
        let mut network = AdversarialNetwork::equal_stake(n)
            .with_seed(seed)
            .with_delivery_order(DeliveryOrder::Shuffled);
        let validators = network.validators().to_vec();
        let mut factory = BlockFactory::new();
        let dag = build_rounds(&mut factory, &validators, 6);

        network.partition(vec![
            vec![validators[0].clone(), validators[1].clone()],
            vec![validators[2].clone(), validators[3].clone()],
        ]);

        for (index, round) in dag.iter().enumerate() {
            // Heal halfway through, then keep delivering under the same
            // randomised schedule.
            if index == 3 {
                network.heal();
            }
            for block in round {
                network.broadcast(block);
            }
            network.settle();

            network
                .check_safety(params(n, f), leader_v1)
                .unwrap_or_else(|violation| panic!("seed {seed}: safety broke: {violation}"));
        }

        network.deliver_everything();
        network
            .check_safety(params(n, f), leader_v1)
            .unwrap_or_else(|violation| {
                panic!("seed {seed}: safety broke after healing: {violation}")
            });

        assert!(
            network.has_converged(params(n, f), leader_v1),
            "seed {seed}: failed to converge after healing"
        );
        assert_eq!(
            network
                .ordered_output(&validators[0], params(n, f), leader_v1)
                .expect("should order cleanly"),
            expected,
            "seed {seed}: healed partition produced a different order"
        );
    }
}

// ---------------------------------------------------------------------------
// Invariant checker self-tests
// ---------------------------------------------------------------------------

/// The safety checker must actually catch divergence, or the tests above prove
/// nothing.
#[test]
fn prefix_consistency_check_detects_divergence() {
    let mut factory = BlockFactory::new();
    let shared = factory.block(&node(1), HashSet::new()).identity;
    let left_only = factory.block(&node(2), HashSet::new()).identity;
    let right_only = factory.block(&node(3), HashSet::new()).identity;

    let agreeing = [
        (node(1), vec![shared.clone()]),
        (node(2), vec![shared.clone(), left_only.clone()]),
    ]
    .into_iter()
    .collect();
    assert!(check_prefix_consistency(&agreeing).is_ok());

    let diverging = [
        (node(1), vec![shared.clone(), left_only]),
        (node(2), vec![shared, right_only]),
    ]
    .into_iter()
    .collect();
    assert_eq!(
        check_prefix_consistency(&diverging),
        Err(Box::new(SafetyViolation::Divergence {
            left: node(1),
            right: node(2),
            index: 1,
        }))
    );
}
