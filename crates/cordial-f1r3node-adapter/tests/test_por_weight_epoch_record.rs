use std::collections::{HashMap, HashSet};

use cordial_f1r3node_adapter::{
    grpc_ingest::BlocklaceAdapter,
    live_ingress::LiveIngress,
    por::{
        PorRuntime, PorWeightEpochPolicy, PorWeightEpochRecord, PorWeightEpochRecordError,
        PorWeightEpochStore, finalized_prefix_checkpoint,
    },
    shard_conf::CasperShardConf,
};
use cordial_miners_core::{
    Block, BlockContent, BlockIdentity, NodeId,
    consensus::{depth, wave_of_round},
};
use cordial_por::{PorConfig, ReputationState};
use tempfile::tempdir;

struct AcceptBlocks;

impl BlocklaceAdapter<BlockIdentity> for AcceptBlocks {
    fn on_block(&mut self, _block: Block) -> anyhow::Result<()> {
        Ok(())
    }
}

fn fixture() -> (LiveIngress<AcceptBlocks>, PorWeightEpochRecord, [u8; 32]) {
    let validator = NodeId(vec![1]);
    let mut ingress = LiveIngress::with_consensus_view(
        AcceptBlocks,
        HashMap::from([(validator.clone(), 10)]),
        CasperShardConf::default(),
        "root",
    );
    let mut predecessor: Option<BlockIdentity> = None;
    for index in 0..30u8 {
        let mut hash = [0; 32];
        hash[0] = index + 1;
        let identity = BlockIdentity {
            content_hash: hash,
            creator: validator.clone(),
            signature: Vec::new(),
        };
        ingress
            .ingest_trusted_block(Block {
                identity: identity.clone(),
                content: BlockContent {
                    payload: Vec::new(),
                    predecessors: predecessor.into_iter().collect::<HashSet<_>>(),
                },
            })
            .unwrap();
        predecessor = Some(identity);
    }
    let published = ingress.latest_finalized_ordered_output(3).unwrap();
    let source = finalized_prefix_checkpoint(&published, 1)
        .unwrap()
        .expect("one finalized checkpoint");
    let source_wave = wave_of_round(
        depth(ingress.blocklace(), &source.terminal_block).unwrap(),
        3,
    )
    .unwrap();
    let boundary_wave = wave_of_round(
        depth(ingress.blocklace(), published.anchor.as_ref().unwrap()).unwrap(),
        3,
    )
    .unwrap();
    let policy = PorWeightEpochPolicy::new(boundary_wave + 1 - source_wave).unwrap();
    let checkpoint_hash = [42; 32];
    let record = PorWeightEpochRecord::prepare(
        &ingress,
        &source,
        checkpoint_hash,
        &HashMap::from([(validator, 20)]),
        policy,
    )
    .unwrap();
    (ingress, record, checkpoint_hash)
}

#[test]
fn prepared_epoch_survives_store_reopen_and_replays_sealed_prefix() {
    let directory = tempdir().unwrap();
    let (mut ingress, record, checkpoint_hash) = fixture();
    let store = PorWeightEpochStore::open(directory.path()).unwrap();
    assert!(store.restore().unwrap().is_none());
    store.persist(&record).unwrap();
    store.persist(&record).unwrap();

    let recovered = PorWeightEpochStore::open(directory.path())
        .unwrap()
        .restore()
        .unwrap()
        .unwrap();
    assert_eq!(recovered, record);
    assert_eq!(recovered.por_round(), 1);
    assert_eq!(recovered.new_weights().get(&NodeId(vec![1])), Some(&20));
    assert!(recovered.effective_wave() > 0);
    let replayed = recovered
        .verify_replay(ingress.blocklace(), &[NodeId(vec![1])], checkpoint_hash)
        .unwrap();
    assert!(replayed.blocks.len() >= 16);
    assert_eq!(ingress.bonds().get(&NodeId(vec![1])), Some(&10));

    let mut previous_hash = [0; 32];
    previous_hash[0] = 30;
    let mut next_hash = [0; 32];
    next_hash[0] = 31;
    ingress
        .ingest_trusted_block(Block {
            identity: BlockIdentity {
                content_hash: next_hash,
                creator: NodeId(vec![1]),
                signature: Vec::new(),
            },
            content: BlockContent {
                payload: Vec::new(),
                predecessors: HashSet::from([BlockIdentity {
                    content_hash: previous_hash,
                    creator: NodeId(vec![1]),
                    signature: Vec::new(),
                }]),
            },
        })
        .unwrap();
    let later = recovered
        .verify_replay(ingress.blocklace(), &[NodeId(vec![1])], checkpoint_hash)
        .unwrap();
    assert!(later.blocks.starts_with(&replayed.blocks));

    assert!(matches!(
        recovered.verify_replay(ingress.blocklace(), &[NodeId(vec![1])], [7; 32]),
        Err(PorWeightEpochRecordError::Malformed(
            "reputation checkpoint changed"
        ))
    ));
    assert!(matches!(
        recovered.verify_replay(ingress.blocklace(), &[NodeId(vec![2])], checkpoint_hash),
        Err(PorWeightEpochRecordError::Malformed(
            "authorized validator set changed"
        ))
    ));
}

#[test]
fn runtime_discards_epoch_proposal_when_reputation_state_never_committed() {
    let directory = tempdir().unwrap();
    let (_, record, _) = fixture();
    let store = PorWeightEpochStore::open(directory.path()).unwrap();
    store.persist(&record).unwrap();
    let validator = NodeId(vec![1]);
    let mut initial = ReputationState::new(0);
    initial.set_reputation(validator.clone(), 100);
    let ingress = LiveIngress::with_consensus_view(
        AcceptBlocks,
        HashMap::from([(validator, 10)]),
        CasperShardConf::default(),
        "root",
    );
    let runtime = PorRuntime::open(
        directory.path(),
        initial,
        ingress,
        PorConfig::default(),
        b"root",
        3,
    )
    .unwrap();
    assert_eq!(runtime.state().state().unwrap().round(), 0);
    assert!(runtime.active_future_epoch().is_none());
    assert!(store.restore().unwrap().is_none());
}

#[test]
fn epoch_store_rejects_conflicting_record_and_corrupt_bytes() {
    let directory = tempdir().unwrap();
    let (ingress, record, checkpoint_hash) = fixture();
    let store = PorWeightEpochStore::open(directory.path()).unwrap();
    store.persist(&record).unwrap();
    let changed = PorWeightEpochRecord::prepare(
        &ingress,
        record.source(),
        checkpoint_hash,
        &HashMap::from([(NodeId(vec![1]), 30)]),
        PorWeightEpochPolicy::new(record.delay_waves()).unwrap(),
    )
    .unwrap();
    assert!(matches!(
        store.persist(&changed),
        Err(PorWeightEpochRecordError::ConflictingRecord)
    ));

    let mut bytes = std::fs::read(store.record_path()).unwrap();
    let last = bytes.len() - 1;
    bytes[last] ^= 1;
    std::fs::write(store.record_path(), bytes).unwrap();
    assert!(matches!(
        store.restore(),
        Err(PorWeightEpochRecordError::ChecksumMismatch)
    ));
}
