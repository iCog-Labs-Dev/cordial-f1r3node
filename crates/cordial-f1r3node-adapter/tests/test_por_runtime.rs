use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    fs,
};

use cordial_f1r3node_adapter::{
    grpc_ingest::BlocklaceAdapter,
    live_ingress::{LiveIngress, PorWeightActivationError},
    ordered_output::OrderedFinalizedOutput,
    por::{
        CompletedPorRatingRound, DurablePorState, DurablePorStateError, PorFinalityTracker,
        PorRatingRoundCoordinator, PorRatingRoundCutoffPolicy, PorRuntime, PorRuntimeError,
        PorWeightActivationOutcome, PorWeightEpochPolicy, PorWeightEpochStore,
        RatingEnvelopeBroadcaster, close_checkpoint_rating_exchange_if_complete,
        finalized_prefix_checkpoint,
        rating_key::load_bonded_rating_key,
        stage_local_checkpoint_rating_batch,
        transport::{file_exchange::FileRatingExchange, outbox::RatingOutbox},
    },
    shard_conf::CasperShardConf,
    shared_ordered_output::ReadOrderedOutput,
    snapshot::CORDIAL_WAVELENGTH,
};
use cordial_miners_core::{
    Block, BlockContent, BlockIdentity, Blocklace, NodeId,
    consensus::{depth, wave_of_round},
    crypto::CryptoVerifier,
};
use cordial_por::{PorConfig, ReputationState, authorized_validator_weights};
use k256::ecdsa::SigningKey;
use tempfile::tempdir;

const WAVELENGTH: u64 = CORDIAL_WAVELENGTH;
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

fn raw_block(tag: u8, creator: &NodeId, predecessors: &[&Block]) -> Block {
    let mut content_hash = [0; 32];
    content_hash[0] = tag;

    Block {
        identity: BlockIdentity {
            content_hash,
            creator: creator.clone(),
            signature: vec![tag],
        },
        content: BlockContent {
            payload: vec![tag],
            predecessors: predecessors
                .iter()
                .map(|block| block.identity.clone())
                .collect(),
        },
    }
}

fn weighted_finality_wave() -> (Vec<Block>, Vec<NodeId>, BlockIdentity) {
    let validators: Vec<_> = (1..=5).map(|id| NodeId(vec![id])).collect();
    let leader = raw_block(11, &validators[0], &[]);
    let round_one: Vec<_> = validators[1..4]
        .iter()
        .enumerate()
        .map(|(index, validator)| {
            raw_block(u8::try_from(12 + index).unwrap(), validator, &[&leader])
        })
        .collect();
    let round_one_refs: Vec<_> = round_one.iter().collect();
    let round_two: Vec<_> = validators[1..4]
        .iter()
        .enumerate()
        .map(|(index, validator)| {
            raw_block(
                u8::try_from(15 + index).unwrap(),
                validator,
                &round_one_refs,
            )
        })
        .collect();

    let leader_identity = leader.identity.clone();
    let mut blocks = vec![leader];
    blocks.extend(round_one);
    blocks.extend(round_two);
    (blocks, validators, leader_identity)
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
        1,
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

fn genesis_state() -> (NodeId, NodeId, ReputationState) {
    let validator_a = NodeId(vec![1]);
    let validator_b = NodeId(vec![2]);
    let mut state = ReputationState::new(0);
    state.set_reputation(validator_a.clone(), 20);
    state.set_reputation(validator_b.clone(), 80);
    (validator_a, validator_b, state)
}

#[test]
fn startup_restores_weights_before_ingress_is_exposed() {
    let directory = tempdir().unwrap();
    let (validator_a, validator_b, state) = genesis_state();
    let configured_bonds = HashMap::from([(validator_a.clone(), 50), (validator_b.clone(), 50)]);
    let ingress = LiveIngress::with_consensus_view(
        (),
        configured_bonds.clone(),
        CasperShardConf::default(),
        "root",
    );

    let runtime = PorRuntime::open(
        directory.path(),
        state,
        ingress,
        PorConfig::default(),
        SHARD_ID,
        WAVELENGTH,
    )
    .unwrap();

    assert_eq!(
        runtime.startup_activation(),
        PorWeightActivationOutcome::Activated
    );
    assert_eq!(runtime.ingress().bonds().get(&validator_a), Some(&20));
    assert_eq!(runtime.ingress().bonds().get(&validator_b), Some(&80));
    drop(runtime);

    let fresh_ingress =
        LiveIngress::with_consensus_view((), configured_bonds, CasperShardConf::default(), "root");
    let reopened = PorRuntime::open(
        directory.path(),
        ReputationState::new(99),
        fresh_ingress,
        PorConfig::default(),
        SHARD_ID,
        WAVELENGTH,
    )
    .unwrap();

    assert_eq!(
        reopened.startup_activation(),
        PorWeightActivationOutcome::Restored
    );
    assert_eq!(reopened.ingress().bonds().get(&validator_a), Some(&20));
    assert_eq!(reopened.ingress().bonds().get(&validator_b), Some(&80));
}

#[test]
fn startup_computes_finality_after_restoring_durable_weights() {
    let directory = tempdir().unwrap();
    let (blocks, validators, leader) = weighted_finality_wave();
    let bootstrap_weights: HashMap<_, _> = validators
        .iter()
        .cloned()
        .enumerate()
        .map(|(index, validator)| (validator, if index == 4 { 100 } else { 1 }))
        .collect();
    let active_weights: HashMap<_, _> = validators
        .iter()
        .cloned()
        .enumerate()
        .map(|(index, validator)| (validator, if index == 4 { 1 } else { 100 }))
        .collect();
    let mut state = ReputationState::new(0);
    for (validator, weight) in &active_weights {
        state.set_reputation(validator.clone(), *weight);
    }

    let ingress = LiveIngress::with_consensus_view(
        AcceptingAdapter,
        bootstrap_weights.clone(),
        CasperShardConf::default(),
        "root",
    );
    let initial = PorRuntime::open(
        directory.path(),
        state,
        ingress,
        PorConfig::default(),
        SHARD_ID,
        WAVELENGTH,
    )
    .unwrap();
    assert_eq!(
        initial.startup_activation(),
        PorWeightActivationOutcome::Activated
    );
    drop(initial);

    let mut bootstrap_ingress = LiveIngress::with_consensus_view(
        AcceptingAdapter,
        bootstrap_weights.clone(),
        CasperShardConf::default(),
        "root",
    );
    for block in blocks.iter().cloned() {
        bootstrap_ingress.ingest_trusted_block(block).unwrap();
    }
    assert!(
        bootstrap_ingress
            .latest_finalized_ordered_output(WAVELENGTH)
            .unwrap()
            .anchor
            .is_none()
    );

    let mut restarted_ingress = LiveIngress::with_consensus_view(
        AcceptingAdapter,
        bootstrap_weights,
        CasperShardConf::default(),
        "root",
    );
    for block in blocks {
        restarted_ingress.ingest_trusted_block(block).unwrap();
    }
    let restarted = PorRuntime::open(
        directory.path(),
        ReputationState::new(99),
        restarted_ingress,
        PorConfig::default(),
        SHARD_ID,
        WAVELENGTH,
    )
    .unwrap();

    assert_eq!(
        restarted.startup_activation(),
        PorWeightActivationOutcome::Restored
    );
    assert_eq!(restarted.ingress().bonds(), &active_weights);
    assert_eq!(
        restarted
            .ingress()
            .ordered_output_reader()
            .latest()
            .and_then(|output| output.anchor.clone()),
        Some(leader.consensus_identity())
    );
}

#[test]
fn recovered_non_genesis_state_requires_a_durable_active_projection() {
    let directory = tempdir().unwrap();
    let fixture = round_fixture();
    let completed = completed_round(&fixture);
    let mut durable =
        cordial_f1r3node_adapter::por::DurablePorState::open(directory.path(), fixture.state)
            .unwrap();
    durable
        .apply_completed_round(&completed, &fixture.config, SHARD_ID)
        .unwrap();
    drop(durable);

    let ingress = LiveIngress::with_consensus_view(
        AcceptingAdapter,
        HashMap::from([(node(1), 100)]),
        CasperShardConf::default(),
        "root",
    );
    assert!(matches!(
        PorRuntime::open(
            directory.path(),
            ReputationState::new(99),
            ingress,
            fixture.config,
            SHARD_ID,
            WAVELENGTH,
        ),
        Err(PorRuntimeError::Durable(
            DurablePorStateError::MissingActivatedWeights { committed_round: 1 }
        ))
    ));
}

#[test]
fn startup_finishes_a_committed_but_unactivated_round() {
    let directory = tempdir().unwrap();
    let fixture = round_fixture();
    let completed = completed_round(&fixture);
    let validator = node(1);
    let mut durable =
        cordial_f1r3node_adapter::por::DurablePorState::open(directory.path(), fixture.state)
            .unwrap();
    let mut live_ingress = LiveIngress::with_consensus_view(
        AcceptingAdapter,
        HashMap::from([(validator.clone(), 100)]),
        CasperShardConf::default(),
        "root",
    );
    durable.activate_weights(&mut live_ingress).unwrap();
    durable
        .apply_completed_round(&completed, &fixture.config, SHARD_ID)
        .unwrap();
    assert_eq!(
        durable
            .activated_weight_record()
            .unwrap()
            .unwrap()
            .reputation_round(),
        0
    );
    drop(durable);

    let mut restarted_ingress = LiveIngress::with_consensus_view(
        AcceptingAdapter,
        HashMap::from([(validator, 7)]),
        CasperShardConf::default(),
        "root",
    );
    for block in fixture.blocks {
        restarted_ingress.ingest_trusted_block(block).unwrap();
    }
    let restarted = PorRuntime::open(
        directory.path(),
        ReputationState::new(99),
        restarted_ingress,
        fixture.config,
        SHARD_ID,
        WAVELENGTH,
    )
    .unwrap();

    assert_eq!(
        restarted.startup_activation(),
        PorWeightActivationOutcome::Activated
    );
    assert_eq!(restarted.state().state().unwrap().round(), 1);
    assert_eq!(
        restarted
            .state()
            .activated_weight_record()
            .unwrap()
            .unwrap()
            .reputation_round(),
        1
    );
}

#[test]
fn startup_defers_pending_activation_until_source_blocks_are_replayed() {
    let directory = tempdir().unwrap();
    let fixture = round_fixture();
    let completed = completed_round(&fixture);
    let validator = node(1);
    let mut durable =
        cordial_f1r3node_adapter::por::DurablePorState::open(directory.path(), fixture.state)
            .unwrap();
    let mut first_ingress = LiveIngress::with_consensus_view(
        AcceptingAdapter,
        HashMap::from([(validator.clone(), 100)]),
        CasperShardConf::default(),
        "root",
    );
    durable.activate_weights(&mut first_ingress).unwrap();
    durable
        .apply_completed_round(&completed, &fixture.config, SHARD_ID)
        .unwrap();
    drop(durable);

    let empty_ingress = LiveIngress::with_consensus_view(
        AcceptingAdapter,
        HashMap::from([(validator, 7)]),
        CasperShardConf::default(),
        "root",
    );
    let mut restarted = PorRuntime::open(
        directory.path(),
        ReputationState::new(99),
        empty_ingress,
        fixture.config,
        SHARD_ID,
        WAVELENGTH,
    )
    .unwrap();
    assert_eq!(
        restarted.startup_activation(),
        PorWeightActivationOutcome::Restored
    );
    assert_eq!(restarted.state().state().unwrap().round(), 1);
    assert!(restarted.state().has_unactivated_committed_round().unwrap());
    assert_eq!(
        restarted
            .state()
            .activated_weight_record()
            .unwrap()
            .unwrap()
            .reputation_round(),
        0
    );

    for block in fixture.blocks {
        restarted.ingress_mut().ingest_trusted_block(block).unwrap();
    }
    assert!(
        restarted
            .publish_finalized_output()
            .unwrap()
            .anchor
            .is_some()
    );
    assert_eq!(
        restarted.retry_weight_activation().unwrap(),
        PorWeightActivationOutcome::Activated
    );
    assert!(!restarted.state().has_unactivated_committed_round().unwrap());
    assert_eq!(
        restarted
            .state()
            .activated_weight_record()
            .unwrap()
            .unwrap()
            .reputation_round(),
        1
    );
}

#[test]
fn invalid_runtime_parameters_fail_before_state_initialization() {
    let directory = tempdir().unwrap();
    let (_, _, state) = genesis_state();
    let ingress = LiveIngress::with_consensus_view(
        (),
        HashMap::from([(NodeId(vec![1]), 100)]),
        CasperShardConf::default(),
        "root",
    );

    assert!(matches!(
        PorRuntime::open(
            directory.path(),
            state.clone(),
            ingress,
            PorConfig::default(),
            SHARD_ID,
            0,
        ),
        Err(PorRuntimeError::UnsupportedWavelength {
            expected: WAVELENGTH,
            actual: 0,
        })
    ));
    assert!(!directory.path().join("por").exists());

    let ingress = LiveIngress::with_consensus_view(
        (),
        HashMap::from([(NodeId(vec![1]), 100)]),
        CasperShardConf::default(),
        "root",
    );
    assert!(matches!(
        PorRuntime::open(
            directory.path(),
            state.clone(),
            ingress,
            PorConfig::default(),
            SHARD_ID,
            WAVELENGTH + 1,
        ),
        Err(PorRuntimeError::UnsupportedWavelength {
            expected: WAVELENGTH,
            actual: 4,
        })
    ));
    assert!(!directory.path().join("por").exists());

    let ingress = LiveIngress::with_consensus_view(
        (),
        HashMap::from([(NodeId(vec![1]), 100)]),
        CasperShardConf::default(),
        "root",
    );
    assert!(matches!(
        PorRuntime::open(
            directory.path(),
            state,
            ingress,
            PorConfig::default(),
            Vec::new(),
            WAVELENGTH,
        ),
        Err(PorRuntimeError::InvalidShard(_))
    ));
    assert!(!directory.path().join("por").exists());
}

#[allow(deprecated)]
#[test]
fn completed_round_is_automatically_activated_after_commit() {
    let directory = tempdir().unwrap();
    let fixture = round_fixture();
    let completed = completed_round(&fixture);
    let ingress = LiveIngress::with_consensus_view(
        AcceptingAdapter,
        HashMap::from([(node(1), 100)]),
        CasperShardConf::default(),
        "root",
    );
    let mut runtime = PorRuntime::open(
        directory.path(),
        fixture.state,
        ingress,
        fixture.config,
        SHARD_ID,
        WAVELENGTH,
    )
    .unwrap();

    for block in fixture.blocks {
        runtime.ingress_mut().ingest_trusted_block(block).unwrap();
    }
    let output = runtime.publish_finalized_output().unwrap();
    assert!(output.anchor.is_some());

    #[allow(deprecated)]
    let committed = runtime.commit_completed_round(&completed).unwrap();

    assert_eq!(
        committed.activation.unwrap(),
        PorWeightActivationOutcome::Activated
    );
    assert_eq!(runtime.state().state().unwrap().round(), 1);
    let expected =
        authorized_validator_weights(runtime.state().state().unwrap(), &[node(1)]).unwrap();
    assert_eq!(runtime.ingress().bonds(), &expected);
    assert_eq!(runtime.ingress().bonds().len(), 1);
}

#[allow(deprecated)]
#[test]
fn completed_checkpoint_commits_activates_and_recovers_from_live_finality() {
    let directory = tempdir().unwrap();
    let validator = node(1);
    let mut initial = ReputationState::new(0);
    initial.set_reputation(validator.clone(), 100);
    let ingress = LiveIngress::with_consensus_view(
        AcceptingAdapter,
        HashMap::from([(validator.clone(), 100)]),
        CasperShardConf::default(),
        "root",
    );
    let mut runtime = PorRuntime::open(
        directory.path(),
        initial.clone(),
        ingress,
        PorConfig::default(),
        SHARD_ID,
        WAVELENGTH,
    )
    .unwrap();
    let mut blocks = Vec::new();
    for tag in 1..=32 {
        let predecessor = blocks.last().map(|block: &Block| &block.identity);
        let next = block(tag, 1, predecessor);
        runtime
            .ingress_mut()
            .ingest_trusted_block(next.clone())
            .unwrap();
        blocks.push(next);
    }
    let output = runtime.publish_finalized_output().unwrap();
    let checkpoint = finalized_prefix_checkpoint(&output, 1).unwrap().unwrap();
    let key_path = directory.path().join("validator.key");
    fs::write(&key_path, hex::encode([1; 32])).unwrap();
    let key = load_bonded_rating_key(&key_path, runtime.ingress().bonds()).unwrap();
    let outbox = RatingOutbox::open(directory.path()).unwrap();
    let exchange = FileRatingExchange::open(&directory.path().join("exchange"), SHARD_ID).unwrap();
    stage_local_checkpoint_rating_batch(
        runtime.ingress().blocklace(),
        &output,
        &checkpoint,
        runtime.state().state().unwrap(),
        runtime.config(),
        &key,
        &outbox,
    )
    .unwrap();
    let completed = close_checkpoint_rating_exchange_if_complete(
        runtime.ingress().blocklace(),
        &output,
        &checkpoint,
        runtime.state().state().unwrap(),
        runtime.config(),
        &key,
        &HashSet::from([validator.clone()]),
        &outbox,
        &exchange,
    )
    .unwrap()
    .unwrap();

    let pending_root = directory.path().join("pending");
    let mut pending = DurablePorState::open(&pending_root, initial.clone()).unwrap();
    let mut genesis_ingress = LiveIngress::with_consensus_view(
        AcceptingAdapter,
        HashMap::from([(validator.clone(), 100)]),
        CasperShardConf::default(),
        "root",
    );
    pending.activate_weights(&mut genesis_ingress).unwrap();
    pending
        .apply_completed_checkpoint_round(&completed, &PorConfig::default(), SHARD_ID)
        .unwrap();
    for block in &blocks[..10] {
        genesis_ingress.ingest_trusted_block(block.clone()).unwrap();
    }
    let partial_output = genesis_ingress
        .latest_finalized_ordered_output(WAVELENGTH)
        .unwrap();
    assert!(partial_output.anchor.is_some());
    assert!(partial_output.blocks.len() < 16);
    assert!(matches!(
        pending.activate_weights(&mut genesis_ingress),
        Err(DurablePorStateError::Activation(
            PorWeightActivationError::FinalizedCheckpointUnavailable { round: 1 }
        ))
    ));
    assert_eq!(
        pending
            .activated_weight_record()
            .unwrap()
            .unwrap()
            .reputation_round(),
        0
    );
    drop(pending);
    let pending_ingress = LiveIngress::with_consensus_view(
        AcceptingAdapter,
        HashMap::from([(validator.clone(), 7)]),
        CasperShardConf::default(),
        "root",
    );
    let mut pending_runtime = PorRuntime::open(
        &pending_root,
        initial.clone(),
        pending_ingress,
        PorConfig::default(),
        SHARD_ID,
        WAVELENGTH,
    )
    .unwrap();
    assert_eq!(
        pending_runtime.startup_activation(),
        PorWeightActivationOutcome::Restored
    );
    assert!(
        pending_runtime
            .state()
            .has_unactivated_committed_round()
            .unwrap()
    );
    assert!(matches!(
        pending_runtime.retry_weight_activation(),
        Err(PorRuntimeError::CommittedCheckpointUnavailable { round: 1 })
    ));
    for block in &blocks[..10] {
        pending_runtime
            .ingress_mut()
            .ingest_trusted_block(block.clone())
            .unwrap();
    }
    assert!(matches!(
        pending_runtime.publish_finalized_output(),
        Err(PorRuntimeError::CommittedCheckpointUnavailable { round: 1 })
    ));
    for block in &blocks[10..] {
        pending_runtime
            .ingress_mut()
            .ingest_trusted_block(block.clone())
            .unwrap();
    }
    assert_eq!(
        pending_runtime.retry_weight_activation().unwrap(),
        PorWeightActivationOutcome::Activated
    );
    assert!(
        !pending_runtime
            .state()
            .has_unactivated_committed_round()
            .unwrap()
    );
    drop(pending_runtime);

    let mut wrong_ingress = LiveIngress::with_consensus_view(
        AcceptingAdapter,
        HashMap::from([(validator.clone(), 100)]),
        CasperShardConf::default(),
        "root",
    );
    let mut wrong_predecessor = None;
    for tag in 101..=132 {
        let next = block(tag, 1, wrong_predecessor.as_ref());
        wrong_ingress.ingest_trusted_block(next.clone()).unwrap();
        wrong_predecessor = Some(next.identity);
    }
    wrong_ingress
        .latest_finalized_ordered_output(WAVELENGTH)
        .unwrap();
    let mut pending_direct = DurablePorState::open(&pending_root, initial.clone()).unwrap();
    assert!(matches!(
        pending_direct.activate_weights(&mut wrong_ingress),
        Err(DurablePorStateError::Activation(
            PorWeightActivationError::FinalizedCheckpointMismatch { round: 1 }
        ))
    ));
    drop(pending_direct);
    assert!(matches!(
        PorRuntime::open(
            &pending_root,
            initial.clone(),
            wrong_ingress,
            PorConfig::default(),
            SHARD_ID,
            WAVELENGTH,
        ),
        Err(PorRuntimeError::CheckpointSource(_))
    ));

    let committed = runtime
        .commit_completed_checkpoint_round(&completed)
        .unwrap();
    assert_eq!(
        committed.activation.unwrap(),
        PorWeightActivationOutcome::Activated
    );
    assert_eq!(
        runtime.state().committed_checkpoint_source().unwrap(),
        Some(completed.source())
    );
    assert_eq!(runtime.state().state().unwrap().round(), 1);
    drop(runtime);

    let restarted_ingress = LiveIngress::with_consensus_view(
        AcceptingAdapter,
        HashMap::from([(validator.clone(), 7)]),
        CasperShardConf::default(),
        "root",
    );
    let mut restarted = PorRuntime::open(
        directory.path(),
        initial,
        restarted_ingress,
        PorConfig::default(),
        SHARD_ID,
        WAVELENGTH,
    )
    .unwrap();
    assert!(matches!(
        restarted.publish_finalized_output(),
        Err(PorRuntimeError::CommittedCheckpointUnavailable { round: 1 })
    ));
    for block in blocks {
        restarted.ingress_mut().ingest_trusted_block(block).unwrap();
    }
    restarted.publish_finalized_output().unwrap();
    assert_eq!(
        restarted.state().committed_checkpoint_source().unwrap(),
        Some(completed.source())
    );
    assert_eq!(restarted.state().state().unwrap().round(), 1);
    assert_eq!(
        restarted.ingress().bonds(),
        &HashMap::from([(validator, 100)])
    );
}

#[test]
fn committed_future_epoch_preserves_prefix_and_recovers_after_replay() {
    let directory = tempdir().unwrap();
    let validators: Vec<_> = (1..=4).map(node).collect();
    let mut initial = ReputationState::new(0);
    for validator in &validators {
        initial.set_reputation(validator.clone(), 100);
    }
    let ingress = LiveIngress::with_consensus_view(
        AcceptingAdapter,
        validators
            .iter()
            .cloned()
            .map(|validator| (validator, 100))
            .collect(),
        CasperShardConf::default(),
        "root",
    );
    let mut runtime = PorRuntime::open(
        directory.path(),
        initial.clone(),
        ingress,
        PorConfig::default(),
        SHARD_ID,
        WAVELENGTH,
    )
    .unwrap();
    let mut blocks = Vec::new();
    let mut previous_layer: Vec<Block> = Vec::new();
    let mut tag = 1u8;
    for _round in 0..30 {
        let predecessors: Vec<_> = previous_layer.iter().collect();
        let mut layer = Vec::new();
        for validator in &validators {
            let next = raw_block(tag, validator, &predecessors);
            runtime
                .ingress_mut()
                .ingest_trusted_block(next.clone())
                .unwrap();
            blocks.push(next.clone());
            layer.push(next);
            tag += 1;
        }
        previous_layer = layer;
    }
    let before = runtime.publish_finalized_output().unwrap();
    let checkpoint = finalized_prefix_checkpoint(&before, 1).unwrap().unwrap();
    let keys: Vec<_> = (1..=4)
        .map(|seed| {
            let path = directory.path().join(format!("validator-{seed}.key"));
            fs::write(&path, hex::encode([seed; 32])).unwrap();
            load_bonded_rating_key(&path, runtime.ingress().bonds()).unwrap()
        })
        .collect();
    let outbox = RatingOutbox::open(directory.path()).unwrap();
    let exchange = FileRatingExchange::open(&directory.path().join("exchange"), SHARD_ID).unwrap();
    for key in &keys {
        stage_local_checkpoint_rating_batch(
            runtime.ingress().blocklace(),
            &before,
            &checkpoint,
            runtime.state().state().unwrap(),
            runtime.config(),
            key,
            &outbox,
        )
        .unwrap();
    }
    let authorized: HashSet<_> = validators.iter().cloned().collect();
    for key in &keys[1..] {
        close_checkpoint_rating_exchange_if_complete(
            runtime.ingress().blocklace(),
            &before,
            &checkpoint,
            runtime.state().state().unwrap(),
            runtime.config(),
            key,
            &authorized,
            &outbox,
            &exchange,
        )
        .unwrap();
    }
    let completed = close_checkpoint_rating_exchange_if_complete(
        runtime.ingress().blocklace(),
        &before,
        &checkpoint,
        runtime.state().state().unwrap(),
        runtime.config(),
        &keys[0],
        &authorized,
        &outbox,
        &exchange,
    )
    .unwrap()
    .unwrap();
    let source_wave = wave_of_round(
        depth(runtime.ingress().blocklace(), &checkpoint.terminal_block).unwrap(),
        WAVELENGTH,
    )
    .unwrap();
    let boundary_wave = wave_of_round(
        depth(
            runtime.ingress().blocklace(),
            before.anchor.as_ref().unwrap(),
        )
        .unwrap(),
        WAVELENGTH,
    )
    .unwrap();
    let policy = PorWeightEpochPolicy::new(boundary_wave + 1 - source_wave).unwrap();
    runtime
        .commit_completed_checkpoint_round_with_epoch(&completed, policy)
        .unwrap();
    let active = runtime.publish_finalized_output().unwrap();
    assert!(active.blocks.starts_with(&before.blocks));
    assert_eq!(runtime.active_future_epoch().unwrap().por_round(), 1);
    assert!(!runtime.has_unactivated_committed_round().unwrap());
    let active_weights = runtime.ingress().bonds().clone();
    assert!(
        PorWeightEpochStore::open(directory.path())
            .unwrap()
            .restore()
            .unwrap()
            .is_some()
    );
    drop(runtime);

    let ingress = LiveIngress::with_consensus_view(
        AcceptingAdapter,
        validators
            .into_iter()
            .map(|validator| (validator, 7))
            .collect(),
        CasperShardConf::default(),
        "root",
    );
    let mut restarted = PorRuntime::open(
        directory.path(),
        initial,
        ingress,
        PorConfig::default(),
        SHARD_ID,
        WAVELENGTH,
    )
    .unwrap();
    assert!(restarted.publish_finalized_output().is_err());
    for block in blocks {
        restarted.ingress_mut().ingest_trusted_block(block).unwrap();
    }
    let recovered = restarted.publish_finalized_output().unwrap();
    assert_eq!(recovered.blocks, active.blocks);
    assert_eq!(recovered.anchor, active.anchor);
    assert_eq!(restarted.ingress().bonds(), &active_weights);
    assert!(!restarted.has_unactivated_committed_round().unwrap());
}

#[test]
fn committed_round_with_pending_activation_can_be_retried() {
    let directory = tempdir().unwrap();
    let fixture = round_fixture();
    let completed = completed_round(&fixture);
    let ingress = LiveIngress::with_consensus_view(
        AcceptingAdapter,
        HashMap::from([(node(1), 100)]),
        CasperShardConf::default(),
        "root",
    );
    let mut runtime = PorRuntime::open(
        directory.path(),
        fixture.state,
        ingress,
        fixture.config,
        SHARD_ID,
        WAVELENGTH,
    )
    .unwrap();

    #[allow(deprecated)]
    let committed = runtime.commit_completed_round(&completed).unwrap();

    assert!(committed.activation_pending());
    assert!(matches!(
        committed.activation,
        Err(DurablePorStateError::Activation(
            PorWeightActivationError::MissingFinalizedOutput { source_wave: 0 }
        ))
    ));
    assert_eq!(runtime.state().state().unwrap().round(), 1);

    for block in fixture.blocks {
        runtime.ingress_mut().ingest_trusted_block(block).unwrap();
    }
    assert!(runtime.publish_finalized_output().unwrap().anchor.is_some());
    assert_eq!(
        runtime.retry_weight_activation().unwrap(),
        PorWeightActivationOutcome::Activated
    );
    assert!(!runtime.state().has_unactivated_committed_round().unwrap());
}

#[test]
fn invalid_penalty_config_fails_before_opening_durable_state() {
    let directory = tempdir().unwrap();
    let (_, _, state) = genesis_state();
    let ingress = LiveIngress::with_consensus_view(
        (),
        HashMap::from([(NodeId(vec![1]), 100)]),
        CasperShardConf::default(),
        "root",
    );
    let mut config = PorConfig::default();
    config.base_slash_penalty = config.scale + 1;
    assert!(matches!(
        PorRuntime::open(
            directory.path(),
            state,
            ingress,
            config,
            SHARD_ID,
            WAVELENGTH
        ),
        Err(PorRuntimeError::InvalidConfig(
            cordial_por::PorError::InvalidConfiguration(_)
        ))
    ));
    assert!(!directory.path().join("por").exists());
}
