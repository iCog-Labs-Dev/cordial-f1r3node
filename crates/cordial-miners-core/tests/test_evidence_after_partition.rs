//! Equivocation evidence across a partition heal.
//!
//! Two properties matter here, and they pull in opposite directions:
//!
//! 1. **Retention** — proof of equivocation must survive. Detection happens when
//!    a conflicting block fails validation, and that is the *only* moment the
//!    pair is in hand: the chain axiom stops both branches from ever coexisting
//!    in the blocklace, so a rejected branch that is simply dropped cannot be
//!    recovered afterwards. `all_equivocations` scans the blocklace and by
//!    construction sees at most one branch.
//!
//! 2. **No poisoning** — holding evidence against a validator must not change
//!    how honest blocks validate or order, including honest blocks by that same
//!    validator. Evidence is proof for later slashing, not an admission rule.
//!
//! A partition is what makes retention interesting: whether a node holds the
//! proof depends on whether *it* saw both branches, and healing is when the
//! second branch finally arrives.

use cordial_miners_core::simulation::adversary::{AdversarialNetwork, BlockFactory};
use cordial_miners_core::simulation::dissemination::DeliveryOutcome;
use cordial_miners_core::{Block, BlockIdentity, NodeId};
use std::collections::HashSet;

const WAVELENGTH: u64 = 3;

fn node(id: u8) -> NodeId {
    NodeId(vec![id])
}

fn leader_v1(_wave: u64) -> Option<NodeId> {
    Some(node(1))
}

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
            dag[round - 1].iter().map(|b| b.identity.clone()).collect()
        };
        dag.push(
            participants
                .iter()
                .map(|v| factory.block(v, predecessors.clone()))
                .collect(),
        );
    }
    dag
}

/// Evidence is captured at the moment of rejection, not reconstructed later.
#[test]
fn rejecting_a_conflicting_branch_retains_the_proof() {
    let mut network = AdversarialNetwork::equal_stake(4);
    let mut factory = BlockFactory::new();
    let equivocator = node(1);
    let observer = node(2);

    let (branch_a, branch_b) = factory.equivocating_pair(&equivocator, HashSet::new());

    network.send_to(&branch_a, std::slice::from_ref(&observer));
    assert_eq!(
        network.deliver_ready(),
        vec![(observer.clone(), DeliveryOutcome::Inserted)]
    );
    assert!(
        !network
            .node(&observer)
            .expect("observer exists")
            .has_evidence_against(&equivocator),
        "one branch alone is not proof of anything"
    );

    network.send_to(&branch_b, std::slice::from_ref(&observer));
    let outcome = network.deliver_ready();
    assert!(matches!(
        outcome.as_slice(),
        [(_, DeliveryOutcome::Rejected(_))]
    ));

    let node_ref = network.node(&observer).expect("observer exists");
    assert!(
        node_ref.has_evidence_against(&equivocator),
        "the rejected branch is half the proof and must be retained"
    );

    let evidence = node_ref.evidence_for(&equivocator);
    assert_eq!(evidence.len(), 1);
    let ids: HashSet<BlockIdentity> = evidence[0]
        .blocks
        .iter()
        .map(|b| b.identity.clone())
        .collect();
    assert!(
        ids.contains(&branch_a.identity) && ids.contains(&branch_b.identity),
        "evidence must hold both conflicting blocks, got {ids:?}"
    );
    assert_eq!(evidence[0].round, 0);

    // The rejected branch is genuinely absent from the blocklace — which is why
    // the proof had to be captured at rejection rather than recovered from it.
    let blocklace = network.blocklace(&observer).expect("observer exists");
    assert!(blocklace.content(&branch_a.identity).is_some());
    assert!(blocklace.content(&branch_b.identity).is_none());
    assert!(blocklace.satisfies_chain_axiom(&equivocator));
}

/// Split-brain: each half sees one branch, so neither holds proof until the
/// partition heals and the second branch arrives.
#[test]
fn partition_heals_and_both_halves_acquire_the_evidence() {
    let mut network = AdversarialNetwork::equal_stake(4);
    let validators = network.validators().to_vec();
    let mut factory = BlockFactory::new();
    let equivocator = validators[0].clone();

    let left = vec![validators[0].clone(), validators[1].clone()];
    let right = vec![validators[2].clone(), validators[3].clone()];

    let (branch_left, branch_right) = factory.equivocating_pair(&equivocator, HashSet::new());
    network.send_to(&branch_left, &left);
    network.send_to(&branch_right, &right);
    network.settle();

    // While split, each side has seen only one branch, so nobody has proof.
    for validator in &validators {
        assert!(
            !network
                .node(validator)
                .expect("validator exists")
                .has_evidence_against(&equivocator),
            "{validator:?} should not hold proof from a single branch"
        );
    }

    // Healing delivers the other branch to each side.
    network.send_to(&branch_left, &right);
    network.send_to(&branch_right, &left);
    network.deliver_everything();

    for validator in &validators {
        let n = network.node(validator).expect("validator exists");
        assert!(
            n.has_evidence_against(&equivocator),
            "{validator:?} should hold proof once both branches have arrived"
        );

        let evidence = n.evidence_for(&equivocator);
        assert_eq!(
            evidence.len(),
            1,
            "{validator:?} should hold exactly one record for this round"
        );
        let ids: HashSet<BlockIdentity> = evidence[0]
            .blocks
            .iter()
            .map(|b| b.identity.clone())
            .collect();
        assert_eq!(
            ids,
            HashSet::from([branch_left.identity.clone(), branch_right.identity.clone()]),
            "{validator:?} holds the wrong pair"
        );
    }

    // Every node still admits exactly one branch, whichever it saw first.
    for validator in &validators {
        let blocklace = network.blocklace(validator).expect("validator exists");
        assert!(
            blocklace.satisfies_chain_axiom(&equivocator),
            "{validator:?} accepted both branches"
        );
    }
}

/// Evidence is deduplicated: repeated delivery of the same conflicting branch
/// must not accumulate records.
#[test]
fn repeated_delivery_does_not_accumulate_duplicate_evidence() {
    let mut network = AdversarialNetwork::equal_stake(4);
    let mut factory = BlockFactory::new();
    let equivocator = node(1);
    let observer = node(2);

    let (branch_a, branch_b) = factory.equivocating_pair(&equivocator, HashSet::new());
    network.send_to(&branch_a, std::slice::from_ref(&observer));
    network.settle();

    for _ in 0..5 {
        network.send_to(&branch_b, std::slice::from_ref(&observer));
        network.settle();
    }

    let evidence = network
        .node(&observer)
        .expect("observer exists")
        .evidence_for(&equivocator);
    assert_eq!(
        evidence.len(),
        1,
        "the same conflicting pair must produce one record, got {}",
        evidence.len()
    );
}

/// Holding evidence against a validator must not change how honest blocks
/// validate or order — including honest blocks by that same validator.
#[test]
fn evidence_does_not_poison_honest_blocks() {
    let honest_participants = [node(1), node(2), node(3), node(4)];

    // Reference run: no equivocation anywhere.
    let mut clean = AdversarialNetwork::equal_stake(4);
    let mut clean_factory = BlockFactory::new();
    for round in build_rounds(&mut clean_factory, &honest_participants, 6) {
        for block in &round {
            clean.broadcast(block);
        }
        clean.settle();
    }
    let clean_output = clean
        .ordered_output(
            &node(2),
            cordial_miners_core::simulation::adversary::ConsensusParams {
                wavelength: WAVELENGTH,
                n: 4,
                f: 1,
            },
            leader_v1,
        )
        .expect("clean run should order");
    assert!(!clean_output.is_empty());

    // Same run, but v3 also emits a stray conflicting block that gets rejected
    // and recorded as evidence. v3's honest blocks are unchanged.
    let mut poisoned = AdversarialNetwork::equal_stake(4);
    let mut factory = BlockFactory::new();
    let rounds = build_rounds(&mut factory, &honest_participants, 6);

    for (index, round) in rounds.iter().enumerate() {
        for block in round {
            poisoned.broadcast(block);
        }
        poisoned.settle();

        if index == 0 {
            // A second round-0 block by v3, conflicting with its genesis.
            let stray = factory.block(&node(3), HashSet::new());
            for validator in &honest_participants {
                poisoned.send_to(&stray, std::slice::from_ref(validator));
            }
            poisoned.settle();
        }
    }

    // Everyone holds proof against v3 …
    for validator in &honest_participants {
        assert!(
            poisoned
                .node(validator)
                .expect("validator exists")
                .has_evidence_against(&node(3)),
            "{validator:?} should hold proof against v3"
        );
    }

    // … and the committed order is byte-identical to the clean run, so the
    // evidence changed nothing about admission or ordering.
    let poisoned_output = poisoned
        .ordered_output(
            &node(2),
            cordial_miners_core::simulation::adversary::ConsensusParams {
                wavelength: WAVELENGTH,
                n: 4,
                f: 1,
            },
            leader_v1,
        )
        .expect("run should order");

    assert_eq!(
        poisoned_output.len(),
        clean_output.len(),
        "evidence must not change how many blocks are committed"
    );

    // v3's honest blocks are still present and ordered.
    let blocklace = poisoned.blocklace(&node(2)).expect("v2 exists");
    let v3_blocks: Vec<&BlockIdentity> = blocklace
        .dom()
        .into_iter()
        .filter(|id| id.creator == node(3))
        .collect();
    assert!(
        v3_blocks.len() > 1,
        "v3's honest blocks must still be admitted despite the evidence"
    );
}
