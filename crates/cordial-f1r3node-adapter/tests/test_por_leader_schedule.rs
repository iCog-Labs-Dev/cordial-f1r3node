use std::collections::HashMap;

use cordial_f1r3node_adapter::por::{LeaderSchedule, LeaderScheduleError, LeaderSelectionPolicy};
use cordial_miners_core::NodeId;

fn node(id: u8) -> NodeId {
    NodeId(vec![id])
}

fn weights(entries: &[(u8, u64)]) -> HashMap<NodeId, u64> {
    entries
        .iter()
        .map(|(id, weight)| (node(*id), *weight))
        .collect()
}

#[test]
fn round_robin_matches_canonical_authorized_validator_order() {
    let schedule = LeaderSchedule::from_weights(
        LeaderSelectionPolicy::RoundRobin,
        4,
        &weights(&[(3, 0), (1, 60), (2, 40)]),
    )
    .unwrap();

    assert_eq!(schedule.leaders(), &[node(1), node(2), node(3)]);
    assert_eq!(schedule.leader_for_wave(3), None);
    assert_eq!(schedule.leader_for_wave(4), Some(node(2)));
    assert_eq!(schedule.leader_for_wave(6), Some(node(1)));
}

#[test]
fn por_schedule_is_deterministic_across_map_insertion_order() {
    let first = LeaderSchedule::from_weights(
        LeaderSelectionPolicy::PorWeightedFair {
            schedule_length: 10,
        },
        7,
        &weights(&[(1, 60), (2, 30), (3, 10)]),
    )
    .unwrap();
    let second = LeaderSchedule::from_weights(
        LeaderSelectionPolicy::PorWeightedFair {
            schedule_length: 10,
        },
        7,
        &weights(&[(3, 10), (1, 60), (2, 30)]),
    )
    .unwrap();

    assert_eq!(first, second);
    assert_eq!(first.leader_for_wave(6), None);
    assert_eq!(first.leader_for_wave(7), first.leaders().first().cloned());
    assert_eq!(first.leader_for_wave(17), first.leaders().first().cloned());
}

#[test]
fn por_schedule_is_bounded_proportional_and_keeps_positive_nodes_live() {
    let schedule = LeaderSchedule::from_weights(
        LeaderSelectionPolicy::PorWeightedFair {
            schedule_length: 10,
        },
        0,
        &weights(&[(1, 60), (2, 30), (3, 10)]),
    )
    .unwrap();

    let count = |id| {
        schedule
            .leaders()
            .iter()
            .filter(|leader| **leader == node(id))
            .count()
    };
    assert_eq!(schedule.leaders().len(), 10);
    assert_eq!((count(1), count(2), count(3)), (5, 3, 2));
}

#[test]
fn por_schedule_excludes_zero_weight_keys() {
    let schedule = LeaderSchedule::from_weights(
        LeaderSelectionPolicy::PorWeightedFair { schedule_length: 4 },
        0,
        &weights(&[(1, 10), (2, 0)]),
    )
    .unwrap();

    assert_eq!(schedule.leaders(), &[node(1), node(1), node(1), node(1)]);
}

#[test]
fn exact_apportionment_ties_use_node_id() {
    let schedule = LeaderSchedule::from_weights(
        LeaderSelectionPolicy::PorWeightedFair { schedule_length: 5 },
        0,
        &weights(&[(3, 1), (1, 1), (2, 1)]),
    )
    .unwrap();

    let count = |id| {
        schedule
            .leaders()
            .iter()
            .filter(|leader| **leader == node(id))
            .count()
    };
    assert_eq!((count(1), count(2), count(3)), (2, 2, 1));
}

#[test]
fn invalid_schedule_inputs_fail_closed() {
    assert_eq!(
        LeaderSchedule::from_weights(LeaderSelectionPolicy::RoundRobin, 0, &HashMap::new()),
        Err(LeaderScheduleError::EmptyAuthorizedValidatorSet)
    );
    assert_eq!(
        LeaderSchedule::from_weights(
            LeaderSelectionPolicy::PorWeightedFair { schedule_length: 2 },
            0,
            &weights(&[(1, 1), (2, 1), (3, 1)])
        ),
        Err(LeaderScheduleError::ScheduleTooShort {
            schedule_length: 2,
            eligible_validators: 3,
        })
    );
    assert_eq!(
        LeaderSchedule::from_weights(
            LeaderSelectionPolicy::PorWeightedFair { schedule_length: 3 },
            0,
            &weights(&[(1, 0), (2, 0)])
        ),
        Err(LeaderScheduleError::ZeroActiveReputation)
    );
}
