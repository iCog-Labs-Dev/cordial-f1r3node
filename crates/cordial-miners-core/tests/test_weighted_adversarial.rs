//! Weighted adversarial simulation: quorum behaviour under skewed stake.
//!
//! Equal-stake simulation cannot exercise the weighted path meaningfully — when
//! every validator holds the same stake, a stake supermajority and a count
//! supermajority are the same set of nodes, so an implementation that counted
//! validators instead of weighing them would pass every test.
//!
//! These tests skew stake so the two disagree, in both directions:
//!
//! - a **count majority that is not a stake supermajority** must not finalise
//! - a **count minority that is a stake supermajority** must finalise
//!
//! One control matters throughout: **the elected leader stays active**. If the
//! leader is among the validators being silenced or isolated, finality fails
//! merely because no leader block exists, and the test says nothing about stake
//! at all. The stake layout below therefore gives the leader the *smallest*
//! holding, so it can always be present without itself supplying the quorum.

use cordial_miners_core::simulation::adversary::{
    AdversarialNetwork, BlockFactory, ConsensusParams,
};
use cordial_miners_core::{Block, BlockIdentity, NodeId};
use std::collections::HashSet;

const WAVELENGTH: u64 = 3;

fn node(id: u8) -> NodeId {
    NodeId(vec![id])
}

/// v1 leads every wave and holds the least stake, so it can stay active in every
/// scenario without being the reason a quorum is reached.
fn leader_v1(_wave: u64) -> Option<NodeId> {
    Some(node(1))
}

/// v2 leads — used for the high-stake equivocation case.
fn leader_v2(_wave: u64) -> Option<NodeId> {
    Some(node(2))
}

/// Stake: v1=10 (leader), v2=40, v3=25, v4=25. Total 100, so a strict two-thirds
/// majority needs more than 66.
///
/// - without v2: {v1,v3,v4} = 60 → 3 of 4 validators, *not* a stake supermajority
/// - without v4: {v1,v2,v3} = 75 → 3 of 4 validators, *is* a stake supermajority
///
/// Same validator count, opposite verdicts. That contrast is the whole point.
fn skewed_network() -> AdversarialNetwork {
    AdversarialNetwork::weighted(&[(node(1), 10), (node(2), 40), (node(3), 25), (node(4), 25)])
}

/// Stake so concentrated that one validator clears two thirds: v1=70, rest 10 each.
fn dominant_network() -> AdversarialNetwork {
    AdversarialNetwork::weighted(&[(node(1), 70), (node(2), 10), (node(3), 10), (node(4), 10)])
}

/// Dense DAG over `participants` only, so absent validators are never referenced
/// and their missing blocks cannot stall the others behind the closure axiom.
fn build_rounds(
    factory: &mut BlockFactory,
    participants: &[NodeId],
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
        let blocks = participants
            .iter()
            .map(|v| factory.block(v, predecessors.clone()))
            .collect();
        dag.push(blocks);
    }
    dag
}

fn run<F>(network: &mut AdversarialNetwork, participants: &[NodeId], rounds: usize, leader: F)
where
    F: Fn(u64) -> Option<NodeId> + Copy,
{
    let mut factory = BlockFactory::new();
    for round in build_rounds(&mut factory, participants, rounds) {
        for block in &round {
            network.broadcast(block);
        }
        network.settle();
        network
            .check_weighted_safety(WAVELENGTH, leader)
            .unwrap_or_else(|violation| panic!("weighted safety broke: {violation}"));
    }
}

/// The stake arithmetic every other test here depends on, stated rather than
/// assumed.
#[test]
fn skewed_stake_separates_count_from_weight() {
    let network = skewed_network();
    assert_eq!(network.total_stake(), 100);
    assert_eq!(network.stake_of(&node(1)), 10, "the leader holds the least");

    // 3 of 4 validators, 60 stake: a count majority, not a stake supermajority.
    assert!(!network.is_stake_supermajority(&[node(1), node(3), node(4)]));
    // 3 of 4 validators, 75 stake: both.
    assert!(network.is_stake_supermajority(&[node(1), node(2), node(3)]));

    // One validator can be a supermajority when stake is concentrated enough.
    let dominant = dominant_network();
    assert!(dominant.is_stake_supermajority(&[node(1)]));
    assert!(!dominant.is_stake_supermajority(&[node(2), node(3), node(4)]));
}

/// A silent high-stake validator blocks finality even though most validators are
/// active — and the leader is one of the active ones, so a leader block exists.
#[test]
fn silent_high_stake_validator_prevents_weighted_finality() {
    let mut network = skewed_network();
    let active = [node(1), node(3), node(4)];

    assert!(
        active.contains(&node(1)),
        "control: the leader must be active, or finality fails for lack of a leader"
    );
    assert!(
        !network.is_stake_supermajority(&active),
        "precondition: a count majority that is not a stake supermajority"
    );

    run(&mut network, &active, 6, leader_v1);

    let leaders = network.all_weighted_final_leader_waves(WAVELENGTH, leader_v1);
    assert!(
        leaders.values().all(Option::is_none),
        "60 of 100 stake must not finalise, however many validators that is: {leaders:?}"
    );
    for (id, output) in network.all_weighted_ordered_outputs(WAVELENGTH, leader_v1) {
        assert!(
            output.is_empty(),
            "{id:?} committed without a stake supermajority"
        );
    }

    // The discriminating check: the *count-based* path finalises this very same
    // DAG, because three of four creators is a count supermajority. So the
    // refusal above is attributable to the weighting and nothing else — if
    // weighted finality were counting validators, this test would pass
    // vacuously.
    let count_params = ConsensusParams {
        wavelength: WAVELENGTH,
        n: 4,
        f: 1,
    };
    let count_leaders = network.all_final_leader_waves(count_params, leader_v1);
    assert!(
        count_leaders.values().any(Option::is_some),
        "sanity: the count-based path should finalise this DAG, otherwise the \
         weighted refusal proves nothing: {count_leaders:?}"
    );
}

/// Same validator count, but the silent one is low-stake: 75 of 100 remains, so
/// finality proceeds. Paired with the test above, this isolates stake as the only
/// difference between the two outcomes.
#[test]
fn silent_low_stake_validator_does_not_prevent_weighted_finality() {
    let mut network = skewed_network();
    let active = [node(1), node(2), node(3)];

    assert!(
        network.is_stake_supermajority(&active),
        "precondition: same 3-of-4 count as the previous test, but 75 stake"
    );

    run(&mut network, &active, 6, leader_v1);

    let leaders = network.all_weighted_final_leader_waves(WAVELENGTH, leader_v1);
    assert!(
        active
            .iter()
            .all(|id| leaders.get(id).is_some_and(Option::is_some)),
        "75 of 100 stake should finalise: {leaders:?}"
    );
}

/// A count *minority* holding a stake supermajority does finalise.
#[test]
fn stake_supermajority_finalises_even_as_a_count_minority() {
    let mut network = dominant_network();
    let active = [node(1)];
    assert!(network.is_stake_supermajority(&active));

    run(&mut network, &active, 6, leader_v1);

    let leaders = network.all_weighted_final_leader_waves(WAVELENGTH, leader_v1);
    assert!(
        leaders.values().any(Option::is_some),
        "a 70% stake holder is a supermajority and must be able to finalise: {leaders:?}"
    );
}

/// With the full skewed set active, the weighted path finalises and converges.
#[test]
fn full_skewed_set_converges_on_the_weighted_order() {
    let mut network = skewed_network();
    let all: Vec<NodeId> = network.validators().to_vec();
    assert!(network.is_stake_supermajority(&all));

    run(&mut network, &all, 6, leader_v1);

    assert!(
        network.has_converged_weighted(WAVELENGTH, leader_v1),
        "the whole validator set is a supermajority and should converge"
    );
}

/// Which side of a partition progresses is decided by stake, not by how many
/// validators are on it.
///
/// Both halves use the same 1-vs-3 split shape with the leader in the majority.
/// Isolating the 25-stake validator leaves 75, which finalises; isolating the
/// 40-stake validator leaves 60, which does not.
///
/// This uses `partition` rather than `delay_node` deliberately: a delay starves a
/// node's own inbox but does nothing to stop its blocks reaching everyone else,
/// so the remaining validators still see its stake and finalise normally.
/// Removing a validator's stake from other nodes' *views* is what a partition
/// does.
#[test]
fn which_partition_side_progresses_depends_on_stake_not_count() {
    // Isolating the 25-stake validator: the other three hold 75.
    let mut low = skewed_network();
    let majority_low = vec![node(1), node(2), node(3)];
    let isolated_low = node(4);
    assert!(low.is_stake_supermajority(&majority_low));
    low.partition(vec![majority_low.clone(), vec![isolated_low.clone()]]);
    run(&mut low, &majority_low, 6, leader_v1);

    let low_leaders = low.all_weighted_final_leader_waves(WAVELENGTH, leader_v1);
    assert!(
        majority_low
            .iter()
            .all(|id| low_leaders.get(id).is_some_and(Option::is_some)),
        "75 of 100 stake should finalise: {low_leaders:?}"
    );
    assert!(
        low_leaders.get(&isolated_low).is_some_and(Option::is_none),
        "the isolated validator sees nothing and cannot finalise"
    );

    // Isolating the 40-stake validator: the other three hold only 60.
    let mut high = skewed_network();
    let majority_high = vec![node(1), node(3), node(4)];
    let isolated_high = node(2);
    assert!(
        !high.is_stake_supermajority(&majority_high),
        "precondition: same 3-validator count, but not a stake supermajority"
    );
    high.partition(vec![majority_high.clone(), vec![isolated_high.clone()]]);
    run(&mut high, &majority_high, 6, leader_v1);

    let high_leaders = high.all_weighted_final_leader_waves(WAVELENGTH, leader_v1);
    assert!(
        high_leaders.values().all(Option::is_none),
        "60 of 100 stake must not finalise despite being 3 validators of 4: {high_leaders:?}"
    );

    high.heal();
    high.deliver_everything();
    high.check_weighted_safety(WAVELENGTH, leader_v1)
        .unwrap_or_else(|violation| panic!("weighted safety broke after healing: {violation}"));
}

/// A high-stake equivocating leader cannot get two conflicting weighted orders
/// finalised.
///
/// The leader here is v2, the 40-stake validator, so this is genuinely high-stake
/// equivocation rather than a low-stake leader misbehaving.
#[test]
fn equivocating_high_stake_leader_cannot_split_the_weighted_order() {
    let mut network = skewed_network();
    let validators = network.validators().to_vec();
    let equivocator = node(2);
    let mut factory = BlockFactory::new();

    let left = vec![node(1), node(2)];
    let right = vec![node(3), node(4)];

    // Honest round 0 from everyone.
    let round0 = build_rounds(&mut factory, &validators, 1);
    for block in &round0[0] {
        network.broadcast(block);
    }
    network.settle();

    let round0_ids: HashSet<BlockIdentity> = round0[0]
        .iter()
        .map(|block| block.identity.clone())
        .collect();

    // The high-stake leader shows a different branch to each half.
    let (branch_left, branch_right) = factory.equivocating_pair(&equivocator, round0_ids.clone());
    network.send_to(&branch_left, &left);
    network.send_to(&branch_right, &right);

    let honest: Vec<NodeId> = validators
        .iter()
        .filter(|v| **v != equivocator)
        .cloned()
        .collect();
    for _ in 0..4 {
        let blocks: Vec<Block> = honest
            .iter()
            .map(|v| factory.block(v, round0_ids.clone()))
            .collect();
        for block in &blocks {
            network.broadcast(block);
        }
        network.settle();
        network
            .check_weighted_safety(WAVELENGTH, leader_v2)
            .unwrap_or_else(|violation| {
                panic!("weighted safety broke during equivocation: {violation}")
            });
    }

    let outputs = network.all_weighted_ordered_outputs(WAVELENGTH, leader_v2);
    for (id, output) in &outputs {
        assert!(
            !(output.contains(&branch_left.identity) && output.contains(&branch_right.identity)),
            "{id:?} committed both branches of the equivocating leader"
        );
    }

    // Both branches reach everyone; the chain axiom admits only one.
    network.send_to(&branch_left, &right);
    network.send_to(&branch_right, &left);
    network.deliver_everything();
    network
        .check_weighted_safety(WAVELENGTH, leader_v2)
        .unwrap_or_else(|violation| panic!("weighted safety broke after healing: {violation}"));

    for validator in &validators {
        let blocklace = network.blocklace(validator).expect("validator exists");
        assert!(
            blocklace.satisfies_chain_axiom(&equivocator),
            "{validator:?} accepted both branches of the equivocating leader"
        );
    }
}
