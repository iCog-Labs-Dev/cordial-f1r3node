//! Export the production pipeline's penalty boundary, not a second transition.
//! By default validate checked-in bytes; POR_REGENERATE_SLASHING=1 rewrites them.
#[path = "support/slashing.rs"]
mod support;

use cordial_por::*;
use serde_json::{Value, json};
use std::path::PathBuf;
use support::{node, penalty_at};

fn category(error: &PorError) -> &'static str {
    match error {
        PorError::InvalidConfiguration(_) => "invalid_configuration",
        PorError::InvalidPenaltyEvents(_) => "invalid_penalty_events",
        PorError::SlashOverflow | PorError::InactivityDecayOverflow => "arithmetic_overflow",
        other => panic!("unmapped conformance error: {other:?}"),
    }
}

fn entries(list: &ReputationList) -> Value {
    json!({"round": list.round, "entries": list.entries.iter().map(|e| {
        json!({"id": (1..=8).find(|id| node(*id) == e.node_id).unwrap(),
            "active": e.reputation, "retained": e.retained_reputation,
            "excluded": e.is_excluded})
    }).collect::<Vec<_>>()})
}

fn weights(state: &ReputationState) -> Value {
    let ids: Vec<_> = state
        .reputation_list()
        .entries
        .iter()
        .map(|e| e.node_id.clone())
        .collect();
    let projected = authorized_validator_weights(state, &ids).unwrap();
    // Traverse the canonical list, never the HashMap's iteration order.
    json!(
        ids.iter()
            .map(|id| json!({
                "id": (1..=8).find(|n| node(*n) == *id).unwrap(), "weight": projected[id]
            }))
            .collect::<Vec<_>>()
    )
}

fn context(state: &ReputationState) -> ReputationBlockContext<'_> {
    ReputationBlockContext {
        shard_id: b"root",
        source_finalized_wave: state.round(),
        previous_block: state.latest_block(),
    }
}

fn vector(state: &ReputationState) -> ReputationVector {
    ReputationVector {
        round: state.round(),
        values: state.reputation_list().entries.clone(),
    }
}

fn events(round: u64, slashed: &[u8], inactive: &[u8]) -> ReputationPenaltyEvents {
    ReputationPenaltyEvents {
        round,
        equivocations: slashed.iter().map(|id| penalty_at(*id, 0, round)).collect(),
        inactivity: inactive
            .iter()
            .map(|id| InactivityPenalty {
                offender: node(*id),
                missed_rounds: 1,
            })
            .collect(),
    }
}

fn make_block(
    state: &ReputationState,
    ratings: &[RatingRecord],
    config: &PorConfig,
    events: Option<&ReputationPenaltyEvents>,
) -> Result<ReputationBlock, PorError> {
    let round = state.round() + 1;
    let list = replay_reputation_transition_with_penalties(
        &vector(state),
        ratings,
        round,
        config,
        events,
    )?;
    build_reputation_block_with_penalties(
        context(state),
        &build_rating_batch(round, ratings.to_vec(), config)?,
        list,
        config,
        events,
    )
}

#[derive(Clone)]
struct Case {
    name: &'static str,
    state: ReputationState,
    config: PorConfig,
    ratings: Vec<RatingRecord>,
    events: ReputationPenaltyEvents,
}

fn base(name: &'static str, balances: &[u64], slashed: &[u8], inactive: &[u8]) -> Case {
    let mut state = ReputationState::new(0);
    for (i, balance) in balances.iter().enumerate() {
        state.set_reputation(node((i + 1) as u8), *balance);
    }
    Case {
        name,
        state,
        config: PorConfig::new(1000, 200),
        ratings: vec![],
        events: events(1, slashed, inactive),
    }
}

fn capture(case: Case) -> Value {
    let Case {
        name,
        mut state,
        config,
        ratings,
        events,
    } = case;
    let previous = vector(&state);
    let round = state.round() + 1;
    // Valid fallback config lets invalid-config cases still carry an independently
    // computed candidate; config validation itself is part of Lean admission.
    let candidate_config = if config.validate().is_ok() {
        config.clone()
    } else {
        PorConfig::new(1000, 200)
    };
    let candidate =
        replay_reputation_transition(&previous, &ratings, round, &candidate_config).unwrap();
    let result = replay_reputation_transition_with_penalties(
        &previous,
        &ratings,
        round,
        &config,
        Some(&events),
    );
    // Pending data and latest_block must survive every rejected application.
    state.add_rating(RatingRecord::new(round, node(3), node(4), 10, vec![7]));
    let before = state.clone();
    let mut tiers = vec![];
    let expected = match result {
        Ok(list) => {
            let block = make_block(&state, &ratings, &config, Some(&events)).unwrap();
            assert_eq!(block.reputation_list, list);
            verify_reputation_transition_with_penalties(
                &previous,
                &ratings,
                &block,
                context(&state),
                &config,
                Some(&events),
            )
            .unwrap();
            // The byte/hash boundary is Rust-owned. Check deterministic commitment,
            // full wire roundtrip, and rejection of a recomputed-root forgery here.
            assert_eq!(
                decode_reputation_block(&encode_reputation_block(&block).unwrap()).unwrap(),
                block
            );
            let mut reversed = events.clone();
            reversed.equivocations.reverse();
            reversed.inactivity.reverse();
            assert_eq!(
                make_block(&state, &ratings, &config, Some(&reversed)).unwrap(),
                block
            );
            let mut forged = block.clone();
            let innocent = forged
                .reputation_list
                .entries
                .iter_mut()
                .find(|e| !e.is_excluded)
                .unwrap();
            innocent.reputation += 1;
            forged.header.reputation_root =
                reputation_list_commitment(&forged.reputation_list).unwrap();
            assert!(
                state
                    .apply_reputation_block_with_penalties(
                        b"root",
                        previous.round,
                        &ratings,
                        forged,
                        &config,
                        Some(&events)
                    )
                    .is_err()
            );
            assert_eq!(state, before);
            if !events.equivocations.is_empty() {
                let total: u64 = previous
                    .values
                    .iter()
                    .filter(|e| !e.is_excluded)
                    .map(|e| e.reputation)
                    .sum();
                // All exported proofs have offense depth zero, one correlation group.
                let offending: u64 = events
                    .equivocations
                    .iter()
                    .map(|event| {
                        previous
                            .values
                            .iter()
                            .find(|e| e.node_id == event.offender)
                            .unwrap()
                            .reputation
                    })
                    .sum();
                let penalty = compute_slash_penalty(offending, total, &config).unwrap();
                tiers = events.equivocations.iter().map(|event| json!({
                    "id": (1..=8).find(|id| node(*id) == event.offender).unwrap(), "penalty": penalty
                })).collect();
                tiers.sort_by_key(|e| e["id"].as_u64().unwrap());
            }
            state
                .apply_reputation_block_with_penalties(
                    b"root",
                    previous.round,
                    &ratings,
                    block,
                    &config,
                    Some(&events),
                )
                .unwrap();
            assert_eq!(state.reputation_list(), &list);
            assert_eq!(state.pending_ratings(), before.pending_ratings());
            json!({"status": "ok", "state": entries(state.reputation_list()), "weights": weights(&state), "tiers": tiers})
        }
        Err(error) => {
            // An otherwise valid proposal reaches the real audit/application path.
            let mut proposal = make_block(&state, &ratings, &candidate_config, None).unwrap();
            if let Ok(hash) = penalty_events_commitment(round, Some(&events)) {
                proposal.header.penalties_hash = hash;
            }
            let audit_error = verify_reputation_transition_with_penalties(
                &previous,
                &ratings,
                &proposal,
                context(&state),
                &config,
                Some(&events),
            )
            .unwrap_err();
            assert_eq!(category(&audit_error), category(&error), "audit: {name}");
            let apply_error = state
                .apply_reputation_block_with_penalties(
                    b"root",
                    previous.round,
                    &ratings,
                    proposal,
                    &config,
                    Some(&events),
                )
                .unwrap_err();
            assert_eq!(category(&apply_error), category(&error), "apply: {name}");
            assert_eq!(state, before, "complete-state atomicity: {name}");
            json!({"status": category(&error), "state": entries(state.reputation_list()),
                "weights": weights(&state), "tiers": tiers})
        }
    };
    json!({"name": name,
        "config": {"scale": config.scale, "initial": config.initial_reputation,
            "alpha": config.liquid_rank_alpha, "minimum_rating": config.minimum_rating,
            "maximum_rating": config.maximum_rating, "missing_entry_policy": "carry_forward",
            "threshold": config.correlation_threshold, "base_penalty": config.base_slash_penalty,
            "gamma": config.inactivity_decay_gamma},
        "previous": entries(&ReputationList { round: previous.round, entries: previous.values }),
        "candidate": entries(&candidate),
        "ratings": ratings.iter().map(|r| json!({"round": r.round,
            "rater": (1..=8).find(|id| node(*id) == r.rater).unwrap(),
            "recipient": (1..=8).find(|id| node(*id) == r.recipient).unwrap(),
            "score": r.score, "signature": r.signature})).collect::<Vec<_>>(),
        "events": {"round": events.round,
            "equivocations": events.equivocations.iter().map(|e| json!({
                "id": (1..=8).find(|id| node(*id) == e.offender).unwrap(),
                "evidence_length": e.evidence.len(), "offense_round": 0,
                "application_round": round})).collect::<Vec<_>>(),
            "inactivity": events.inactivity.iter().map(|e| json!({
                "id": (1..=8).find(|id| node(*id) == e.offender).unwrap(),
                "missed_rounds": e.missed_rounds})).collect::<Vec<_>>()},
        "expected": expected})
}

fn fixtures() -> Value {
    let equal = [1000; 4];
    let mut cases = vec![
        base("isolated_below_30", &equal, &[1], &[]),
        base("exactly_30", &[300, 700], &[1], &[]),
        base("smallest_above_30", &[301, 699], &[1], &[]),
        base("correlated_full", &equal, &[1, 2], &[]),
        base("correlated_reversed", &equal, &[2, 1], &[]),
        base("unequal_weights", &[100, 200, 300, 400], &[1, 2], &[]),
        base("rounding", &[101, 899], &[1], &[]),
        base("mixed_penalties", &equal, &[1], &[2]),
        base("inactivity", &equal, &[], &[1, 2]),
        base("inactivity_reversed", &equal, &[], &[2, 1]),
        base("empty_events", &equal, &[], &[]),
        base("duplicate_equivocation", &equal, &[1, 1], &[]),
        base("duplicate_inactivity", &equal, &[], &[2, 2]),
        base("overlap", &equal, &[1], &[1]),
        base("unknown_equivocator", &equal, &[5], &[]),
        base("unknown_inactive", &equal, &[], &[5]),
    ];
    for (name, length) in [
        ("empty_evidence", 0),
        ("oversized_evidence", MAX_PENALTY_EVIDENCE_LEN + 1),
    ] {
        let mut case = base(name, &equal, &[1], &[]);
        case.events.equivocations[0].evidence = vec![0; length];
        cases.push(case);
    }
    let mut wrong = base("wrong_event_round", &equal, &[], &[1]);
    wrong.events.round = 2;
    cases.push(wrong);
    let mut missed = base("unsupported_inactivity", &equal, &[], &[1]);
    missed.events.inactivity[0].missed_rounds = 2;
    cases.push(missed);
    for (name, recipient) in [
        ("slash_ignores_rating_reward", 1),
        ("new_rated_validator", 5),
        ("inactive_rater", 2),
        ("inactive_recipient", 1),
    ] {
        let mut case = base(name, &equal, if recipient < 3 { &[1] } else { &[] }, &[]);
        case.ratings.push(RatingRecord::new(
            1,
            node(2),
            node(recipient),
            1000,
            vec![1],
        ));
        if name == "inactive_rater" {
            case.events = events(1, &[], &[2]);
        }
        if name == "inactive_recipient" {
            case.events = events(1, &[], &[1]);
        }
        // Avoid self-rating in the rater case.
        if name == "inactive_rater" {
            case.ratings[0].recipient = node(1);
        }
        cases.push(case);
    }
    for (name, mutate) in [
        ("zero_scale", 0),
        ("threshold_above_scale", 1),
        ("penalty_above_scale", 2),
        ("gamma_above_scale", 3),
        ("alpha_above_scale", 4),
        ("initial_above_scale", 5),
        ("inverted_rating_bounds", 6),
    ] {
        let mut case = base(name, &equal, &[1], &[]);
        match mutate {
            0 => case.config.scale = 0,
            1 => case.config.correlation_threshold = 1001,
            2 => case.config.base_slash_penalty = 1001,
            3 => case.config.inactivity_decay_gamma = 1001,
            4 => case.config.liquid_rank_alpha = 1001,
            5 => case.config.initial_reputation = 1001,
            _ => case.config.minimum_rating = 1001,
        }
        cases.push(case);
    }
    let mut wide = base("arithmetic_overflow", &[u64::MAX; 4], &[1, 2], &[]);
    wide.config = PorConfig::new(u64::MAX, 0);
    cases.push(wide);
    // Establish a real retained balance and latest block through audited slashing.
    let mut seeded = base("preserve_ejection", &equal, &[1], &[]);
    let block = make_block(&seeded.state, &[], &seeded.config, Some(&seeded.events)).unwrap();
    seeded
        .state
        .apply_reputation_block_with_penalties(
            b"root",
            0,
            &[],
            block,
            &seeded.config,
            Some(&seeded.events),
        )
        .unwrap();
    seeded.events = events(2, &[], &[2]);
    cases.push(seeded.clone());
    seeded.name = "already_ejected";
    seeded.events = events(2, &[1], &[]);
    cases.push(seeded);
    cases.sort_by_key(|case| case.name);
    json!({"schema_version": 1, "boundary": "cordial-por/slashing-structural-v1",
        "fixtures": cases.into_iter().map(capture).collect::<Vec<_>>()})
}

#[test]
fn canonical_slashing_fixtures() {
    let path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../lean/slashing/fixtures.v1.json");
    let bytes = format!("{}\n", serde_json::to_string_pretty(&fixtures()).unwrap());
    if std::env::var("POR_REGENERATE_SLASHING").as_deref() == Ok("1") {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, &bytes).unwrap();
    }
    assert_eq!(
        std::fs::read_to_string(path).expect("regenerate slashing fixtures"),
        bytes,
        "slashing fixtures changed; review POR_REGENERATE_SLASHING=1 regeneration"
    );
}
