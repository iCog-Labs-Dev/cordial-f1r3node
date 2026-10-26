use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
};

use cordial_f1r3node_adapter::{
    grpc_ingest::BlocklaceAdapter,
    live_ingress::{LiveIngress, PorWeightActivationError},
    ordered_output::OrderedFinalizedOutput,
    por::{
        CompletedPorRatingRound, DurablePorState, DurablePorStateError, PorFinalityTracker,
        PorRatingRoundCoordinator, PorRatingRoundCutoffPolicy, PorWeightActivationOutcome,
        PorWeightActivationStore, PorWeightActivationStoreError, RatingEnvelopeBroadcaster,
    },
    shard_conf::CasperShardConf,
};
use cordial_miners_core::{
    Block, BlockContent, BlockIdentity, Blocklace, NodeId, crypto::CryptoVerifier,
};
use cordial_por::{PorConfig, PorError, ReputationState};
use k256::ecdsa::SigningKey;
use tempfile::tempdir;

const WAVELENGTH: u64 = 3;
const SHARD_ID: &[u8] = b"root";

#[derive(Default)]
struct AcceptingAdapter;

impl BlocklaceAdapter<BlockIdentity> for AcceptingAdapter {
    fn on_block(&mut self, _block: Block) -> anyhow::Result<()> {
        Ok(())
    }
}

struct AcceptAll;

impl CryptoVerifier for AcceptAll {
    type Error = String;

    fn verify_block(
        &self,
        _content: &BlockContent,
        _signature: &[u8],
        _creator: &NodeId,
    ) -> Result<(), Self::Error> {
        Ok(())
    }
}

#[derive(Default)]
struct RecordingBroadcaster {
    envelopes: RefCell<Vec<Vec<u8>>>,
}

impl RatingEnvelopeBroadcaster for RecordingBroadcaster {
    fn broadcast_rating_envelope(&self, envelope: &[u8]) -> Result<(), String> {
        self.envelopes.borrow_mut().push(envelope.to_vec());
        Ok(())
    }
}

struct RoundFixture {
    blocks: Vec<Block>,
    blocklace: Blocklace,
    output: OrderedFinalizedOutput,
    state: ReputationState,
    config: PorConfig,
}

fn node(seed: u8) -> NodeId {
    let signing_key = SigningKey::from_slice(&[seed; 32]).unwrap();
    NodeId(
        signing_key
            .verifying_key()
            .to_encoded_point(true)
            .as_bytes()
            .to_vec(),
    )
}

fn block(tag: u8, creator_seed: u8, predecessor: Option<&BlockIdentity>) -> Block {
    let mut content_hash = [0; 32];
    content_hash[0] = tag;

    Block {
        identity: BlockIdentity {
            content_hash,
            creator: node(creator_seed),
            signature: vec![tag],
        },
        content: BlockContent {
            payload: vec![tag],
            predecessors: predecessor.into_iter().cloned().collect::<HashSet<_>>(),
        },
    }
}

fn block_with_predecessors(tag: u8, creator_seed: u8, predecessors: &[BlockIdentity]) -> Block {
    let mut content_hash = [0; 32];
    content_hash[0] = tag;
    Block {
        identity: BlockIdentity {
            content_hash,
            creator: node(creator_seed),
            signature: vec![tag],
        },
        content: BlockContent {
            payload: vec![tag],
            predecessors: predecessors.iter().cloned().collect(),
        },
    }
}

fn round_fixture() -> RoundFixture {
    let leader = block(1, 1, None);
    let second = block(2, 2, Some(&leader.identity));
    let third = block(3, 1, Some(&second.identity));
    let blocks = vec![leader.clone(), second.clone(), third.clone()];

    let mut blocklace = Blocklace::new();
    for block in &blocks {
        blocklace.insert(block.clone(), &AcceptAll).unwrap();
    }

    let output = OrderedFinalizedOutput::new(
        vec![
            third.identity.clone(),
            leader.identity.clone(),
            second.identity.clone(),
        ],
        Some(leader.identity),
        WAVELENGTH,
        2,
        3,
    )
    .with_timestamp(0);

    let mut state = ReputationState::new(0);
    for seed in [1, 2, 9] {
        state.set_reputation(node(seed), 100);
    }

    RoundFixture {
        blocks,
        blocklace,
        output,
        state,
        config: PorConfig::default(),
    }
}

fn completed_round(fixture: &RoundFixture) -> CompletedPorRatingRound {
    let opened = PorFinalityTracker::new()
        .observe_finalized_output(&fixture.blocklace, &fixture.output)
        .unwrap()
        .into_iter()
        .last()
        .unwrap();
    let mut coordinator = PorRatingRoundCoordinator::new(
        &fixture.blocklace,
        &fixture.output,
        opened,
        &fixture.state,
        &fixture.config,
    )
    .unwrap();
    coordinator.produce_local_batch(&node(9), &[9; 32]).unwrap();
    coordinator
        .broadcast_pending(&RecordingBroadcaster::default())
        .unwrap();
    coordinator
        .close_at_finalized_wave(
            &PorRatingRoundCutoffPolicy::default(),
            opened.finalized_wave + 1,
        )
        .unwrap();
    coordinator.into_completed().unwrap()
}

fn genesis_state() -> (NodeId, NodeId, NodeId, ReputationState) {
    let validator_a = NodeId(vec![1]);
    let validator_b = NodeId(vec![2]);
    let non_validator = NodeId(vec![3]);
    let mut state = ReputationState::new(0);
    state.set_reputation(validator_a.clone(), 20);
    state.set_reputation(validator_b.clone(), 80);
    state.set_reputation(non_validator.clone(), 1_000);
    (validator_a, validator_b, non_validator, state)
}

#[test]
fn activation_records_exact_projection_without_changing_membership() {
    let directory = tempdir().unwrap();
    let (validator_a, validator_b, non_validator, state) = genesis_state();
    let mut runtime = DurablePorState::open(directory.path(), state).unwrap();
    let mut ingress = LiveIngress::with_consensus_view(
        (),
        HashMap::from([(validator_a.clone(), 50), (validator_b.clone(), 50)]),
        CasperShardConf::default(),
        "root",
    );

    ingress
        .validate_por_weights(runtime.state().unwrap())
        .unwrap();
    assert!(runtime.has_unactivated_committed_round().unwrap());
    assert_eq!(
        runtime.activate_weights(&mut ingress).unwrap(),
        PorWeightActivationOutcome::Activated
    );

    let activated = runtime.activated_weight_record().unwrap().unwrap();
    assert_eq!(activated.reputation_round(), 0);
    assert_eq!(activated.checkpoint_hash(), None);
    assert_eq!(activated.source_finalized_wave(), None);
    assert_eq!(
        activated.weights(),
        &HashMap::from([(validator_a.clone(), 20), (validator_b.clone(), 80),])
    );
    assert!(runtime.weight_activation_record_path().is_file());
    assert!(!runtime.has_unactivated_committed_round().unwrap());
    assert_eq!(ingress.bonds().len(), 2);
    assert_eq!(ingress.bonds().get(&validator_a), Some(&20));
    assert_eq!(ingress.bonds().get(&validator_b), Some(&80));
    assert!(!ingress.bonds().contains_key(&non_validator));

    assert_eq!(
        runtime.activate_weights(&mut ingress).unwrap(),
        PorWeightActivationOutcome::AlreadyActive
    );
}

#[test]
fn restart_reapplies_a_durable_activation_to_fresh_ingress_memory() {
    let directory = tempdir().unwrap();
    let (validator_a, validator_b, _, state) = genesis_state();
    let initial_bonds = HashMap::from([(validator_a.clone(), 50), (validator_b.clone(), 50)]);
    let mut runtime = DurablePorState::open(directory.path(), state).unwrap();
    let mut ingress = LiveIngress::with_consensus_view(
        (),
        initial_bonds.clone(),
        CasperShardConf::default(),
        "root",
    );
    runtime.activate_weights(&mut ingress).unwrap();
    drop(runtime);

    let mut reopened = DurablePorState::open(directory.path(), ReputationState::new(99)).unwrap();
    let mut fresh_ingress =
        LiveIngress::with_consensus_view((), initial_bonds, CasperShardConf::default(), "root");

    assert!(!reopened.has_unactivated_committed_round().unwrap());
    assert_eq!(
        reopened
            .restore_activated_weights(&mut fresh_ingress)
            .unwrap(),
        Some(PorWeightActivationOutcome::Restored)
    );
    assert_eq!(fresh_ingress.bonds().get(&validator_a), Some(&20));
    assert_eq!(fresh_ingress.bonds().get(&validator_b), Some(&80));
}

#[test]
fn restart_rejects_membership_drift_before_restoring_weights() {
    let directory = tempdir().unwrap();
    let (validator_a, validator_b, _, state) = genesis_state();
    let mut runtime = DurablePorState::open(directory.path(), state).unwrap();
    let mut ingress = LiveIngress::with_consensus_view(
        (),
        HashMap::from([(validator_a.clone(), 50), (validator_b, 50)]),
        CasperShardConf::default(),
        "root",
    );
    runtime.activate_weights(&mut ingress).unwrap();
    drop(runtime);

    let mut reopened = DurablePorState::open(directory.path(), ReputationState::new(99)).unwrap();
    let mut changed_membership = LiveIngress::with_consensus_view(
        (),
        HashMap::from([(validator_a, 100)]),
        CasperShardConf::default(),
        "root",
    );

    assert!(matches!(
        reopened.restore_activated_weights(&mut changed_membership),
        Err(DurablePorStateError::Activation(
            PorWeightActivationError::ActivatedValidatorSetMismatch {
                configured: 1,
                recorded: 2,
            }
        ))
    ));
}

#[test]
fn corrupt_activation_record_is_a_startup_error() {
    let directory = tempdir().unwrap();
    let (validator_a, validator_b, _, state) = genesis_state();
    let mut runtime = DurablePorState::open(directory.path(), state).unwrap();
    let mut ingress = LiveIngress::with_consensus_view(
        (),
        HashMap::from([(validator_a, 50), (validator_b, 50)]),
        CasperShardConf::default(),
        "root",
    );
    runtime.activate_weights(&mut ingress).unwrap();

    let record_path = runtime.weight_activation_record_path().to_path_buf();
    let mut encoded = std::fs::read(&record_path).unwrap();
    let final_byte = encoded.last_mut().unwrap();
    *final_byte ^= 1;
    std::fs::write(record_path, encoded).unwrap();
    drop(runtime);

    assert!(matches!(
        DurablePorState::open(directory.path(), ReputationState::new(99)),
        Err(DurablePorStateError::ActivationPersistence(
            PorWeightActivationStoreError::ChecksumMismatch
        ))
    ));
}

#[test]
fn activation_rejection_is_retryable_and_does_not_record_success() {
    let directory = tempdir().unwrap();
    let validator_a = NodeId(vec![1]);
    let validator_b = NodeId(vec![2]);
    let mut state = ReputationState::new(0);
    state.set_reputation(validator_a.clone(), 100);
    let mut runtime = DurablePorState::open(directory.path(), state).unwrap();
    let mut ingress = LiveIngress::with_consensus_view(
        (),
        HashMap::from([(validator_a.clone(), 50), (validator_b.clone(), 50)]),
        CasperShardConf::default(),
        "root",
    );

    assert!(matches!(
        runtime.activate_weights(&mut ingress),
        Err(DurablePorStateError::Activation(
            PorWeightActivationError::Projection(
                PorError::MissingAuthorizedValidatorReputation(missing)
            )
        )) if missing == validator_b
    ));
    assert!(runtime.activated_weight_record().unwrap().is_none());
    assert!(!runtime.weight_activation_record_path().exists());
    assert!(runtime.has_unactivated_committed_round().unwrap());

    ingress.set_bonds(HashMap::from([(validator_a.clone(), 50)]));
    assert_eq!(
        runtime.activate_weights(&mut ingress).unwrap(),
        PorWeightActivationOutcome::Activated
    );
    assert_eq!(ingress.bonds(), &HashMap::from([(validator_a, 100)]));
}

#[test]
fn weight_activation_preserves_the_published_final_leader() {
    let mut seeds: Vec<u8> = (1..=5).collect();
    seeds.sort_by_key(|seed| node(*seed));
    let initial_weights: HashMap<_, _> = seeds
        .iter()
        .enumerate()
        .map(|(index, seed)| (node(*seed), if index == 4 { 1 } else { 100 }))
        .collect();
    let mut ingress = LiveIngress::with_consensus_view(
        AcceptingAdapter,
        initial_weights.clone(),
        CasperShardConf::default(),
        "root",
    );
    let leader = block_with_predecessors(11, seeds[0], &[]);
    let round_one: Vec<_> = seeds[1..4]
        .iter()
        .enumerate()
        .map(|(index, seed)| {
            block_with_predecessors(
                12 + index as u8,
                *seed,
                std::slice::from_ref(&leader.identity),
            )
        })
        .collect();
    let frontier: Vec<_> = round_one
        .iter()
        .map(|block| block.identity.clone())
        .collect();
    let round_two: Vec<_> = seeds[1..4]
        .iter()
        .enumerate()
        .map(|(index, seed)| block_with_predecessors(15 + index as u8, *seed, &frontier))
        .collect();
    for block in std::iter::once(&leader)
        .chain(round_one.iter())
        .chain(round_two.iter())
    {
        ingress.ingest_trusted_block(block.clone()).unwrap();
    }
    let published = ingress.latest_finalized_ordered_output(WAVELENGTH).unwrap();
    assert_eq!(published.anchor, Some(leader.identity.consensus_identity()));

    let mut state = ReputationState::new(0);
    for (index, seed) in seeds.iter().enumerate() {
        state.set_reputation(node(*seed), if index == 4 { 100 } else { 1 });
    }
    assert!(matches!(
        ingress.validate_por_weights(&state),
        Err(PorWeightActivationError::WouldRewriteFinalizedLeader)
    ));
    assert!(matches!(
        ingress.apply_por_weights(&state),
        Err(PorWeightActivationError::WouldRewriteFinalizedLeader)
    ));
    assert_eq!(ingress.bonds(), &initial_weights);
}

#[test]
fn marker_write_failure_fail_closes_and_restart_recovers() {
    let directory = tempdir().unwrap();
    let (validator_a, validator_b, _, state) = genesis_state();
    let mut runtime = DurablePorState::open(directory.path(), state).unwrap();
    let temporary_path = runtime
        .weight_activation_record_path()
        .parent()
        .unwrap()
        .join(".weight-activation.bin.tmp");
    std::fs::create_dir(&temporary_path).unwrap();
    let initial_bonds = HashMap::from([(validator_a.clone(), 50), (validator_b.clone(), 50)]);
    let mut ingress = LiveIngress::with_consensus_view(
        (),
        initial_bonds.clone(),
        CasperShardConf::default(),
        "root",
    );

    assert!(matches!(
        runtime.activate_weights(&mut ingress),
        Err(DurablePorStateError::ActivationPersistence(
            PorWeightActivationStoreError::Io(_)
        ))
    ));
    assert_eq!(ingress.bonds().get(&validator_a), Some(&20));
    assert_eq!(ingress.bonds().get(&validator_b), Some(&80));
    assert!(matches!(
        runtime.state(),
        Err(DurablePorStateError::RecoveryRequired)
    ));

    std::fs::remove_dir(temporary_path).unwrap();
    drop(runtime);

    let mut reopened = DurablePorState::open(directory.path(), ReputationState::new(99)).unwrap();
    let mut fresh_ingress =
        LiveIngress::with_consensus_view((), initial_bonds, CasperShardConf::default(), "root");
    assert!(reopened.has_unactivated_committed_round().unwrap());
    assert_eq!(
        reopened.activate_weights(&mut fresh_ingress).unwrap(),
        PorWeightActivationOutcome::Activated
    );
    assert!(reopened.weight_activation_record_path().is_file());
    assert_eq!(fresh_ingress.bonds().get(&validator_a), Some(&20));
    assert_eq!(fresh_ingress.bonds().get(&validator_b), Some(&80));
}

#[test]
fn committed_round_waits_for_source_finality_then_activates_on_retry() {
    let directory = tempdir().unwrap();
    let fixture = round_fixture();
    let completed = completed_round(&fixture);
    let bonds = HashMap::from([(node(1), 100)]);
    let mut runtime = DurablePorState::open(directory.path(), fixture.state.clone()).unwrap();
    let mut ingress = LiveIngress::with_consensus_view(
        AcceptingAdapter,
        bonds,
        CasperShardConf::default(),
        "root",
    );

    runtime.activate_weights(&mut ingress).unwrap();
    runtime
        .apply_completed_round(&completed, &fixture.config, SHARD_ID)
        .unwrap();
    assert!(runtime.has_unactivated_committed_round().unwrap());
    assert_eq!(
        runtime
            .activated_weight_record()
            .unwrap()
            .unwrap()
            .reputation_round(),
        0
    );

    assert!(matches!(
        runtime.activate_weights(&mut ingress),
        Err(DurablePorStateError::Activation(
            PorWeightActivationError::MissingFinalizedOutput { source_wave: 0 }
        ))
    ));
    assert_eq!(runtime.state().unwrap().round(), 1);
    assert!(runtime.has_unactivated_committed_round().unwrap());

    for block in fixture.blocks {
        ingress.ingest_trusted_block(block).unwrap();
    }
    let output = ingress.latest_finalized_ordered_output(WAVELENGTH).unwrap();
    assert!(output.anchor.is_some());

    assert_eq!(
        runtime.activate_weights(&mut ingress).unwrap(),
        PorWeightActivationOutcome::Activated
    );
    assert_eq!(
        runtime
            .activated_weight_record()
            .unwrap()
            .unwrap()
            .reputation_round(),
        1
    );
    assert!(!runtime.has_unactivated_committed_round().unwrap());
    assert_eq!(ingress.bonds().len(), 1);

    let activated_round_one = runtime.activated_weight_record().unwrap().unwrap().clone();
    let stale_directory = tempdir().unwrap();
    PorWeightActivationStore::open(stale_directory.path())
        .unwrap()
        .persist(&activated_round_one)
        .unwrap();
    assert!(matches!(
        DurablePorState::open(stale_directory.path(), ReputationState::new(0)),
        Err(DurablePorStateError::ActivationAheadOfState {
            activated_round: 1,
            committed_round: 0,
        })
    ));
}
