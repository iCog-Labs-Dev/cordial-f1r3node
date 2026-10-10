use cordial_miners_core::NodeId;
use cordial_por::{
    EquivocationPenalty, InactivityPenalty, PorConfig, PorError, RatingRecord, ReputationBlock,
    ReputationBlockContext, ReputationEntry, ReputationPenaltyEvents, ReputationState,
    ReputationVector, build_rating_batch, build_reputation_block_with_penalties,
    decode_reputation_state_snapshot, encode_reputation_state_snapshot,
    replay_reputation_transition, replay_reputation_transition_with_penalties,
    reputation_list_commitment, verify_reputation_transition,
    verify_reputation_transition_with_penalties,
};

fn node(id: u8) -> NodeId {
    NodeId(vec![id])
}
fn previous() -> ReputationVector {
    ReputationVector {
        round: 0,
        values: (1..=4)
            .map(|id| ReputationEntry::new(node(id), 1000))
            .collect(),
    }
}
fn config() -> PorConfig {
    PorConfig::new(1000, 200)
}
fn events(slashed: &[u8], inactive: &[u8]) -> ReputationPenaltyEvents {
    ReputationPenaltyEvents {
        round: 1,
        equivocations: slashed
            .iter()
            .map(|id| EquivocationPenalty {
                offender: node(*id),
                evidence: vec![*id],
            })
            .collect(),
        inactivity: inactive
            .iter()
            .map(|id| InactivityPenalty {
                offender: node(*id),
                missed_rounds: 1,
            })
            .collect(),
    }
}
fn context() -> ReputationBlockContext<'static> {
    ReputationBlockContext {
        shard_id: b"root",
        source_finalized_wave: 0,
        previous_block: None,
    }
}
fn block(events: &ReputationPenaltyEvents) -> ReputationBlock {
    let config = config();
    let list =
        replay_reputation_transition_with_penalties(&previous(), &[], 1, &config, Some(events))
            .unwrap();
    build_reputation_block_with_penalties(
        context(),
        &build_rating_batch(1, vec![], &config).unwrap(),
        list,
        &config,
        Some(events),
    )
    .unwrap()
}

#[test]
fn replay_and_audit_apply_isolated_slash_and_inactivity() {
    let events = events(&[1], &[2]);
    let block = block(&events);
    let weights: Vec<_> = block
        .reputation_list
        .entries
        .iter()
        .map(|entry| entry.reputation)
        .collect();
    assert_eq!(weights, vec![0, 990, 1000, 1000]);
    assert_eq!(block.reputation_list.entries[0].retained_reputation, 750);
    assert!(block.reputation_list.entries[0].is_excluded);
    verify_reputation_transition_with_penalties(
        &previous(),
        &[],
        &block,
        context(),
        &config(),
        Some(&events),
    )
    .unwrap();
    assert_eq!(
        verify_reputation_transition(&previous(), &[], &block, context(), &config()),
        Err(PorError::ReputationBlockPenaltiesHashMismatch)
    );
}

#[test]
fn replay_applies_correlated_full_slash_independent_of_event_order() {
    let mut events = events(&[1, 2], &[]);
    let first = block(&events);
    events.equivocations.reverse();
    assert_eq!(block(&events), first);
    assert_eq!(first.reputation_list.entries[0].reputation, 0);
    assert_eq!(first.reputation_list.entries[1].reputation, 0);
    verify_reputation_transition_with_penalties(
        &previous(),
        &[],
        &first,
        context(),
        &config(),
        Some(&events),
    )
    .unwrap();
}

#[test]
fn forged_penalty_result_is_rejected_even_with_recomputed_root() {
    let events = events(&[1], &[2]);
    let mut block = block(&events);
    block.reputation_list.entries[0].retained_reputation += 1;
    block.header.reputation_root = reputation_list_commitment(&block.reputation_list).unwrap();
    assert_eq!(
        verify_reputation_transition_with_penalties(
            &previous(),
            &[],
            &block,
            context(),
            &config(),
            Some(&events)
        ),
        Err(PorError::ReputationValueMismatch)
    );
}

#[test]
fn configuration_change_is_rejected_before_replay() {
    let events = events(&[1], &[]);
    let block = block(&events);
    let mut config = config();
    config.base_slash_penalty += 1;
    assert_eq!(
        verify_reputation_transition_with_penalties(
            &previous(),
            &[],
            &block,
            context(),
            &config,
            Some(&events)
        ),
        Err(PorError::ReputationBlockConfigHashMismatch)
    );
}

#[test]
fn absence_alone_does_not_trigger_decay_and_none_preserves_old_api() {
    let expected = replay_reputation_transition(&previous(), &[], 1, &config()).unwrap();
    assert_eq!(expected.entries, previous().values);
    assert_eq!(
        replay_reputation_transition_with_penalties(&previous(), &[], 1, &config(), None).unwrap(),
        expected
    );
    assert_eq!(
        replay_reputation_transition_with_penalties(
            &previous(),
            &[],
            1,
            &config(),
            Some(&events(&[], &[]))
        )
        .unwrap(),
        expected
    );
}

#[test]
fn explicit_inactivity_compounds_once_per_consecutive_round() {
    let first = replay_reputation_transition_with_penalties(
        &previous(),
        &[],
        1,
        &config(),
        Some(&events(&[], &[1])),
    )
    .unwrap();
    let previous = ReputationVector {
        round: 1,
        values: first.entries,
    };
    let mut second_events = events(&[], &[1]);
    second_events.round = 2;
    let next = replay_reputation_transition_with_penalties(
        &previous,
        &[],
        2,
        &config(),
        Some(&second_events),
    )
    .unwrap();
    assert_eq!(next.entries[0].reputation, 980);
}

#[test]
fn invalid_or_ambiguous_events_are_rejected() {
    let mut wrong_round = events(&[1], &[]);
    wrong_round.round = 2;
    let mut empty_evidence = events(&[1], &[]);
    empty_evidence.equivocations[0].evidence.clear();
    let mut cumulative_inactivity = events(&[], &[1]);
    cumulative_inactivity.inactivity[0].missed_rounds = 2;
    for events in [
        wrong_round,
        empty_evidence,
        cumulative_inactivity,
        events(&[1, 1], &[]),
        events(&[5], &[]),
        events(&[], &[5]),
        events(&[1], &[1]),
        events(&[], &[1, 1]),
    ] {
        assert!(matches!(
            replay_reputation_transition_with_penalties(
                &previous(),
                &[],
                1,
                &config(),
                Some(&events)
            ),
            Err(PorError::InvalidPenaltyEvents(_))
        ));
    }
}

#[test]
fn inactive_node_must_be_absent_from_ratings() {
    let ratings = vec![RatingRecord::new(1, node(1), node(2), 1000, vec![1])];
    for id in [1, 2] {
        assert!(matches!(
            replay_reputation_transition_with_penalties(
                &previous(),
                &ratings,
                1,
                &config(),
                Some(&events(&[], &[id]))
            ),
            Err(PorError::InvalidPenaltyEvents(_))
        ));
    }
}

#[test]
fn excluded_keys_do_not_inflate_active_weight_or_receive_new_penalties() {
    let mut previous = previous();
    previous.values[3] = ReputationEntry::ejected(node(4));
    let next = replay_reputation_transition_with_penalties(
        &previous,
        &[],
        1,
        &config(),
        Some(&events(&[1], &[])),
    )
    .unwrap();
    // One third of active weight exceeds 30%; the excluded fourth key is not counted.
    assert_eq!(next.entries[0].reputation, 0);
    assert!(next.entries[3].is_excluded);
    assert!(matches!(
        replay_reputation_transition_with_penalties(
            &previous,
            &[],
            1,
            &config(),
            Some(&events(&[4], &[]))
        ),
        Err(PorError::InvalidPenaltyEvents(_))
    ));
}

#[test]
fn aggregate_weights_larger_than_u64_are_supported() {
    let mut previous = previous();
    for entry in &mut previous.values {
        entry.reputation = u64::MAX;
    }
    let config = PorConfig::new(1000, 0);
    let next = replay_reputation_transition_with_penalties(
        &previous,
        &[],
        1,
        &config,
        Some(&events(&[1], &[])),
    )
    .unwrap();
    assert_eq!(
        next.entries[0].retained_reputation,
        ((u128::from(u64::MAX) * 750) / 1000) as u64
    );
}

#[test]
fn state_application_is_atomic_and_snapshot_round_trips() {
    let events = events(&[1], &[2]);
    let block = block(&events);
    let mut state = ReputationState::new(0);
    for entry in previous().values {
        state.set_reputation(entry.node_id, entry.reputation);
    }
    let original = state.clone();
    let mut invalid = block.clone();
    invalid.reputation_list.entries[0].retained_reputation += 1;
    invalid.header.reputation_root = reputation_list_commitment(&invalid.reputation_list).unwrap();
    assert!(
        state
            .apply_reputation_block_with_penalties(
                b"root",
                0,
                &[],
                invalid,
                &config(),
                Some(&events)
            )
            .is_err()
    );
    assert_eq!(state, original);
    state
        .apply_reputation_block_with_penalties(b"root", 0, &[], block, &config(), Some(&events))
        .unwrap();
    assert_eq!(state.reputation_list().entries[0].reputation, 0);
    assert_eq!(state.reputation_list().entries[0].retained_reputation, 750);
    assert!(state.is_ejected(&node(1)));
    assert_eq!(
        decode_reputation_state_snapshot(&encode_reputation_state_snapshot(&state).unwrap())
            .unwrap(),
        state
    );
}

#[test]
fn slash_uses_previous_weight_without_rating_rewards_or_reclamping() {
    let ratings = vec![RatingRecord::new(1, node(2), node(1), 1000, vec![1])];
    let config = config();
    let next = replay_reputation_transition_with_penalties(
        &previous(),
        &ratings,
        1,
        &config,
        Some(&events(&[1], &[])),
    )
    .unwrap();
    assert_eq!(next.entries[0].reputation, 0);
    assert_eq!(next.entries[0].retained_reputation, 750);
    assert!(next.entries[0].is_excluded);
}

#[test]
fn zero_active_total_is_a_configuration_error() {
    let mut previous = previous();
    for entry in &mut previous.values {
        entry.reputation = 0;
    }
    assert!(matches!(
        replay_reputation_transition_with_penalties(
            &previous,
            &[],
            1,
            &config(),
            Some(&events(&[1], &[]))
        ),
        Err(PorError::InvalidConfiguration(_))
    ));
}

#[test]
fn evidence_substitution_is_rejected_even_when_the_result_is_identical() {
    let original = events(&[1], &[]);
    let block = block(&original);
    let mut replaced = original.clone();
    replaced.equivocations[0].evidence = vec![99];
    // Both evidence references justify the same arithmetic but are different inputs.
    assert_eq!(
        replay_reputation_transition_with_penalties(
            &previous(),
            &[],
            1,
            &config(),
            Some(&original)
        )
        .unwrap(),
        replay_reputation_transition_with_penalties(
            &previous(),
            &[],
            1,
            &config(),
            Some(&replaced)
        )
        .unwrap()
    );
    assert_eq!(
        verify_reputation_transition_with_penalties(
            &previous(),
            &[],
            &block,
            context(),
            &config(),
            Some(&replaced)
        ),
        Err(PorError::ReputationBlockPenaltiesHashMismatch)
    );
    let mut tampered = block.clone();
    tampered.header.penalties_hash =
        cordial_por::penalty_events_commitment(1, Some(&replaced)).unwrap();
    assert_ne!(
        cordial_por::reputation_block_hash(&block).unwrap(),
        cordial_por::reputation_block_hash(&tampered).unwrap()
    );
}

#[test]
fn penalty_commitment_survives_wire_round_trip() {
    let events = events(&[1], &[2]);
    let block = block(&events);
    let encoded = cordial_por::encode_reputation_block(&block).unwrap();
    let restored = cordial_por::decode_reputation_block(&encoded).unwrap();
    assert_eq!(restored, block);
    assert_eq!(
        restored.header.penalties_hash,
        cordial_por::penalty_events_commitment(1, Some(&events)).unwrap()
    );
    verify_reputation_transition_with_penalties(
        &previous(),
        &[],
        &restored,
        context(),
        &config(),
        Some(&events),
    )
    .unwrap();
}

#[test]
fn zero_effect_events_are_still_committed() {
    let mut config = config();
    config.inactivity_decay_gamma = 0;
    let events = events(&[], &[1]);
    let list =
        replay_reputation_transition_with_penalties(&previous(), &[], 1, &config, Some(&events))
            .unwrap();
    assert_eq!(list.entries, previous().values);
    let block = build_reputation_block_with_penalties(
        context(),
        &build_rating_batch(1, vec![], &config).unwrap(),
        list,
        &config,
        Some(&events),
    )
    .unwrap();
    assert_eq!(
        verify_reputation_transition(&previous(), &[], &block, context(), &config),
        Err(PorError::ReputationBlockPenaltiesHashMismatch)
    );
}

#[test]
fn slash_atomically_ejects_key_and_retains_capital_across_rounds_and_restart() {
    let events = events(&[1], &[]);
    let first = block(&events);
    let mut state = ReputationState::new(0);
    for entry in previous().values {
        state.set_reputation(entry.node_id, entry.reputation);
    }
    state
        .apply_reputation_block_with_penalties(
            b"root",
            0,
            &[],
            first.clone(),
            &config(),
            Some(&events),
        )
        .unwrap();
    assert!(state.is_ejected(&node(1)));
    assert!(!cordial_por::reputation_weights(&state).contains_key(&node(1)));
    assert_eq!(
        cordial_por::authorized_validator_weights(&state, &[node(1), node(2)]).unwrap()[&node(1)],
        0
    );
    state.set_reputation(node(1), 9999);
    assert_eq!(state.reputation_list().entries[0].retained_reputation, 750);
    assert_eq!(state.reputation_list().entries[0].reputation, 0);

    let prior = ReputationVector {
        round: 1,
        values: state.reputation_list().entries.clone(),
    };
    let next_list = replay_reputation_transition(&prior, &[], 2, &config()).unwrap();
    assert_eq!(next_list.entries[0], prior.values[0]);
    let next = cordial_por::build_reputation_block(
        ReputationBlockContext {
            shard_id: b"root",
            source_finalized_wave: 1,
            previous_block: Some(&first),
        },
        &build_rating_batch(2, vec![], &config()).unwrap(),
        next_list,
        &config(),
    )
    .unwrap();
    state
        .apply_reputation_block(b"root", 1, &[], next, &config())
        .unwrap();
    let restored =
        decode_reputation_state_snapshot(&encode_reputation_state_snapshot(&state).unwrap())
            .unwrap();
    assert_eq!(restored, state);
    assert!(restored.is_ejected(&node(1)));
    assert_eq!(
        restored.reputation_list().entries[0].retained_reputation,
        750
    );
    assert_eq!(
        cordial_por::authorized_validator_weights(&restored, &[node(1), node(2)]).unwrap()
            [&node(1)],
        0
    );

    // Even a direct vector replacement cannot erase the retained tombstone.
    state
        .apply_reputation_vector(ReputationVector {
            round: 3,
            values: vec![ReputationEntry::new(node(2), 1000)],
        })
        .unwrap();
    assert!(state.is_ejected(&node(1)));
    assert_eq!(state.reputation_list().entries[0].retained_reputation, 750);
}

#[test]
fn full_slash_ejects_with_no_retained_balance() {
    let events = events(&[1, 2], &[]);
    let block = block(&events);
    let mut state = ReputationState::new(0);
    for entry in previous().values {
        state.set_reputation(entry.node_id, entry.reputation);
    }
    state
        .apply_reputation_block_with_penalties(b"root", 0, &[], block, &config(), Some(&events))
        .unwrap();
    for id in [1, 2] {
        assert!(state.is_ejected(&node(id)));
        let entry = &state.reputation_list().entries[usize::from(id - 1)];
        assert_eq!((entry.reputation, entry.retained_reputation), (0, 0));
    }
}

#[test]
fn failed_slash_does_not_partially_eject() {
    let events = events(&[1], &[]);
    let mut block = block(&events);
    block.reputation_list.entries[0].retained_reputation += 1;
    block.header.reputation_root = reputation_list_commitment(&block.reputation_list).unwrap();
    let mut state = ReputationState::new(0);
    for entry in previous().values {
        state.set_reputation(entry.node_id, entry.reputation);
    }
    let original = state.clone();
    assert!(
        state
            .apply_reputation_block_with_penalties(b"root", 0, &[], block, &config(), Some(&events))
            .is_err()
    );
    assert_eq!(state, original);
    assert!(!state.is_ejected(&node(1)));
}

#[test]
fn invalid_penalty_configuration_is_rejected_without_events() {
    let valid_block = block(&events(&[], &[]));
    for field in 0..7 {
        let mut config = config();
        match field {
            0 => config.correlation_threshold = config.scale + 1,
            1 => config.base_slash_penalty = config.scale + 1,
            2 => config.inactivity_decay_gamma = config.scale + 1,
            3 => config.initial_reputation = config.scale + 1,
            4 => config.minimum_rating = config.maximum_rating + 1,
            5 => config.liquid_rank_alpha = config.scale + 1,
            _ => config.scale = 0,
        }
        assert!(matches!(
            config.validate(),
            Err(PorError::InvalidConfiguration(_))
        ));
        assert!(matches!(
            replay_reputation_transition(&previous(), &[], 1, &config),
            Err(PorError::InvalidConfiguration(_))
        ));
        assert!(matches!(
            verify_reputation_transition(&previous(), &[], &valid_block, context(), &config),
            Err(PorError::InvalidConfiguration(_))
        ));
        assert!(matches!(
            cordial_por::build_reputation_block(
                context(),
                &cordial_por::RatingBatch {
                    round: 1,
                    ratings: vec![]
                },
                valid_block.reputation_list.clone(),
                &config
            ),
            Err(PorError::InvalidConfiguration(_))
        ));
        assert!(matches!(
            cordial_por::blend_reputation_transition(
                &ReputationVector {
                    round: 1,
                    values: vec![]
                },
                &previous(),
                &config
            ),
            Err(PorError::InvalidConfiguration(_))
                | Err(PorError::InvalidTransitionScale)
                | Err(PorError::InvalidLiquidRankAlpha)
        ));
    }
}

#[test]
fn canonical_penalty_commitment_covers_categories_round_and_offenders() {
    use cordial_por::penalty_events_commitment;
    let original = events(&[1, 2], &[3, 4]);
    let hash = penalty_events_commitment(1, Some(&original)).unwrap();
    let mut reordered = original.clone();
    reordered.equivocations.reverse();
    reordered.inactivity.reverse();
    assert_eq!(
        penalty_events_commitment(1, Some(&reordered)).unwrap(),
        hash
    );
    let mut changed = original.clone();
    changed.inactivity.pop();
    assert_ne!(penalty_events_commitment(1, Some(&changed)).unwrap(), hash);
    let mut changed = original.clone();
    changed.equivocations[0].offender = node(5);
    assert_ne!(penalty_events_commitment(1, Some(&changed)).unwrap(), hash);
    let mut changed = original.clone();
    changed.round = 2;
    assert_ne!(penalty_events_commitment(2, Some(&changed)).unwrap(), hash);
    assert!(penalty_events_commitment(1, Some(&changed)).is_err());
    assert_eq!(
        penalty_events_commitment(1, None).unwrap(),
        penalty_events_commitment(1, Some(&events(&[], &[]))).unwrap()
    );
    assert_ne!(
        penalty_events_commitment(1, Some(&events(&[1], &[]))).unwrap(),
        penalty_events_commitment(1, Some(&events(&[], &[1]))).unwrap()
    );
}

#[test]
fn penalty_commitment_rejects_duplicate_overlapping_or_unbounded_evidence() {
    use cordial_por::{MAX_PENALTY_EVIDENCE_LEN, penalty_events_commitment};
    let mut oversized = events(&[1], &[]);
    oversized.equivocations[0].evidence = vec![0; MAX_PENALTY_EVIDENCE_LEN + 1];
    let mut bad_count = events(&[], &[1]);
    bad_count.inactivity[0].missed_rounds = 2;
    for events in [
        oversized,
        bad_count,
        events(&[1, 1], &[]),
        events(&[1], &[1]),
    ] {
        assert!(penalty_events_commitment(1, Some(&events)).is_err());
    }
}

#[test]
fn retained_capital_cannot_be_active_or_disappear_from_an_audited_successor() {
    let events = events(&[1], &[]);
    let first = block(&events);
    let mut invalid = first.reputation_list.clone();
    invalid.entries[0].is_excluded = false;
    assert_eq!(
        reputation_list_commitment(&invalid),
        Err(PorError::ReputationExclusionMismatch)
    );
    invalid.entries[0].is_excluded = true;
    invalid.entries[0].reputation = 1;
    assert_eq!(
        reputation_list_commitment(&invalid),
        Err(PorError::ReputationExclusionMismatch)
    );

    let prior = ReputationVector {
        round: 1,
        values: first.reputation_list.entries.clone(),
    };
    let mut next_list = replay_reputation_transition(&prior, &[], 2, &config()).unwrap();
    next_list.entries[0].retained_reputation = 0;
    let context = ReputationBlockContext {
        shard_id: b"root",
        source_finalized_wave: 1,
        previous_block: Some(&first),
    };
    let forged = cordial_por::build_reputation_block(
        context,
        &build_rating_batch(2, vec![], &config()).unwrap(),
        next_list,
        &config(),
    )
    .unwrap();
    assert_eq!(
        verify_reputation_transition(&prior, &[], &forged, context, &config()),
        Err(PorError::ReputationValueMismatch)
    );
}
