use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
};

use cordial_f1r3node_adapter::{
    grpc_ingest::BlocklaceAdapter,
    live_ingress::{LiveIngress, PorWeightActivationError},
    ordered_output::OrderedFinalizedOutput,
    por::{
        CompletedPorRatingRound, DurablePorStateError, PorFinalityTracker,
        PorRatingRoundCoordinator, PorRatingRoundCutoffPolicy, PorRuntime, PorRuntimeError,
        PorWeightActivationOutcome, RatingEnvelopeBroadcaster,
    },
    shard_conf::CasperShardConf,
    shared_ordered_output::ReadOrderedOutput,
    snapshot::CORDIAL_WAVELENGTH,
};
use cordial_miners_core::{
    Block, BlockContent, BlockIdentity, Blocklace, NodeId, crypto::CryptoVerifier,
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
