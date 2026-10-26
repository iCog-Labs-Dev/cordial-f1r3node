//! Checkpoint carriers must be verified, complete, and confined to their window.
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
        let mut lace = Blocklace::new();
        let mut ids: Vec<BlockIdentity> = vec![];
        for index in 0..32 {
            let payload = CordialBlockPayload {
                por_authorizations: if index == 8 {
                    carriers.clone()
                } else if index == 24 {
                    later.clone()
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
            ids.push(block.identity);
        }
        let output = OrderedFinalizedOutput::new(ids.clone(), ids.last().cloned(), 3, 32, 32)
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
        for seed in 1..=4 {
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
            for rating in batch.ratings {
                publisher
                    .publish(
                        &BlockProductionRatingEnvelopeV2::new(checkpoint.rating_source(), rating)
                            .unwrap()
                            .encode(),
                    )
                    .unwrap();
            }
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
            for rating in stale.ratings {
                publisher
                    .publish(
                        &BlockProductionRatingEnvelopeV2::new(checkpoint.rating_source(), rating)
                            .unwrap()
                            .encode(),
                    )
                    .unwrap();
            }
        }
        close_checkpoint_rating_exchange_if_complete(
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
        .unwrap()
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
