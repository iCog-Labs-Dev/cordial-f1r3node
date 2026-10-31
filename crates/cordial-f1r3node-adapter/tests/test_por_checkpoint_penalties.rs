//! Checkpoint carriers must be verified, complete, and confined to their window.
#[path = "support/por_event_trace.rs"]
mod por_event_trace;
macro_rules! trace_event {
    ($kind:expr, $round:expr, $payload:expr $(,)?) => {
        if por_event_trace::enabled() {
            por_event_trace::emit($kind, $round, $payload);
        }
    };
}

use cordial_f1r3node_adapter::{
    ordered_output::OrderedFinalizedOutput,
    por::{
        checkpoint_ratings::{
            close_checkpoint_rating_exchange_if_complete, stage_local_checkpoint_rating_batch,
        },
        lifecycle::checkpoint::CompletedCheckpointRatingRound,
        persistence::DurablePorState,
        rating_key::load_bonded_rating_key,
        transport::{file_exchange::FileRatingExchange, outbox::RatingOutbox},
    },
    por_finality::finalized_prefix_checkpoint,
    por_rating_wire::BlockProductionRatingEnvelopeV2,
    por_ratings::build_finalized_checkpoint_rating_batch,
};
use cordial_miners_core::{
    Block, BlockContent, BlockIdentity, Blocklace, NodeId,
    consensus::{CordialEquivocationEvidence, verify_equivocation_evidence},
    crypto,
    execution::{BlockState, CordialBlockPayload, PorEquivocationAuthorization},
};
use cordial_por::{PorConfig, ReputationState, checkpoint_equivocation_authorization_payload};
use k256::ecdsa::SigningKey;
use std::{
    collections::{HashMap, HashSet},
    fs,
};

fn node(seed: u8) -> NodeId {
    NodeId(
        SigningKey::from_slice(&[seed; 32])
            .unwrap()
            .verifying_key()
            .to_encoded_point(true)
            .as_bytes()
            .to_vec(),
    )
}
fn signed(seed: u8, payload: Vec<u8>, predecessors: HashSet<BlockIdentity>) -> Block {
    let content = BlockContent {
        payload,
        predecessors,
    };
    let creator = node(seed);
    let content_hash = crypto::block_hash(&creator, &content);
    Block {
        identity: BlockIdentity {
            content_hash,
            creator,
            signature: crypto::sign(&content_hash, &[seed; 32]),
        },
        content,
    }
}
fn raw(seed: u8, tag: u8) -> CordialEquivocationEvidence {
    CordialEquivocationEvidence::new(
        node(seed),
        0,
        [0, 1]
            .map(|branch| signed(seed, vec![tag, branch], HashSet::new()))
            .to_vec(),
    )
}
fn carrier(claim: &CordialEquivocationEvidence, round: u64) -> PorEquivocationAuthorization {
    let proof = verify_equivocation_evidence(claim, &Blocklace::new()).unwrap();
    trace_event!(
        "equivocation_verified",
        round,
        serde_json::json!({
            "validator": (1..=4u8).find(|seed| node(*seed) == claim.validator).map(|seed| format!("validator-{seed}")),
            "evidence_round": claim.round,
            "blocks": claim.blocks.iter().map(|block| serde_json::json!({"id": hex::encode(block.identity.content_hash), "parents": block.content.predecessors.iter().map(|parent| hex::encode(parent.content_hash)).collect::<Vec<_>>()})).collect::<Vec<_>>(),
        })
    );
    PorEquivocationAuthorization {
        shard_id: b"root".to_vec(),
        source_finalized_wave: 0,
        round,
        message: checkpoint_equivocation_authorization_payload(b"root", round, &proof).unwrap(),
    }
}
struct Fixture {
    lace: Blocklace,
    output: OrderedFinalizedOutput,
    state: ReputationState,
    membership: HashMap<NodeId, u64>,
    config: PorConfig,
}
impl Fixture {
    fn new(carriers: Vec<PorEquivocationAuthorization>) -> Self {
        Self::two_windows(carriers, vec![])
    }
    fn two_windows(
        carriers: Vec<PorEquivocationAuthorization>,
        later: Vec<PorEquivocationAuthorization>,
    ) -> Self {
        Self::windows(vec![carriers, later])
    }

    fn windows(windows: Vec<Vec<PorEquivocationAuthorization>>) -> Self {
        let mut lace = Blocklace::new();
        let mut ids: Vec<BlockIdentity> = vec![];
        for index in 0..(windows.len() as u64 * 16) {
            let payload = CordialBlockPayload {
                por_authorizations: if index % 16 == 8 {
                    windows[(index / 16) as usize].clone()
                } else {
                    vec![]
                },
                state: BlockState {
                    pre_state_hash: vec![],
                    post_state_hash: vec![],
                    bonds: vec![],
                    block_number: index,
                },
                deploys: vec![],
                rejected_deploys: vec![],
                system_deploys: vec![],
            }
            .to_bytes();
            let block = signed(1, payload, ids.last().cloned().into_iter().collect());
            lace.insert(block.clone(), &crypto::Secp256k1Scheme)
                .unwrap();
            trace_event!(
                "block_added",
                index / 16 + 1,
                serde_json::json!({
                    "id": hex::encode(block.identity.content_hash), "creator": "validator-1", "height": index,
                    "parents": block.content.predecessors.iter().map(|parent| hex::encode(parent.content_hash)).collect::<Vec<_>>(),
                    "authorization_count": if index % 16 == 8 { windows[(index / 16) as usize].len() } else { 0 },
                }),
            );
            ids.push(block.identity);
        }
        let output = OrderedFinalizedOutput::new(ids.clone(), ids.last().cloned(), 3, 4, ids.len())
            .with_timestamp(0);
        let mut state = ReputationState::new(0);
        let membership = (1..=4)
            .map(|seed| {
                state.set_reputation(node(seed), 1000);
                (node(seed), 1000)
            })
            .collect();
        Self {
            lace,
            output,
            state,
            membership,
            config: PorConfig::new(1000, 200),
        }
    }
    fn complete(
        &self,
        state: &ReputationState,
        round: u64,
        local: u8,
    ) -> CompletedCheckpointRatingRound {
        let dir = tempfile::tempdir().unwrap();
        let key_path = dir.path().join("key");
        fs::write(&key_path, hex::encode([local; 32])).unwrap();
        let key = load_bonded_rating_key(&key_path, &self.membership).unwrap();
        let outbox = RatingOutbox::open(&dir.path().join("outbox")).unwrap();
        let exchange = FileRatingExchange::open(&dir.path().join("exchange"), b"root").unwrap();
        let checkpoint = finalized_prefix_checkpoint(&self.output, round)
            .unwrap()
            .unwrap();
        let count = stage_local_checkpoint_rating_batch(
            &self.lace,
            &self.output,
            &checkpoint,
            state,
            &self.config,
            &key,
            &outbox,
        )
        .unwrap();
        trace_event!(
            "ratings_staged",
            round,
            serde_json::json!({"observer": local, "count": count, "ejected": state.is_ejected(&node(local))}),
        );
        if state.is_ejected(&node(local)) {
            assert_eq!(count, 0);
            assert!(
                outbox
                    .load_checkpoint_batch(checkpoint.rating_source(), &node(local))
                    .unwrap()
                    .is_none()
            );
            assert!(
                build_finalized_checkpoint_rating_batch(
                    &self.lace,
                    &self.output,
                    &checkpoint,
                    &node(local),
                    state,
                    &self.config,
                    &[local; 32]
                )
                .is_err()
            );
        }
        // Each observer has its own exchange and delivery schedule. Closure
        // must wait for missing required ratings, but empty peer batches are
        // already complete under the checkpoint collector contract.
        let mut peers: Vec<u8> = (1..=4).collect();
        peers.rotate_left(usize::from(local - 1));
        if local.is_multiple_of(2) {
            peers.reverse();
        }
        let authorized = self.membership.keys().cloned().collect();
        let pending = close_checkpoint_rating_exchange_if_complete(
            &self.lace,
            &self.output,
            &checkpoint,
            state,
            &self.config,
            &key,
            &authorized,
            &outbox,
            &exchange,
        )
        .unwrap();
        trace_event!(
            "closure_checked",
            round,
            serde_json::json!({"observer": local, "complete": pending.is_some()}),
        );
        let mut missing_peer_ratings = false;
        for &seed in &peers {
            if seed == local || state.is_ejected(&node(seed)) {
                continue;
            }
            let required = cordial_f1r3node_adapter::por::interactions::
                admit_checkpoint_block_production_interactions(
                    &self.lace,
                    &self.output,
                    &checkpoint,
                    &node(seed),
                    state,
                )
                .unwrap();
            missing_peer_ratings |= !required.is_empty();
        }
        assert_eq!(
            pending.is_none(),
            missing_peer_ratings,
            "round {round}, observer {local}: closure must wait exactly when peers have required ratings"
        );
        for seed in peers {
            if seed == local || state.is_ejected(&node(seed)) {
                continue;
            }
            let batch = build_finalized_checkpoint_rating_batch(
                &self.lace,
                &self.output,
                &checkpoint,
                &node(seed),
                state,
                &self.config,
                &[seed; 32],
            )
            .unwrap();
            let publisher = exchange
                .checkpoint_broadcaster(checkpoint.rating_source(), node(seed))
                .unwrap();
            let mut ratings = batch.ratings;
            if local.is_multiple_of(2) {
                ratings.reverse();
            }
            let published_count = ratings.len();
            for rating in ratings {
                let encoded =
                    BlockProductionRatingEnvelopeV2::new(checkpoint.rating_source(), rating)
                        .unwrap()
                        .encode();
                publisher.publish(&encoded).unwrap();
                publisher.publish(&encoded).unwrap();
            }
            trace_event!(
                "peer_batch_published",
                round,
                serde_json::json!({"observer": local, "peer": seed, "count": published_count, "copies_per_rating": 2}),
            );
        }
        // An ejected key can still write a signed file. Such files must not
        // enter the batch or prevent eligible observers from closing it.
        let mut stale_state = ReputationState::new(state.round());
        for identity in self.membership.keys() {
            stale_state.set_reputation(identity.clone(), 1000);
        }
        for seed in 1..=4 {
            if !state.is_ejected(&node(seed)) {
                continue;
            }
            let stale = build_finalized_checkpoint_rating_batch(
                &self.lace,
                &self.output,
                &checkpoint,
                &node(seed),
                &stale_state,
                &self.config,
                &[seed; 32],
            )
            .unwrap();
            let publisher = exchange
                .checkpoint_broadcaster(checkpoint.rating_source(), node(seed))
                .unwrap();
            let stale_count = stale.ratings.len();
            for rating in stale.ratings {
                publisher
                    .publish(
                        &BlockProductionRatingEnvelopeV2::new(checkpoint.rating_source(), rating)
                            .unwrap()
                            .encode(),
                    )
                    .unwrap();
            }
            trace_event!(
                "excluded_batch_published",
                round,
                serde_json::json!({"observer": local, "peer": seed, "count": stale_count, "copies_per_rating": 1})
            );
        }
        let completed = close_checkpoint_rating_exchange_if_complete(
            &self.lace,
            &self.output,
            &checkpoint,
            state,
            &self.config,
            &key,
            &self.membership.keys().cloned().collect(),
            &outbox,
            &exchange,
        )
        .unwrap()
        .unwrap();
        trace_event!(
            "checkpoint_closed",
            round,
            serde_json::json!({"observer": local}),
        );
        completed
    }
}

#[test]
fn finalized_penalty_survives_restart_and_ejected_observer_closes_next_round() {
    let claim = raw(2, 9);
    let fixture = Fixture::new(vec![carrier(&claim, 1)]);
    let completed = fixture.complete(&fixture.state, 1, 1);
    let directory = tempfile::tempdir().unwrap();
    let mut durable = DurablePorState::open(directory.path(), fixture.state.clone()).unwrap();
    // A closed rating batch alone cannot omit a finalized authorization.
    assert!(
        durable
            .apply_completed_checkpoint_round(&completed, &fixture.config, b"root")
            .is_err()
    );
    assert_eq!(durable.state().unwrap(), &fixture.state);
    assert!(durable.history().is_empty().unwrap());
    let authorized = completed
        .with_finalized_equivocations(&fixture.lace, &fixture.output, &fixture.membership, b"root")
        .unwrap();
    let first = durable
        .apply_completed_checkpoint_round(&authorized, &fixture.config, b"root")
        .unwrap();
    assert!(durable.state().unwrap().is_ejected(&node(2)));
    assert_eq!(
        cordial_por::authorized_validator_weights(
            durable.state().unwrap(),
            &fixture.membership.keys().cloned().collect::<Vec<_>>()
        )
        .unwrap()[&node(2)],
        0
    );
    let retained = durable
        .state()
        .unwrap()
        .reputation_list()
        .entries
        .iter()
        .find(|entry| entry.node_id == node(2))
        .unwrap()
        .retained_reputation;
    assert_eq!(retained, 750);
    drop(durable);
    let mut recovered = DurablePorState::open(directory.path(), ReputationState::new(99)).unwrap();
    assert_eq!(
        recovered.state().unwrap().latest_block(),
        Some(&first.block)
    );
    assert!(recovered.state().unwrap().is_ejected(&node(2)));
    // Membership still includes the ejected key, but only preceding-state eligible raters are required.
    let active_view = fixture.complete(recovered.state().unwrap(), 2, 1);
    let ejected_view = fixture.complete(recovered.state().unwrap(), 2, 2);
    assert_eq!(active_view, ejected_view);
    assert!(
        active_view
            .batch()
            .ratings
            .iter()
            .all(|rating| rating.rater != node(2))
    );
    let next = active_view
        .with_finalized_equivocations(&fixture.lace, &fixture.output, &fixture.membership, b"root")
        .unwrap();
    assert!(next.penalties().unwrap().equivocations.is_empty());
    recovered
        .apply_completed_checkpoint_round(&next, &fixture.config, b"root")
        .unwrap();
    assert_eq!(recovered.state().unwrap().round(), 2);
    let entry = recovered
        .state()
        .unwrap()
        .reputation_list()
        .entries
        .iter()
        .find(|entry| entry.node_id == node(2))
        .unwrap();
    assert!(entry.is_excluded);
    assert_eq!((entry.reputation, entry.retained_reputation), (0, retained));
}

/// Export a deterministic, evidence-backed presentation fixture when requested.
/// Four durable observers independently replay the same finalized source and
/// complete signed rating batch, then recover the same ejected state.
#[test]
fn four_observer_checkpoint_penalty_demo() {
    let claim = raw(2, 9);
    let fixture = Fixture::new(vec![carrier(&claim, 1)]);
    let completed = fixture.complete(&fixture.state, 1, 1);
    let initial_weights = demo_weights(&fixture.state, &fixture.membership);
    let rating_count = completed.batch().ratings.len();
    let authorized = completed
        .with_finalized_equivocations(&fixture.lace, &fixture.output, &fixture.membership, b"root")
        .unwrap();
    assert_eq!(authorized.penalties().unwrap().equivocations.len(), 1);

    let mut observers = Vec::new();
    let mut penalties_hash = None;
    for seed in 1..=4u8 {
        let directory = tempfile::tempdir().unwrap();
        let mut durable = DurablePorState::open(directory.path(), fixture.state.clone()).unwrap();
        let independently_authorized = fixture
            .complete(&fixture.state, 1, seed)
            .with_finalized_equivocations(
                &fixture.lace,
                &fixture.output,
                &fixture.membership,
                b"root",
            )
            .unwrap();
        assert_eq!(independently_authorized, authorized);
        let first = durable
            .apply_completed_checkpoint_round(&independently_authorized, &fixture.config, b"root")
            .unwrap();
        penalties_hash.get_or_insert(first.block.header.penalties_hash);
        assert_eq!(penalties_hash, Some(first.block.header.penalties_hash));
        let first_hash = hex::encode(cordial_por::reputation_block_hash(&first.block).unwrap());
        let first_weights = demo_weights(durable.state().unwrap(), &fixture.membership);
        assert_eq!(first_weights["validator-2"], 0);
        assert_ne!(first_weights["validator-1"], first_weights["validator-3"]);
        let retained = demo_retained(durable.state().unwrap(), &node(2));
        assert_eq!(retained, 750);
        drop(durable);

        let mut recovered =
            DurablePorState::open(directory.path(), ReputationState::new(99)).unwrap();
        assert_eq!(recovered.state().unwrap().round(), 1);
        assert!(recovered.state().unwrap().is_ejected(&node(2)));
        assert_eq!(
            demo_weights(recovered.state().unwrap(), &fixture.membership),
            first_weights
        );
        let second = fixture.complete(recovered.state().unwrap(), 2, seed);
        assert!(
            second
                .batch()
                .ratings
                .iter()
                .all(|rating| rating.rater != node(2))
        );
        let second = second
            .with_finalized_equivocations(
                &fixture.lace,
                &fixture.output,
                &fixture.membership,
                b"root",
            )
            .unwrap();
        assert!(second.penalties().unwrap().equivocations.is_empty());
        let second = recovered
            .apply_completed_checkpoint_round(&second, &fixture.config, b"root")
            .unwrap();
        assert!(recovered.state().unwrap().is_ejected(&node(2)));
        assert_eq!(
            demo_retained(recovered.state().unwrap(), &node(2)),
            retained
        );
        observers.push(serde_json::json!({
            "observer": format!("observer-{seed}"),
            "round_1_checkpoint": first_hash,
            "round_1_weights": first_weights,
            "round_1_retained": retained,
            "round_2_checkpoint": hex::encode(cordial_por::reputation_block_hash(&second.block).unwrap()),
            "round_2_weights": demo_weights(recovered.state().unwrap(), &fixture.membership),
            "recovered_round": 1,
            "final_round": recovered.state().unwrap().round(),
        }));
    }
    for observer in &observers[1..] {
        for key in [
            "round_1_checkpoint",
            "round_1_weights",
            "round_2_checkpoint",
            "round_2_weights",
        ] {
            assert_eq!(observer[key], observers[0][key], "{key} differs");
        }
    }
    if let Ok(path) = std::env::var("POR_DEMO_OUTPUT") {
        let source =
            cordial_f1r3node_adapter::por_finality::finalized_prefix_checkpoint(&fixture.output, 1)
                .unwrap()
                .unwrap();
        let output = serde_json::json!({
            "schema_version": 1,
            "kind": "deterministic_checkpoint_fixture",
            "description": "Four independent durable PoR replays of signed checkpoint ratings and one finalized equivocation authorization",
            "source_round": 1,
            "source_prefix_len": source.finalized_prefix_len,
            "source_prefix_hash": hex::encode(source.prefix_hash),
            "initial_weights": initial_weights,
            "rating_count": rating_count,
            "offender": "validator-2",
            "proof_blocks": claim.blocks.iter().map(|block| hex::encode(block.identity.content_hash)).collect::<Vec<_>>(),
            "penalties_hash": hex::encode(penalties_hash.unwrap()),
            "base_penalty_percent": 25,
            "observers": observers,
        });
        fs::write(path, serde_json::to_vec_pretty(&output).unwrap()).unwrap();
    }
}

fn demo_weights(
    state: &ReputationState,
    membership: &HashMap<NodeId, u64>,
) -> std::collections::BTreeMap<String, u64> {
    let authorized = (1..=4u8).map(node).collect::<Vec<_>>();
    let weights = cordial_por::authorized_validator_weights(
        state,
        &membership.keys().cloned().collect::<Vec<_>>(),
    )
    .unwrap();
    authorized
        .into_iter()
        .enumerate()
        .map(|(index, id)| (format!("validator-{}", index + 1), weights[&id]))
        .collect()
}

fn demo_retained(state: &ReputationState, offender: &NodeId) -> u64 {
    state
        .reputation_list()
        .entries
        .iter()
        .find(|entry| &entry.node_id == offender)
        .unwrap()
        .retained_reputation
}

#[test]
fn checkpoint_admission_requires_complete_set_and_is_order_independent() {
    let first = raw(2, 1);
    let same_offender = raw(2, 2);
    let other = raw(3, 3);
    let fixture = Fixture::new(vec![
        carrier(&first, 1),
        carrier(&other, 1),
        carrier(&same_offender, 1),
        carrier(&first, 1),
    ]);
    let completed = fixture.complete(&fixture.state, 1, 1);
    assert!(
        completed
            .clone()
            .with_authorized_equivocations(
                std::slice::from_ref(&first),
                &fixture.lace,
                &fixture.output,
                &fixture.membership,
                b"root"
            )
            .is_err()
    );
    let canonical = completed
        .clone()
        .with_finalized_equivocations(&fixture.lace, &fixture.output, &fixture.membership, b"root")
        .unwrap();
    let reordered = completed
        .with_authorized_equivocations(
            &[other, same_offender, first.clone(), first],
            &fixture.lace,
            &fixture.output,
            &fixture.membership,
            b"root",
        )
        .unwrap();
    assert_eq!(canonical, reordered);
    assert_eq!(canonical.penalties().unwrap().equivocations.len(), 2);
}

#[test]
fn checkpoint_admission_rejects_forgery_unapproved_proof_and_wrong_source() {
    let claim = raw(2, 1);
    let fixture = Fixture::new(vec![carrier(&claim, 1)]);
    let completed = fixture.complete(&fixture.state, 1, 1);
    let mut forged = claim.clone();
    forged.blocks[0].identity.signature.clear();
    for proof in [forged, raw(2, 99)] {
        assert!(
            completed
                .clone()
                .with_authorized_equivocations(
                    &[proof],
                    &fixture.lace,
                    &fixture.output,
                    &fixture.membership,
                    b"root"
                )
                .is_err()
        );
    }
    let mut other_prefix = fixture.output.clone();
    other_prefix.blocks.swap(0, 1);
    assert!(
        completed
            .clone()
            .with_finalized_equivocations(
                &fixture.lace,
                &other_prefix,
                &fixture.membership,
                b"root"
            )
            .is_err()
    );
    for absent in [node(1), node(2)] {
        let mut membership = fixture.membership.clone();
        membership.remove(&absent);
        let filtered = completed
            .clone()
            .with_finalized_equivocations(&fixture.lace, &fixture.output, &membership, b"root")
            .unwrap();
        assert!(filtered.penalties().unwrap().equivocations.is_empty());
    }
    let mut bad_carrier = carrier(&claim, 1);
    // An invalid or malformed finalized claim must not block progress.
    bad_carrier
        .message
        .truncate(cordial_por::POR_CHECKPOINT_EQUIVOCATION_AUTHORIZATION_DOMAIN.len());
    bad_carrier.message.push(1);
    let malformed = Fixture::new(vec![bad_carrier]);
    let closed = malformed.complete(&malformed.state, 1, 1);
    let authorized = closed
        .with_finalized_equivocations(
            &malformed.lace,
            &malformed.output,
            &malformed.membership,
            b"root",
        )
        .unwrap();
    assert!(authorized.penalties().unwrap().equivocations.is_empty());
}

#[test]
fn intents_only_authorize_the_window_that_contains_them() {
    let claim = raw(2, 1);
    // A round-two intent published in the first window must never authorize either window.
    let fixture = Fixture::new(vec![carrier(&claim, 2)]);
    let first = fixture.complete(&fixture.state, 1, 1);
    assert!(
        first
            .clone()
            .with_authorized_equivocations(
                std::slice::from_ref(&claim),
                &fixture.lace,
                &fixture.output,
                &fixture.membership,
                b"root"
            )
            .is_err()
    );
    let first = first
        .with_finalized_equivocations(&fixture.lace, &fixture.output, &fixture.membership, b"root")
        .unwrap();
    assert!(first.penalties().unwrap().equivocations.is_empty());
    let dir = tempfile::tempdir().unwrap();
    let mut state = DurablePorState::open(dir.path(), fixture.state.clone()).unwrap();
    state
        .apply_completed_checkpoint_round(&first, &fixture.config, b"root")
        .unwrap();
    let second = fixture.complete(state.state().unwrap(), 2, 1);
    assert!(
        second
            .with_authorized_equivocations(
                &[claim],
                &fixture.lace,
                &fixture.output,
                &fixture.membership,
                b"root"
            )
            .is_err()
    );
}

#[test]
fn admission_reextracts_carriers_from_the_trusted_dag_even_for_empty_submissions() {
    struct AcceptAll;
    impl crypto::CryptoVerifier for AcceptAll {
        type Error = String;
        fn verify_block(&self, _: &BlockContent, _: &[u8], _: &NodeId) -> Result<(), String> {
            Ok(())
        }
    }
    let real = Fixture::new(vec![carrier(&raw(2, 1), 1)]);
    let mut substituted = Blocklace::new();
    for id in &real.output.blocks {
        let mut block = real.lace.get(id).unwrap();
        // Model an untrusted closure caller preserving output identities while
        // dropping the carrier from the content in its own DAG.
        let mut payload = CordialBlockPayload::from_bytes(&block.content.payload).unwrap();
        payload.por_authorizations.clear();
        block.content.payload = payload.to_bytes();
        substituted.insert(block, &AcceptAll).unwrap();
    }
    let untrusted = Fixture {
        lace: substituted,
        output: real.output.clone(),
        state: real.state.clone(),
        membership: real.membership.clone(),
        config: real.config.clone(),
    };
    let completed = untrusted.complete(&untrusted.state, 1, 1);
    assert!(
        completed
            .with_finalized_equivocations(&real.lace, &real.output, &real.membership, b"root")
            .is_err()
    );
}

#[test]
fn invalid_finalized_claim_alongside_valid_claim_does_not_block_progress() {
    let valid_claim = raw(2, 1);
    let valid_carrier = carrier(&valid_claim, 1);
    let mut bad_carrier = carrier(&valid_claim, 1);
    bad_carrier
        .message
        .truncate(cordial_por::POR_CHECKPOINT_EQUIVOCATION_AUTHORIZATION_DOMAIN.len());
    bad_carrier.message.push(99);

    let fixture = Fixture::new(vec![valid_carrier, bad_carrier]);
    let completed = fixture.complete(&fixture.state, 1, 1);
    let authorized = completed
        .with_finalized_equivocations(&fixture.lace, &fixture.output, &fixture.membership, b"root")
        .unwrap();
    assert_eq!(authorized.penalties().unwrap().equivocations.len(), 1);
    assert_eq!(
        authorized.penalties().unwrap().equivocations[0].offender,
        node(2)
    );
    let again = authorized
        .clone()
        .with_finalized_equivocations(&fixture.lace, &fixture.output, &fixture.membership, b"root")
        .unwrap();
    assert_eq!(again, authorized);
    let dir = tempfile::tempdir().unwrap();
    let mut durable = DurablePorState::open(dir.path(), fixture.state.clone()).unwrap();
    durable
        .apply_completed_checkpoint_round(&again, &fixture.config, b"root")
        .unwrap();
    assert!(durable.state().unwrap().is_ejected(&node(2)));
}

#[test]
fn repeated_evidence_against_already_ejected_validator_is_idempotent() {
    let claim = raw(2, 1);
    let fixture = Fixture::two_windows(vec![carrier(&claim, 1)], vec![carrier(&claim, 2)]);
    let completed = fixture.complete(&fixture.state, 1, 1);
    let authorized = completed
        .with_finalized_equivocations(&fixture.lace, &fixture.output, &fixture.membership, b"root")
        .unwrap();

    let dir = tempfile::tempdir().unwrap();
    let mut state = DurablePorState::open(dir.path(), fixture.state.clone()).unwrap();
    state
        .apply_completed_checkpoint_round(&authorized, &fixture.config, b"root")
        .unwrap();
    assert!(state.state().unwrap().is_ejected(&node(2)));

    let excluded_before = state
        .state()
        .unwrap()
        .reputation_list()
        .entries
        .iter()
        .find(|entry| entry.node_id == node(2))
        .unwrap()
        .clone();
    // Now round 2 includes evidence against validator 2 (who is in membership with weight 0).
    let mut round2_membership = fixture.membership.clone();
    round2_membership.insert(node(2), 0); // Ejected validator has 0 weight

    let repeated = fixture.complete(state.state().unwrap(), 2, 1);
    // Explicit, idempotent handling: does not fail authorization, applies no second penalty.
    let authorized_round2 = repeated
        .with_finalized_equivocations(&fixture.lace, &fixture.output, &round2_membership, b"root")
        .unwrap();
    assert!(
        authorized_round2
            .penalties()
            .unwrap()
            .equivocations
            .is_empty()
    );
    state
        .apply_completed_checkpoint_round(&authorized_round2, &fixture.config, b"root")
        .unwrap();
    assert_eq!(state.state().unwrap().round(), 2);
    assert!(state.state().unwrap().is_ejected(&node(2)));
    assert_eq!(
        state
            .state()
            .unwrap()
            .reputation_list()
            .entries
            .iter()
            .find(|entry| entry.node_id == node(2))
            .unwrap(),
        &excluded_before
    );
}

#[test]
fn unbonded_claim_is_ignored_but_valid_authorization_cannot_be_omitted() {
    let good = raw(2, 1);
    let fixture = Fixture::new(vec![carrier(&good, 1), carrier(&raw(8, 1), 1)]);
    let completed = fixture.complete(&fixture.state, 1, 1);
    assert!(
        completed
            .clone()
            .with_authorized_equivocations(
                &[],
                &fixture.lace,
                &fixture.output,
                &fixture.membership,
                b"root"
            )
            .is_err()
    );
    let admitted = completed
        .with_finalized_equivocations(&fixture.lace, &fixture.output, &fixture.membership, b"root")
        .unwrap();
    assert_eq!(admitted.penalties().unwrap().equivocations.len(), 1);
    let again = admitted
        .clone()
        .with_finalized_equivocations(&fixture.lace, &fixture.output, &fixture.membership, b"root")
        .unwrap();
    assert_eq!(again, admitted);
    let dir = tempfile::tempdir().unwrap();
    let mut durable = DurablePorState::open(dir.path(), fixture.state.clone()).unwrap();
    durable
        .apply_completed_checkpoint_round(&again, &fixture.config, b"root")
        .unwrap();
    assert!(durable.state().unwrap().is_ejected(&node(2)));
}

#[test]
fn unavailable_ancestry_blocks_completion_until_recovered() {
    let parent = signed(4, vec![99], HashSet::new());
    let mut history = Blocklace::new();
    history
        .insert(parent.clone(), &crypto::Secp256k1Scheme)
        .unwrap();
    let claim = CordialEquivocationEvidence::new(
        node(2),
        1,
        [0, 1]
            .map(|tag| signed(2, vec![tag], HashSet::from([parent.identity.clone()])))
            .to_vec(),
    );
    let proof = verify_equivocation_evidence(&claim, &history).unwrap();
    let intent = PorEquivocationAuthorization {
        shard_id: b"root".to_vec(),
        source_finalized_wave: 0,
        round: 1,
        message: checkpoint_equivocation_authorization_payload(b"root", 1, &proof).unwrap(),
    };
    let mut fixture = Fixture::new(vec![intent]);
    let completed = fixture.complete(&fixture.state, 1, 1);
    assert!(
        completed
            .clone()
            .with_finalized_equivocations(
                &fixture.lace,
                &fixture.output,
                &fixture.membership,
                b"root"
            )
            .is_err()
    );
    fixture
        .lace
        .insert(parent, &crypto::Secp256k1Scheme)
        .unwrap();
    let recovered = completed
        .with_finalized_equivocations(&fixture.lace, &fixture.output, &fixture.membership, b"root")
        .unwrap();
    let full = fixture
        .complete(&fixture.state, 1, 1)
        .with_finalized_equivocations(&fixture.lace, &fixture.output, &fixture.membership, b"root")
        .unwrap();
    assert_eq!(recovered, full);
    assert_eq!(full.penalties().unwrap().equivocations.len(), 1);
}

#[test]
fn zero_weight_active_offender_is_ejected_and_classification_is_state_bound() {
    let mut fixture = Fixture::new(vec![carrier(&raw(2, 1), 1)]);
    fixture.membership.insert(node(2), 0);
    fixture.state.set_reputation(node(2), 0);
    assert!(!fixture.state.is_ejected(&node(2)));
    let completed = fixture.complete(&fixture.state, 1, 1);
    let admitted = completed
        .with_finalized_equivocations(&fixture.lace, &fixture.output, &fixture.membership, b"root")
        .unwrap();
    assert_eq!(admitted.penalties().unwrap().equivocations.len(), 1);
    let dir = tempfile::tempdir().unwrap();
    let mut durable = DurablePorState::open(dir.path(), fixture.state.clone()).unwrap();
    // A caller cannot use an exclusion from a different preceding state to
    // suppress this active validator's penalty at the durable boundary.
    let mut other_state = fixture.state.clone();
    other_state.eject_validator(&node(2)).unwrap();
    let suppressed = fixture
        .complete(&other_state, 1, 1)
        .with_finalized_equivocations(&fixture.lace, &fixture.output, &fixture.membership, b"root")
        .unwrap();
    assert!(suppressed.penalties().unwrap().equivocations.is_empty());
    assert!(
        durable
            .apply_completed_checkpoint_round(&suppressed, &fixture.config, b"root")
            .is_err()
    );
    assert_eq!(durable.state().unwrap(), &fixture.state);
    durable
        .apply_completed_checkpoint_round(&admitted, &fixture.config, b"root")
        .unwrap();
    assert!(durable.state().unwrap().is_ejected(&node(2)));
}

/// Stationary production, an evidence-backed shock, independently collected
/// observer batches, and recovery over a sustained window. Fixed-point scores
/// use scale 1000; one unit is the explicitly selected convergence tolerance.
#[test]
#[ignore = "long-running convergence simulation; run explicitly"]
fn multi_round_ranking_and_slashing_convergence() {
    const ROUNDS: usize = 128;
    const SHOCK: usize = 32;
    for (name, offenders, initial) in [
        ("honest", vec![], [1000, 1000, 1000, 1000]),
        ("unequal", vec![], [500, 1000, 500, 1000]),
        ("isolated", vec![2], [1000, 1000, 1000, 1000]),
        ("correlated", vec![2, 3], [1000, 1000, 1000, 1000]),
    ] {
        let _event_trace = por_event_trace::start(name, initial, ROUNDS);
        let mut windows = vec![vec![]; ROUNDS];
        windows[SHOCK - 1] = offenders
            .iter()
            .map(|&seed| carrier(&raw(seed, 9), SHOCK as u64))
            .collect();
        let mut fixture = Fixture::windows(windows);
        for (index, weight) in initial.into_iter().enumerate() {
            fixture.state.set_reputation(node(index as u8 + 1), weight);
        }
        let dirs: Vec<_> = (0..4).map(|_| tempfile::tempdir().unwrap()).collect();
        let mut stores: Vec<_> = dirs
            .iter()
            .map(|dir| DurablePorState::open(dir.path(), fixture.state.clone()).unwrap())
            .collect();
        let mut previous = demo_weights(&fixture.state, &fixture.membership);
        let mut stable = 0;
        let mut trace = Vec::new();
        for round in 1..=ROUNDS {
            trace_event!(
                "checkpoint_started",
                round as u64,
                serde_json::json!({"weights": previous, "window_start": (round - 1) * 16, "window_end": round * 16 - 1}),
            );
            let mut checkpoint = None;
            let pre_shock = stores[0].state().unwrap().clone();
            for (index, store) in stores.iter_mut().enumerate() {
                let complete = fixture
                    .complete(store.state().unwrap(), round as u64, index as u8 + 1)
                    .with_finalized_equivocations(
                        &fixture.lace,
                        &fixture.output,
                        &fixture.membership,
                        b"root",
                    )
                    .unwrap();
                let result = store
                    .apply_completed_checkpoint_round(&complete, &fixture.config, b"root")
                    .unwrap();
                let hash = cordial_por::reputation_block_hash(&result.block).unwrap();
                trace_event!(
                    "observer_committed",
                    round as u64,
                    serde_json::json!({
                        "observer": index + 1, "checkpoint": hex::encode(hash),
                        "weights": demo_weights(store.state().unwrap(), &fixture.membership),
                        "retained": (1..=4u8).map(|seed| (format!("validator-{seed}"), demo_retained(store.state().unwrap(), &node(seed)))).collect::<std::collections::BTreeMap<_,_>>(),
                        "excluded": (1..=4u8).filter(|seed| store.state().unwrap().is_ejected(&node(*seed))).map(|seed| format!("validator-{seed}")).collect::<Vec<_>>(),
                    }),
                );
                assert_eq!(
                    *checkpoint.get_or_insert(hash),
                    hash,
                    "{name}, round {round}"
                );
            }
            let weights = demo_weights(stores[0].state().unwrap(), &fixture.membership);
            let delta = weights
                .iter()
                .map(|(id, weight)| weight.abs_diff(previous[id]))
                .max()
                .unwrap();
            let total: u64 = weights.values().sum();
            assert!(total > 0, "{name}: active weight collapsed");
            let previous_total: u64 = previous.values().sum();
            let share_delta = weights
                .iter()
                .map(|(id, weight)| {
                    let current = u128::from(*weight) * 1_000_000 / u128::from(total);
                    let prior = u128::from(previous[id]) * 1_000_000 / u128::from(previous_total);
                    current.abs_diff(prior)
                })
                .max()
                .unwrap();
            let ranking = |values: &std::collections::BTreeMap<String, u64>| {
                let mut rows: Vec<_> = values
                    .iter()
                    .map(|(id, weight)| (id.clone(), *weight))
                    .collect();
                rows.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
                rows.into_iter().map(|(id, _)| id).collect::<Vec<_>>()
            };
            let rank_stable = ranking(&weights) == ranking(&previous);
            stable = if round > SHOCK && delta <= 1 && share_delta <= 1000 && rank_stable {
                stable + 1
            } else {
                0
            };
            for (observer_index, store) in stores.iter().enumerate() {
                assert_eq!(store.state().unwrap(), stores[0].state().unwrap());
                for &offender in &offenders {
                    if round >= SHOCK {
                        assert!(store.state().unwrap().is_ejected(&node(offender)));
                        assert_eq!(weights[&format!("validator-{offender}")], 0);
                        if round == SHOCK {
                            let prior = pre_shock
                                .reputation_list()
                                .entries
                                .iter()
                                .find(|entry| entry.node_id == node(offender))
                                .unwrap()
                                .reputation;
                            // The actual tier is derived from the pre-shock snapshot.
                            let offending: u64 = pre_shock
                                .reputation_list()
                                .entries
                                .iter()
                                .filter(|entry| {
                                    offenders.iter().any(|id| node(*id) == entry.node_id)
                                })
                                .map(|entry| entry.reputation)
                                .sum();
                            let total: u64 = pre_shock
                                .reputation_list()
                                .entries
                                .iter()
                                .filter(|entry| !entry.is_excluded)
                                .map(|entry| entry.reputation)
                                .sum();
                            let penalty = cordial_por::compute_slash_penalty(
                                offending,
                                total,
                                &fixture.config,
                            )
                            .unwrap();
                            assert_eq!(
                                penalty,
                                if name == "correlated" {
                                    fixture.config.scale
                                } else {
                                    fixture.config.base_slash_penalty
                                },
                                "{name}: scenario must exercise the intended tier",
                            );
                            let expected = cordial_por::apply_slash_to_reputation(
                                prior,
                                penalty,
                                &fixture.config,
                            )
                            .unwrap();
                            assert_eq!(
                                demo_retained(store.state().unwrap(), &node(offender)),
                                expected
                            );
                            trace_event!(
                                "penalty_verified",
                                round as u64,
                                serde_json::json!({
                                    "observer": observer_index + 1, "validator": format!("validator-{offender}"),
                                    "prior": prior, "offending_weight": offending, "total_weight": total,
                                    "penalty": penalty, "scale": fixture.config.scale, "retained": expected,
                                })
                            );
                        }
                    }
                }
            }
            let mut trace_events = Vec::new();
            if round == SHOCK && !offenders.is_empty() {
                trace_events.push("slash");
            }
            // These records follow the restart after the preceding checkpoint.
            if round == SHOCK + 1 || round == SHOCK + 2 {
                trace_events.push("restart");
            }
            trace.push(serde_json::json!({
                "scenario": name, "round": round, "weights": weights,
                "checkpoint": hex::encode(checkpoint.unwrap()), "max_delta": delta,
                "share_delta_ppm": share_delta, "rank_stable": rank_stable,
                "total_weight": total, "stable_rounds": stable,
                "observer_agreement": true,
                "events": trace_events,
                "entries": stores[0].state().unwrap().reputation_list().entries.iter().map(|entry|
                    serde_json::json!({
                        "node": hex::encode(&entry.node_id.0),
                        "validator": format!("validator-{}", (1..=4u8).find(|seed| node(*seed) == entry.node_id).unwrap()),
                        "reputation": entry.reputation,
                        "retained_reputation": entry.retained_reputation,
                        "excluded": entry.is_excluded,
                    })
                ).collect::<Vec<_>>(),
            }));
            trace_event!(
                "round_completed",
                round as u64,
                trace.last().unwrap().clone(),
            );
            previous = weights;
            if round == SHOCK || round == SHOCK + 1 {
                // Recover every observer, not just the first one.
                let expected: Vec<_> = stores
                    .iter()
                    .map(|store| store.state().unwrap().clone())
                    .collect();
                drop(stores);
                stores = dirs
                    .iter()
                    .enumerate()
                    .map(|(index, dir)| {
                        let recovered =
                            DurablePorState::open(dir.path(), ReputationState::new(999)).unwrap();
                        assert_eq!(recovered.state().unwrap(), &expected[index]);
                        recovered
                    })
                    .collect();
                trace_event!(
                    "observers_restarted",
                    round as u64,
                    serde_json::json!({"observers": [1,2,3,4], "state_verified": true}),
                );
            }
        }
        assert!(
            stable >= 8,
            "{name}: no sustained convergence by round {ROUNDS}"
        );
        trace_event!(
            "run_completed",
            ROUNDS as u64,
            serde_json::json!({"status": "passed", "stable_rounds": stable}),
        );
        if let Ok(directory) = std::env::var("POR_CONVERGENCE_OUTPUT") {
            fs::create_dir_all(&directory).unwrap();
            fs::write(
                std::path::Path::new(&directory).join(format!("{name}.json")),
                serde_json::to_vec_pretty(&serde_json::json!({
                    "schema_version": 1,
                    "kind": "por_convergence_trace",
                    "scenario": name,
                    "observer_count": 4,
                    "scale": fixture.config.scale,
                    "thresholds": {
                        "score_delta": 1,
                        "share_delta_ppm": 1000,
                        "stable_rounds": 8,
                    },
                    "result": { "status": "passed", "completed_rounds": ROUNDS },
                    "rounds": trace,
                }))
                .unwrap(),
            )
            .unwrap();
        }
    }
}

#[test]
fn four_observer_slash_threshold_boundaries() {
    for (offending_weight, expected_retained) in [(299, 224), (300, 225), (301, 0)] {
        let mut fixture = Fixture::new(vec![carrier(&raw(2, 9), 1)]);
        for (seed, weight) in [
            (1, 400),
            (2, offending_weight),
            (3, 100),
            (4, 500 - offending_weight),
        ] {
            fixture.state.set_reputation(node(seed), weight);
            fixture.membership.insert(node(seed), weight);
        }
        let mut expected = None;
        for observer in 1..=4 {
            let complete = fixture
                .complete(&fixture.state, 1, observer)
                .with_finalized_equivocations(
                    &fixture.lace,
                    &fixture.output,
                    &fixture.membership,
                    b"root",
                )
                .unwrap();
            let dir = tempfile::tempdir().unwrap();
            let mut store = DurablePorState::open(dir.path(), fixture.state.clone()).unwrap();
            let result = store
                .apply_completed_checkpoint_round(&complete, &fixture.config, b"root")
                .unwrap();
            let hash = cordial_por::reputation_block_hash(&result.block).unwrap();
            assert_eq!(*expected.get_or_insert(hash), hash);
            assert!(store.state().unwrap().is_ejected(&node(2)));
            assert_eq!(
                demo_retained(store.state().unwrap(), &node(2)),
                expected_retained
            );
            assert_eq!(
                demo_weights(store.state().unwrap(), &fixture.membership)["validator-2"],
                0
            );
        }
    }
}

/// Real LiveIngress finality and PorRuntime cutover, with independent durable
/// observers, four consecutive epochs, a slash in checkpoint two, and restart.
/// This exercises the adapter boundary; it is not a host-chain slash deploy.
#[test]
fn four_runtime_epochs_preserve_finality_through_slash_and_restart() {
    use cordial_f1r3node_adapter::{
        grpc_ingest::BlocklaceAdapter,
        live_ingress::{LiveIngress, PorWeightActivationError},
        por::{
            PorRuntime, PorRuntimeError, PorWeightEpochPolicy, PorWeightEpochRecordError,
            PorWeightEpochStore,
        },
        shard_conf::CasperShardConf,
        snapshot::CORDIAL_WAVELENGTH,
    };
    struct AcceptBlocks;
    impl BlocklaceAdapter<BlockIdentity> for AcceptBlocks {
        fn on_block(&mut self, _: Block) -> anyhow::Result<()> {
            Ok(())
        }
    }
    let dirs: Vec<_> = (0..4).map(|_| tempfile::tempdir().unwrap()).collect();
    let mut initial = ReputationState::new(0);
    let membership: HashMap<_, _> = (1..=4)
        .map(|seed| {
            initial.set_reputation(node(seed), 200);
            (node(seed), 200)
        })
        .collect();
    let config = PorConfig::new(1000, 200);
    let open = |index: usize| {
        PorRuntime::open(
            dirs[index].path(),
            initial.clone(),
            LiveIngress::with_consensus_view(
                AcceptBlocks,
                membership.clone(),
                CasperShardConf::default(),
                "root",
            ),
            config.clone(),
            b"root",
            CORDIAL_WAVELENGTH,
        )
        .unwrap()
    };
    let mut runtimes: Vec<_> = (0..4).map(&open).collect();
    let mut history: Vec<Vec<Block>> = Vec::new();
    let mut predecessors = HashSet::new();
    let policy = PorWeightEpochPolicy::new(3).unwrap();
    let claim = raw(2, 77);
    let mut prior_prefix = Vec::new();
    let mut restart_prefix = None;
    let mut activated = 0;
    let mut saw_deferred = false;
    for layer_index in 0..120u64 {
        let layer: Vec<_> = (1..=4)
            .map(|seed| {
                let payload = CordialBlockPayload {
                    // The same valid claim is present in the source window of
                    // checkpoint two. Other checkpoint windows ignore this intent.
                    por_authorizations: vec![carrier(&claim, 2)],
                    state: BlockState {
                        pre_state_hash: vec![],
                        post_state_hash: vec![],
                        bonds: vec![],
                        block_number: layer_index,
                    },
                    deploys: vec![],
                    rejected_deploys: vec![],
                    system_deploys: vec![],
                }
                .to_bytes();
                signed(seed, payload, predecessors.clone())
            })
            .collect();
        predecessors = layer.iter().map(|block| block.identity.clone()).collect();
        history.push(layer.clone());
        let mut outputs = Vec::new();
        for (index, runtime) in runtimes.iter_mut().enumerate() {
            let mut delivery = layer.clone();
            delivery.rotate_left(index);
            if index % 2 != 0 {
                delivery.reverse();
            }
            for block in delivery {
                runtime.ingress_mut().ingest_trusted_block(block).unwrap();
            }
            outputs.push(runtime.publish_finalized_output().unwrap());
        }
        for output in &outputs {
            assert_eq!(output.blocks, outputs[0].blocks);
            assert!(output.blocks.starts_with(&prior_prefix));
        }
        prior_prefix = outputs[0].blocks.clone();
        if activated == 4 {
            if prior_prefix.len() > restart_prefix.unwrap() {
                break;
            }
            continue;
        }
        let round = activated + 1;
        if finalized_prefix_checkpoint(&outputs[0], round)
            .unwrap()
            .is_none()
        {
            continue;
        }
        let mut committed = Vec::new();
        for (index, runtime) in runtimes.iter_mut().enumerate() {
            let mut lace = Blocklace::new();
            for block in history.iter().flatten() {
                lace.insert(block.clone(), &crypto::Secp256k1Scheme)
                    .unwrap();
            }
            let fixture = Fixture {
                lace,
                output: outputs[index].clone(),
                state: runtime.state().state().unwrap().clone(),
                // Key loading uses initial authorized membership. Runtime
                // admission independently uses its current activated weights.
                membership: membership.clone(),
                config: config.clone(),
            };
            let complete = fixture.complete(&fixture.state, round, index as u8 + 1);
            let authorized = runtime
                .authorize_checkpoint_equivocations(complete)
                .unwrap();
            match runtime.commit_completed_checkpoint_round_with_epoch(&authorized, policy) {
                Ok(_) => committed.push(true),
                Err(PorRuntimeError::FutureEpoch(PorWeightEpochRecordError::Preview(
                    PorWeightActivationError::EpochBoundaryWavePending { .. },
                ))) => {
                    saw_deferred = true;
                    committed.push(false);
                }
                Err(error) => panic!("round {round}: {error}"),
            }
        }
        assert!(committed.iter().all(|value| *value == committed[0]));
        if !committed[0] {
            continue;
        }
        activated = round;
        let state = runtimes[0].state().state().unwrap().clone();
        let weights = runtimes[0].ingress().bonds().clone();
        for runtime in &mut runtimes {
            assert_eq!(runtime.state().state().unwrap(), &state);
            assert_eq!(runtime.ingress().bonds(), &weights);
            assert_eq!(runtime.active_future_epoch().unwrap().por_round(), round);
            assert!(
                runtime
                    .publish_finalized_output()
                    .unwrap()
                    .blocks
                    .starts_with(&prior_prefix)
            );
            if round >= 2 {
                assert!(runtime.state().state().unwrap().is_ejected(&node(2)));
                assert_eq!(runtime.ingress().bonds()[&node(2)], 0);
            }
        }
        for dir in &dirs {
            assert_eq!(
                PorWeightEpochStore::open(dir.path())
                    .unwrap()
                    .restore_journal()
                    .unwrap()
                    .len(),
                round as usize
            );
        }
        // Empty-ingress recovery must not publish until retained DAG replay.
        drop(runtimes);
        runtimes = (0..4).map(&open).collect();
        for runtime in &mut runtimes {
            assert!(runtime.publish_finalized_output().is_err());
            for layer in &history {
                for block in layer {
                    runtime
                        .ingress_mut()
                        .ingest_trusted_block(block.clone())
                        .unwrap();
                }
            }
            let recovered = runtime.publish_finalized_output().unwrap();
            assert!(recovered.blocks.starts_with(&prior_prefix));
            assert_eq!(runtime.state().state().unwrap(), &state);
            assert_eq!(runtime.ingress().bonds(), &weights);
        }
        if activated == 4 {
            restart_prefix = Some(prior_prefix.len());
        }
    }
    assert!(
        saw_deferred,
        "exercise pending activation before the fixed boundary"
    );
    assert_eq!(
        activated, 4,
        "all four epochs must activate within the horizon"
    );
    assert!(
        prior_prefix.len() > restart_prefix.unwrap(),
        "finality must advance after restart"
    );
}
