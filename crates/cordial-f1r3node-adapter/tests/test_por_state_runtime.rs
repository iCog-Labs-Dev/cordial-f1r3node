use std::{cell::RefCell, collections::HashSet};

use cordial_f1r3node_adapter::{
    ordered_output::OrderedFinalizedOutput,
    por::{
        CompletedPorRatingRound, DurablePorState, DurablePorStateError, PorFinalityTracker,
        PorRatingRoundCoordinator, PorRatingRoundCutoffPolicy, PorStateStore, PorStateStoreError,
        RatingEnvelopeBroadcaster,
    },
};
use cordial_miners_core::{
    Block, BlockContent, BlockIdentity, Blocklace, NodeId, crypto::CryptoVerifier,
};
use cordial_por::{PorConfig, PorError, ReputationState, reputation_weights};
use k256::ecdsa::SigningKey;
use tempfile::tempdir;

const WAVELENGTH: u64 = 3;
const SHARD_ID: &[u8] = b"root";

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

struct Fixture {
    blocklace: Blocklace,
    output: OrderedFinalizedOutput,
    state: ReputationState,
    config: PorConfig,
}

fn private_key(seed: u8) -> [u8; 32] {
    [seed; 32]
}

fn node(seed: u8) -> NodeId {
    let signing_key = SigningKey::from_slice(&private_key(seed)).unwrap();
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

fn fixture() -> Fixture {
    let mut blocklace = Blocklace::new();
    let leader = block(1, 1, None);
    let second = block(2, 2, Some(&leader.identity));
    let third = block(3, 1, Some(&second.identity));

    for block in [&leader, &second, &third] {
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
        3,
        3,
    )
    .with_timestamp(0);

    let mut state = ReputationState::new(0);
    for seed in [1, 2, 9] {
        state.set_reputation(node(seed), 100);
    }

    Fixture {
        blocklace,
        output,
        state,
        config: PorConfig::default(),
    }
}

fn completed_round(fixture: &Fixture) -> CompletedPorRatingRound {
    let opened = PorFinalityTracker::new()
        .observe_finalized_output(&fixture.blocklace, &fixture.output)
        .unwrap()
        .unwrap();
    let mut coordinator = PorRatingRoundCoordinator::new(
        &fixture.blocklace,
        &fixture.output,
        opened,
        &fixture.state,
        &fixture.config,
    )
    .unwrap();
    coordinator
        .produce_local_batch(&node(9), &private_key(9))
        .unwrap();
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

#[test]
fn fresh_startup_durably_initializes_state() {
    let directory = tempdir().unwrap();
    let expected = fixture().state;
    let runtime = DurablePorState::open(directory.path(), expected.clone()).unwrap();

    assert_eq!(runtime.state().unwrap(), &expected);
    assert!(runtime.snapshot_path().is_file());
    assert_eq!(
        PorStateStore::open(directory.path())
            .unwrap()
            .restore()
            .unwrap(),
        Some(expected)
    );
}

#[test]
fn completed_round_is_durable_before_reopen_exposes_it() {
    let directory = tempdir().unwrap();
    let fixture = fixture();
    let completed = completed_round(&fixture);
    let mut runtime = DurablePorState::open(directory.path(), fixture.state.clone()).unwrap();

    let applied = runtime
        .apply_completed_round(&completed, &fixture.config, SHARD_ID)
        .unwrap();
    let committed = runtime.state().unwrap().clone();

    assert_eq!(committed.round(), completed.batch().round);
    assert_eq!(committed.latest_block(), Some(&applied.block));
    assert_eq!(applied.weights, reputation_weights(&committed));
    drop(runtime);

    let fallback = ReputationState::new(99);
    let reopened = DurablePorState::open(directory.path(), fallback).unwrap();
    assert_eq!(reopened.state().unwrap(), &committed);
}

#[test]
fn transition_failure_preserves_disk_and_keeps_runtime_usable() {
    let directory = tempdir().unwrap();
    let fixture = fixture();
    let completed = completed_round(&fixture);
    let before = fixture.state.clone();
    let mut runtime = DurablePorState::open(directory.path(), before.clone()).unwrap();

    assert!(matches!(
        runtime.apply_completed_round(&completed, &fixture.config, b""),
        Err(DurablePorStateError::Transition(
            PorError::MissingReputationBlockShardId
        ))
    ));
    assert_eq!(runtime.state().unwrap(), &before);
    assert_eq!(
        PorStateStore::open(directory.path())
            .unwrap()
            .restore()
            .unwrap(),
        Some(before)
    );

    runtime
        .apply_completed_round(&completed, &fixture.config, SHARD_ID)
        .unwrap();
    assert_eq!(runtime.state().unwrap().round(), completed.batch().round);
}

#[test]
fn persistence_failure_fail_closes_until_startup_recovery() {
    let directory = tempdir().unwrap();
    let fixture = fixture();
    let completed = completed_round(&fixture);
    let initial = fixture.state.clone();
    let mut runtime = DurablePorState::open(directory.path(), initial.clone()).unwrap();

    let temporary_path = runtime
        .snapshot_path()
        .parent()
        .unwrap()
        .join(".reputation-state.bin.tmp");
    std::fs::create_dir(temporary_path).unwrap();

    assert!(matches!(
        runtime.apply_completed_round(&completed, &fixture.config, SHARD_ID),
        Err(DurablePorStateError::Persistence(PorStateStoreError::Io(_)))
    ));
    assert!(matches!(
        runtime.state(),
        Err(DurablePorStateError::RecoveryRequired)
    ));
    assert!(matches!(
        runtime.apply_completed_round(&completed, &fixture.config, SHARD_ID),
        Err(DurablePorStateError::RecoveryRequired)
    ));
    drop(runtime);

    let reopened = DurablePorState::open(directory.path(), ReputationState::new(99)).unwrap();
    assert_eq!(reopened.state().unwrap(), &initial);
}

#[test]
fn corrupt_snapshot_is_a_startup_error_not_a_fresh_initialization() {
    let directory = tempdir().unwrap();
    let initial = fixture().state;
    let store = PorStateStore::open(directory.path()).unwrap();
    store.persist(&initial).unwrap();

    let mut bytes = std::fs::read(store.snapshot_path()).unwrap();
    let midpoint = bytes.len() / 2;
    bytes[midpoint] ^= 1;
    std::fs::write(store.snapshot_path(), bytes).unwrap();

    assert!(matches!(
        DurablePorState::open(directory.path(), ReputationState::new(99)),
        Err(DurablePorStateError::Persistence(
            PorStateStoreError::InvalidSnapshot(PorError::ReputationStateSnapshotChecksumMismatch)
        ))
    ));
}
