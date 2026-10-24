//! Cross-crate slashing lifecycle at the adapter preparation and PoR boundaries.
//!
//! Host authorization/execution and capital transfers are outside this harness.
//! Its formatter uses synthetic host metadata only to inspect prepared requests;
//! a test coordinator verifies the prepared proofs and authorizes them with an
//! explicit fixture policy before supplying penalty events to the real PoR APIs.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::{Arc, Mutex};

use cordial_f1r3node_adapter::crypto_bridge::{F1r3flyCryptoAdapter, SigAlgorithm};
use cordial_f1r3node_adapter::por::ratings::{
    build_verified_rating_batch, sign_admitted_interaction,
};
use cordial_f1r3node_adapter::proposer::{
    BlockSigner, CordialProposer, DisseminationTipSelector, EvidencePoolSource, ExecutionEngine,
    RecordingBroadcaster, Secp256k1BlockSigner,
};
use cordial_f1r3node_adapter::slashing::{FormattedSlashEvidence, SlashDeployFormatter};
use cordial_f1r3node_adapter::tiered_slashing::{
    ExecutionBatchItem, PreparedExecutionRequest, TieredSlashPolicy,
};
use cordial_miners_core::consensus::{
    CordialEquivocationEvidence, CordialEvidencePool, EvidencePool, InvalidBlock, ValidationConfig,
    VerifiedEquivocationEvidence, record_rejected_equivocation, validate_block,
    verify_equivocation_evidence,
};
use cordial_miners_core::crypto::CryptoVerifier;
use cordial_miners_core::execution::{
    BlockState, Bond, CordialBlockPayload, DeployPool, DeployPoolConfig, ExecutionRequest,
    ExecutionResult, RuntimeError,
};
use cordial_miners_core::{Block, BlockContent, BlockIdentity, Blocklace, NodeId};
use cordial_por::{
    EquivocationAuthorizationContext, EquivocationAuthorizationPolicy, EquivocationPenalty,
    InteractionEvidence, InteractionKind, PorConfig, PorError, RatingRecord, ReputationBlock,
    ReputationBlockContext, ReputationEntry, ReputationPenaltyEvents, ReputationState,
    ReputationVector, admit_interaction_evidence, authorized_validator_weights,
    build_reputation_block_with_penalties, decode_reputation_state_snapshot,
    encode_reputation_state_snapshot, replay_reputation_transition,
    replay_reputation_transition_with_penalties, reputation_list_commitment, reputation_weights,
    verify_reputation_transition, verify_reputation_transition_with_penalties,
};
use models::casper::{
    ProcessedSystemDeployProto, SlashSystemDeployDataProto, SystemDeployDataProto,
    system_deploy_data_proto::SystemDeploy,
};
use prost::Message;

const EVIDENCE_ROUND: u64 = 0;
const FINALIZED_WAVE: u64 = 0;
const PENALTY_ROUND: u64 = 1;
const INITIAL_REPUTATION: u64 = 1000;

fn node(seed: u8) -> NodeId {
    let key = k256::ecdsa::SigningKey::from_slice(&[seed; 32]).unwrap();
    NodeId(
        key.verifying_key()
            .to_encoded_point(true)
            .as_bytes()
            .to_vec(),
    )
}

fn verifier() -> F1r3flyCryptoAdapter {
    F1r3flyCryptoAdapter::new(SigAlgorithm::Secp256k1)
}

fn previous(state: &ReputationState) -> ReputationVector {
    ReputationVector {
        round: state.round(),
        values: state.reputation_list().entries.clone(),
    }
}

fn entry(state: &ReputationState, validator: &NodeId) -> ReputationEntry {
    state
        .reputation_list()
        .entries
        .iter()
        .find(|entry| &entry.node_id == validator)
        .unwrap()
        .clone()
}

// This formatter never authorizes a production host slash. In particular, its
// Cordial content hash and epoch zero are only placeholders for preparation.
struct FixtureFormatter;

impl SlashDeployFormatter<NodeId, Block, BlockIdentity> for FixtureFormatter {
    fn to_slash_system_deploys(
        &self,
        evidence: &[CordialEquivocationEvidence],
    ) -> anyhow::Result<Vec<Vec<u8>>> {
        Ok(evidence
            .iter()
            .map(|record| {
                let hash = record
                    .blocks
                    .iter()
                    .map(|b| b.identity.content_hash)
                    .min()
                    .unwrap();
                ProcessedSystemDeployProto {
                    system_deploy: Some(SystemDeployDataProto {
                        system_deploy: Some(SystemDeploy::SlashSystemDeploy(
                            SlashSystemDeployDataProto {
                                invalid_block_hash: hash.to_vec().into(),
                                issuer_public_key: node(4).0.into(),
                                target_activation_epoch: 0,
                            },
                        )),
                    }),
                    deploy_log: vec![],
                    error_msg: String::new(),
                }
                .encode_to_vec()
            })
            .collect())
    }

    fn to_tiered_slash_deploys(
        &self,
        records: &[CordialEquivocationEvidence],
        penalty_ratio_bps: u32,
    ) -> anyhow::Result<Vec<FormattedSlashEvidence>> {
        records
            .iter()
            .zip(self.to_slash_system_deploys(records)?)
            .map(|(record, slash_system_deploy)| {
                let mut blocks = record.blocks.clone();
                blocks.sort_by(|a, b| a.identity.cmp(&b.identity));
                Ok(FormattedSlashEvidence {
                    penalty_ratio_bps,
                    slash_system_deploy,
                    conflicting_block_bytes: blocks
                        .iter()
                        .map(bincode::serialize)
                        .collect::<Result<_, _>>()?,
                })
            })
            .collect()
    }
}

#[derive(Clone, Default)]
struct PreparationCapture(Arc<Mutex<Option<PreparedExecutionRequest>>>);

impl ExecutionEngine for PreparationCapture {
    fn execute(&mut self, _: ExecutionRequest) -> Result<ExecutionResult, RuntimeError> {
        panic!("the tiered path must preserve its prepared request");
    }

    fn execute_prepared(
        &mut self,
        request: PreparedExecutionRequest,
    ) -> Result<ExecutionResult, RuntimeError> {
        let bonds = request.legacy_request().bonds.clone();
        *self.0.lock().unwrap() = Some(request);
        Ok(ExecutionResult {
            post_state_hash: vec![0xaa; 32],
            processed_deploys: vec![],
            rejected_deploys: vec![],
            system_deploys: vec![],
            new_bonds: bonds,
        })
    }
}

struct Fixture {
    config: PorConfig,
    state: ReputationState,
    validators: Vec<NodeId>,
    blocklace: Blocklace,
    evidence: CordialEvidencePool,
}

struct SlashRound {
    block: ReputationBlock,
    events: ReputationPenaltyEvents,
}

// Approves only the coordinator's exact verified proof and context. This fixture
// policy does not establish production finalization or host authorization.
struct FixtureAuthorization<'a> {
    context: &'a EquivocationAuthorizationContext,
    proof: &'a VerifiedEquivocationEvidence,
}

impl EquivocationAuthorizationPolicy for FixtureAuthorization<'_> {
    fn authorize(
        &self,
        context: &EquivocationAuthorizationContext,
        proof: &VerifiedEquivocationEvidence,
    ) -> Result<(), PorError> {
        if context == self.context && proof == self.proof {
            Ok(())
        } else {
            Err(PorError::InvalidPenaltyEvents(
                "proof or context was not approved by the fixture coordinator".into(),
            ))
        }
    }
}

impl Fixture {
    fn new(offenders: &[u8]) -> Self {
        let config = PorConfig::new(1000, INITIAL_REPUTATION);
        let mut state = ReputationState::new(0);
        let validators: Vec<_> = (1..=4).map(node).collect();
        for validator in &validators {
            state.set_reputation(validator.clone(), INITIAL_REPUTATION);
        }
        let weights = authorized_validator_weights(&state, &validators).unwrap();
        let bonds = state
            .reputation_list()
            .entries
            .iter()
            .map(|e| Bond {
                validator: e.node_id.clone(),
                stake: e.reputation,
            })
            .collect::<Vec<_>>();
        let signed_block = |seed, tag| {
            let payload = CordialBlockPayload {
                por_authorizations: vec![],
                state: BlockState {
                    pre_state_hash: vec![],
                    post_state_hash: vec![tag; 32],
                    bonds: bonds.clone(),
                    block_number: 0,
                },
                deploys: vec![],
                rejected_deploys: vec![],
                system_deploys: vec![],
            };
            let content = BlockContent {
                payload: payload.to_bytes(),
                predecessors: HashSet::new(),
            };
            let identity = Secp256k1BlockSigner::new(vec![seed; 32])
                .sign_block(&content, &node(seed))
                .unwrap();
            Block { identity, content }
        };
        let mut blocklace = Blocklace::new();
        for seed in 1..=4 {
            blocklace
                .insert(signed_block(seed, seed), &verifier())
                .unwrap();
        }
        let mut evidence = CordialEvidencePool::new();
        for &seed in offenders {
            let rejected = signed_block(seed, seed + 10);
            verifier()
                .verify_block(&rejected.content, &rejected.identity.signature, &node(seed))
                .unwrap();
            let validation = validate_block(
                &rejected,
                &blocklace,
                &weights,
                &ValidationConfig::default(),
            );
            assert!(
                validation
                    .errors()
                    .iter()
                    .any(|e| matches!(e, InvalidBlock::Equivocation { .. }))
            );
            assert!(record_rejected_equivocation(
                &rejected,
                validation.errors(),
                &blocklace,
                &mut evidence
            ));
            assert!(!record_rejected_equivocation(
                &rejected,
                validation.errors(),
                &blocklace,
                &mut evidence
            ));
        }
        assert_eq!(evidence.len(), offenders.len());
        Self {
            config,
            state,
            validators,
            blocklace,
            evidence,
        }
    }

    fn prepared_events(&self, expected_ratio: u32) -> ReputationPenaltyEvents {
        let weights = authorized_validator_weights(&self.state, &self.validators).unwrap();
        let capture = PreparationCapture::default();
        let mut proposer = CordialProposer::new(
            DisseminationTipSelector,
            capture.clone(),
            Secp256k1BlockSigner::new(vec![4; 32]),
            RecordingBroadcaster::new(),
            node(4),
            weights.clone(),
            DeployPoolConfig::default(),
        )
        .with_slashing(
            EvidencePoolSource::new(&self.evidence, self.validators.clone()),
            FixtureFormatter,
        )
        .with_slash_policy(
            TieredSlashPolicy::new(self.config.clone())
                .unwrap()
                .with_round_weights(EVIDENCE_ROUND, weights)
                .unwrap(),
        );
        proposer
            .propose_at_time(
                &self.blocklace,
                &DeployPool::new(DeployPoolConfig::default()),
                100,
            )
            .unwrap();
        let captured = capture.0.lock().unwrap();
        let batch = captured.as_ref().unwrap().batch();
        assert!(matches!(batch.last(), Some(ExecutionBatchItem::CloseBlock)));
        assert_eq!(batch.len(), self.evidence.len() + 1);
        let mut events = BTreeMap::new();
        for item in &batch[..batch.len() - 1] {
            let ExecutionBatchItem::SlashSystemDeploy {
                validator,
                round,
                evidence,
            } = item
            else {
                panic!("expected prepared slash prefix");
            };
            assert_eq!(*round, EVIDENCE_ROUND);
            assert_eq!(evidence.penalty_ratio_bps, expected_ratio);
            let original = self.evidence.evidence_for(validator);
            assert_eq!(original.len(), 1);
            let proofs: Vec<Block> = evidence
                .conflicting_block_bytes
                .iter()
                .map(|bytes| bincode::deserialize(bytes).unwrap())
                .collect();
            assert_eq!(proofs, original[0].blocks);
            let proof = verify_equivocation_evidence(
                &CordialEquivocationEvidence::new(validator.clone(), *round, proofs),
                &self.blocklace,
            )
            .unwrap();
            let authorization_context = EquivocationAuthorizationContext {
                shard_id: b"root".to_vec(),
                source_finalized_wave: FINALIZED_WAVE,
                round: PENALTY_ROUND,
            };
            let policy = FixtureAuthorization {
                context: &authorization_context,
                proof: &proof,
            };
            let penalty =
                EquivocationPenalty::authorize(&proof, authorization_context.clone(), &policy)
                    .unwrap();
            assert!(events.insert(validator.clone(), penalty).is_none());
        }
        assert_eq!(
            events.keys().collect::<Vec<_>>(),
            self.evidence
                .equivocating_validators_in_round(EVIDENCE_ROUND)
        );
        ReputationPenaltyEvents {
            round: PENALTY_ROUND,
            equivocations: events.into_values().collect(),
            inactivity: vec![],
        }
    }

    fn slash(&mut self, ratio: u32, ratings: &[RatingRecord]) -> SlashRound {
        let events = self.prepared_events(ratio);
        let block = build_round(
            &self.state,
            ratings,
            Some(&events),
            FINALIZED_WAVE,
            &self.config,
        );
        verify_reputation_transition_with_penalties(
            &previous(&self.state),
            ratings,
            &block,
            context(&self.state, FINALIZED_WAVE),
            &self.config,
            Some(&events),
        )
        .unwrap();
        self.state
            .apply_reputation_block_with_penalties(
                b"root",
                FINALIZED_WAVE,
                ratings,
                block.clone(),
                &self.config,
                Some(&events),
            )
            .unwrap();
        assert_eq!(self.state.round(), PENALTY_ROUND);
        assert_eq!(self.state.latest_block(), Some(&block));
        SlashRound { block, events }
    }

    fn assert_offender(&self, seed: u8, retained: u64) {
        let validator = node(seed);
        let entry = entry(&self.state, &validator);
        assert_eq!(entry.reputation, 0);
        assert_eq!(entry.retained_reputation, retained);
        assert!(entry.is_excluded);
        assert!(self.state.is_ejected(&validator));
        assert!(!reputation_weights(&self.state).contains_key(&validator));
        assert_eq!(
            authorized_validator_weights(&self.state, &self.validators).unwrap()[&validator],
            0
        );
    }

    fn ratings(&self) -> Vec<RatingRecord> {
        [1, 2]
            .into_iter()
            .map(|seed| {
                let interaction = admit_interaction_evidence(
                    InteractionEvidence {
                        finalized_wave: FINALIZED_WAVE,
                        round: PENALTY_ROUND,
                        kind: InteractionKind::BlockProduction,
                        rater: node(4),
                        recipient: node(seed),
                        evidence_ref: self
                            .blocklace
                            .dom()
                            .iter()
                            .find(|id| id.creator == node(seed))
                            .unwrap()
                            .content_hash
                            .to_vec(),
                    },
                    &self.state,
                )
                .unwrap();
                sign_admitted_interaction(interaction, &self.config, &[4; 32]).unwrap()
            })
            .collect()
    }
}

fn context(state: &ReputationState, finalized_wave: u64) -> ReputationBlockContext<'_> {
    ReputationBlockContext {
        shard_id: b"root",
        source_finalized_wave: finalized_wave,
        previous_block: state.latest_block(),
    }
}

fn build_round(
    state: &ReputationState,
    ratings: &[RatingRecord],
    events: Option<&ReputationPenaltyEvents>,
    finalized_wave: u64,
    config: &PorConfig,
) -> ReputationBlock {
    let round = finalized_wave + 1;
    let batch = build_verified_rating_batch(round, ratings.to_vec(), config).unwrap();
    let list = replay_reputation_transition_with_penalties(
        &previous(state),
        &batch.ratings,
        round,
        config,
        events,
    )
    .unwrap();
    build_reputation_block_with_penalties(
        context(state, finalized_wave),
        &batch,
        list,
        config,
        events,
    )
    .unwrap()
}

#[test]
fn isolated_fault_retains_75_percent_and_zeroes_only_the_offenders_weight() {
    let mut fixture = Fixture::new(&[1]);
    fixture.slash(2500, &[]);
    fixture.assert_offender(1, 750);
    let exported = reputation_weights(&fixture.state);
    assert_eq!(exported.len(), 3);
    for seed in 2..=4 {
        assert_eq!(
            entry(&fixture.state, &node(seed)),
            ReputationEntry::new(node(seed), 1000)
        );
        assert_eq!(exported[&node(seed)], 1000);
    }
}

#[test]
fn three_correlated_offenders_lose_all_capital_and_leave_the_innocent_unchanged() {
    let mut fixture = Fixture::new(&[1, 2, 3]);
    fixture.slash(10000, &[]);
    for seed in 1..=3 {
        fixture.assert_offender(seed, 0);
    }
    assert_eq!(
        entry(&fixture.state, &node(4)),
        ReputationEntry::new(node(4), 1000)
    );
    assert_eq!(
        reputation_weights(&fixture.state),
        HashMap::from([(node(4), 1000)])
    );
}

#[test]
fn excluded_key_cannot_be_revived_by_vectors_or_direct_assignment() {
    let mut fixture = Fixture::new(&[1]);
    fixture.slash(2500, &[]);
    let mut forged = previous(&fixture.state);
    forged.round += 1;
    *forged
        .values
        .iter_mut()
        .find(|entry| entry.node_id == node(1))
        .unwrap() = ReputationEntry::new(node(1), 9000);
    fixture.state.apply_reputation_vector(forged).unwrap();
    fixture.state.set_reputation(node(1), 9000);
    fixture.assert_offender(1, 750);
    let mut omitted = previous(&fixture.state);
    omitted.values.retain(|entry| entry.node_id != node(1));
    fixture.state.apply_reputation_vector(omitted).unwrap();
    fixture.state.set_reputation(node(1), 9000);
    fixture.assert_offender(1, 750);
}

#[test]
fn exclusion_and_retained_balance_survive_a_later_audited_round_and_restart() {
    let mut fixture = Fixture::new(&[1]);
    fixture.slash(2500, &[]);
    let next = build_round(&fixture.state, &[], None, 1, &fixture.config);
    fixture
        .state
        .apply_reputation_block(b"root", 1, &[], next, &fixture.config)
        .unwrap();
    let restored = decode_reputation_state_snapshot(
        &encode_reputation_state_snapshot(&fixture.state).unwrap(),
    )
    .unwrap();
    assert_eq!(restored, fixture.state);
    fixture.state = restored;
    fixture.assert_offender(1, 750);
}

#[test]
fn fresh_key_registration_exports_the_surviving_balance_without_reviving_the_old_key() {
    let mut fixture = Fixture::new(&[1]);
    fixture.slash(2500, &[]);
    let retained = entry(&fixture.state, &node(1)).retained_reputation;
    assert_eq!(retained, 750);
    // State-level registration supplied by an external lifecycle. This neither
    // authorizes a transfer nor consumes the old retained balance.
    fixture.state.set_reputation(node(5), retained);
    assert_eq!(
        entry(&fixture.state, &node(5)),
        ReputationEntry::new(node(5), 750)
    );
    assert!(!fixture.state.is_ejected(&node(5)));
    assert_eq!(reputation_weights(&fixture.state)[&node(5)], 750);
    fixture.assert_offender(1, 750);
    assert!(
        !authorized_validator_weights(&fixture.state, &fixture.validators)
            .unwrap()
            .contains_key(&node(5))
    );
    fixture.validators.push(node(5));
    assert_eq!(
        authorized_validator_weights(&fixture.state, &fixture.validators).unwrap()[&node(5)],
        750
    );
}

#[test]
fn audit_accepts_a_signed_rating_round_and_applies_the_same_committed_penalties() {
    let mut fixture = Fixture::new(&[1]);
    let initial = fixture.state.clone();
    let ratings = fixture.ratings();
    let control = replay_reputation_transition(
        &previous(&initial),
        &ratings,
        PENALTY_ROUND,
        &fixture.config,
    )
    .unwrap();
    assert_eq!(
        control
            .entries
            .iter()
            .find(|entry| entry.node_id == node(2))
            .unwrap()
            .reputation,
        707
    );
    let round = fixture.slash(2500, &ratings);
    fixture.assert_offender(1, 750);
    for seed in 2..=4 {
        assert_eq!(
            &entry(&fixture.state, &node(seed)),
            control
                .entries
                .iter()
                .find(|entry| entry.node_id == node(seed))
                .unwrap()
        );
    }
    assert_eq!(
        verify_reputation_transition(
            &previous(&initial),
            &ratings,
            &round.block,
            context(&initial, FINALIZED_WAVE),
            &fixture.config
        ),
        Err(PorError::ReputationBlockPenaltiesHashMismatch)
    );
    let committed = fixture.state.clone();
    assert!(
        fixture
            .state
            .apply_reputation_block_with_penalties(
                b"root",
                FINALIZED_WAVE,
                &ratings,
                round.block,
                &fixture.config,
                Some(&round.events)
            )
            .is_err()
    );
    assert_eq!(fixture.state, committed);
}

#[test]
fn audit_rejects_wrong_penalty_math_even_with_a_recomputed_commitment() {
    assert_audit_rejects(
        |block| {
            block
                .reputation_list
                .entries
                .iter_mut()
                .find(|entry| entry.node_id == node(1))
                .unwrap()
                .retained_reputation = 500;
        },
        PorError::ReputationValueMismatch,
    );
}

#[test]
fn audit_rejects_an_omitted_excluded_entry_even_with_a_recomputed_commitment() {
    assert_audit_rejects(
        |block| {
            block
                .reputation_list
                .entries
                .retain(|entry| entry.node_id != node(1));
        },
        PorError::MissingReputationBlockEntry,
    );
}

fn assert_audit_rejects(tamper: impl FnOnce(&mut ReputationBlock), expected: PorError) {
    let mut fixture = Fixture::new(&[1]);
    let initial = fixture.state.clone();
    let ratings = fixture.ratings();
    let mut round = fixture.slash(2500, &ratings);
    tamper(&mut round.block);
    round.block.header.reputation_root =
        reputation_list_commitment(&round.block.reputation_list).unwrap();
    assert_eq!(
        verify_reputation_transition_with_penalties(
            &previous(&initial),
            &ratings,
            &round.block,
            context(&initial, FINALIZED_WAVE),
            &fixture.config,
            Some(&round.events)
        ),
        Err(expected.clone())
    );
    let mut unchanged = initial.clone();
    assert_eq!(
        unchanged.apply_reputation_block_with_penalties(
            b"root",
            FINALIZED_WAVE,
            &ratings,
            round.block,
            &fixture.config,
            Some(&round.events)
        ),
        Err(expected)
    );
    assert_eq!(unchanged, initial);
}

#[test]
fn evidence_and_rating_arrival_order_do_not_change_the_committed_transition() {
    let mut forward = Fixture::new(&[1, 2, 3]);
    let mut reverse = Fixture::new(&[3, 2, 1]);
    let mut ratings = forward.ratings();
    let first = forward.slash(10000, &ratings);
    ratings.reverse();
    let other = reverse.slash(10000, &ratings);
    assert_eq!(first.block, other.block);
    assert_eq!(first.events, other.events);
    assert_eq!(forward.state, reverse.state);
    let mut permuted = first.events;
    permuted.equivocations.reverse();
    let original = Fixture::new(&[1, 2, 3]);
    let rebuilt = build_round(
        &original.state,
        &ratings,
        Some(&permuted),
        FINALIZED_WAVE,
        &original.config,
    );
    assert_eq!(rebuilt, other.block);
}
