use std::collections::{HashMap, HashSet};

use cordial_f1r3node_adapter::{
    grpc_ingest::BlocklaceAdapter,
    live_ingress::{AlreadyValidatedVerifier, LiveIngress, LiveIngressError},
    ordered_output::OrderedFinalizedOutput,
    repository::{BlocklaceRepository, RSpaceBlocklaceRepository, RepoError},
    shard_conf::CasperShardConf,
};
use cordial_f1r3space_adapter::lmdb_store::checkpoint::{
    CHECKPOINT_RECORD_KEY, CHECKPOINT_RECORD_VERSION, CheckpointRecord,
};
use cordial_miners_core::{Block, BlockContent, BlockIdentity, NodeId, crypto::block_hash};

// Four validators need enough sequential witnesses to super-ratify a leader.
const W: u64 = 12;
const MAP_SIZE: usize = 10 * 1024 * 1024;
struct AcceptBlock;
impl BlocklaceAdapter<BlockIdentity> for AcceptBlock {
    fn on_block(&mut self, _: Block) -> anyhow::Result<()> {
        Ok(())
    }
}
type Ingress = LiveIngress<AcceptBlock>;

fn bonds() -> HashMap<NodeId, u64> {
    (1..=4).map(|v| (NodeId(vec![v]), 25)).collect()
}
fn fresh() -> Ingress {
    LiveIngress::with_consensus_view(AcceptBlock, bonds(), CasperShardConf::default(), "root")
}
fn block(round: u64, parents: HashSet<BlockIdentity>) -> Block {
    // Rotate each wave's first creator with the round-robin elected leader.
    let creator = NodeId(vec![((round / W + round % 4) % 4 + 1) as u8]);
    let content = BlockContent {
        payload: round.to_be_bytes().to_vec(),
        predecessors: parents,
    };
    Block {
        identity: BlockIdentity {
            content_hash: block_hash(&creator, &content),
            creator,
            signature: vec![],
        },
        content,
    }
}
fn chain(count: u64) -> Vec<Block> {
    let mut blocks = Vec::new();
    let mut parents = HashSet::new();
    for round in 0..count {
        let next = block(round, parents);
        parents = HashSet::from([next.identity.clone()]);
        blocks.push(next);
    }
    blocks
}
fn feed(live: &mut Ingress, repo: &RSpaceBlocklaceRepository, blocks: &[Block]) {
    for block in blocks {
        live.ingest_and_persist(block.clone(), repo).unwrap();
    }
}
fn output(live: &mut Ingress) -> OrderedFinalizedOutput {
    live.latest_finalized_ordered_output(W).unwrap()
}
fn assert_order(actual: &OrderedFinalizedOutput, expected: &OrderedFinalizedOutput) {
    assert_eq!(actual.blocks, expected.blocks);
    assert_eq!(actual.anchor, expected.anchor);
}
fn record(live: &Ingress) -> CheckpointRecord {
    let lace = live.blocklace();
    CheckpointRecord {
        version: CHECKPOINT_RECORD_VERSION,
        checkpoint: lace.checkpoint().unwrap().clone(),
        origin: lace.checkpoint_origin().unwrap(),
        depth: lace.checkpoint_depth().unwrap(),
        order_prefix: lace.checkpoint_order_prefix().to_vec(),
        weighted_order_prefix: lace.checkpoint_weighted_order_prefix().to_vec(),
    }
}
fn restart(repo: &RSpaceBlocklaceRepository) -> Result<Ingress, RepoError> {
    LiveIngress::with_persistent_store(
        AcceptBlock,
        bonds(),
        CasperShardConf::default(),
        "root",
        repo,
        &AlreadyValidatedVerifier,
    )
    .map(|(live, _)| live)
}

#[test]
fn chain_checkpoint_actually_removes_blocks() {
    let dir = tempfile::tempdir().unwrap();
    let repo = RSpaceBlocklaceRepository::open(dir.path(), MAP_SIZE).unwrap();
    let mut live = fresh();
    let mut reference = fresh();
    let blocks = chain(4 * W);
    feed(&mut live, &repo, &blocks[..(3 * W) as usize]);
    for block in &blocks[..(3 * W) as usize] {
        reference.ingest_trusted_block(block.clone()).unwrap();
    }
    let before = output(&mut live);
    let report = live
        .checkpoint_prune_and_persist(W, &repo)
        .unwrap()
        .unwrap();
    println!(
        "ingested={}, retained={}, removed={}",
        3 * W,
        report.retained_blocks,
        report.removed.len()
    );
    assert!(!report.removed.is_empty());
    assert_order(&output(&mut live), &before);
    assert_order(&before, &output(&mut reference));
    let retained = live.blocklace().dom().len();
    drop(live);
    repo.close();
    let repo = RSpaceBlocklaceRepository::open(dir.path(), MAP_SIZE).unwrap();
    let mut recovered = restart(&repo).unwrap();
    assert_eq!(recovered.blocklace().dom().len(), retained);
    assert!(retained < (3 * W) as usize);
    assert!(
        blocks[..(3 * W) as usize]
            .iter()
            .all(|b| repo.get_block(&b.identity).unwrap().is_some())
    );
    let mut previous = output(&mut recovered);
    assert_order(&previous, &before);
    for block in &blocks[(3 * W) as usize..] {
        feed(&mut recovered, &repo, std::slice::from_ref(block));
        reference.ingest_trusted_block(block.clone()).unwrap();
        let actual = output(&mut recovered);
        assert_order(&actual, &output(&mut reference));
        assert!(actual.preserves_prefix(&previous));
        previous = actual;
    }
    // Simulate a crash after RAM pruning but before a newer record commits.
    let older = repo.checkpoint_record().unwrap().unwrap();
    let newer = recovered.checkpoint_and_prune(W).unwrap().unwrap();
    assert!(!newer.removed.is_empty());
    assert!(newer.checkpoint_depth > older.depth);
    drop(recovered);
    repo.close();
    let repo = RSpaceBlocklaceRepository::open(dir.path(), MAP_SIZE).unwrap();
    let mut recovered = restart(&repo).unwrap();
    assert_eq!(repo.checkpoint_record().unwrap(), Some(older));
    assert!(recovered.blocklace().dom().len() > newer.retained_blocks);
    assert_order(&output(&mut recovered), &output(&mut reference));
    assert!(
        blocks
            .iter()
            .all(|b| repo.get_block(&b.identity).unwrap().is_some())
    );
    repo.close();
}

fn metadata(dir: &std::path::Path, entries: &[(Vec<u8>, Option<Vec<u8>>)]) {
    // Match the existing repository's environment options when injecting faults.
    let env = unsafe {
        heed::EnvOpenOptions::new()
            .map_size(MAP_SIZE)
            .max_dbs(10)
            .max_readers(128)
            .open(dir.join("blocklace"))
            .unwrap()
    };
    let mut txn = env.write_txn().unwrap();
    let db: heed::Database<heed::types::Bytes, heed::types::Bytes> =
        env.create_database(&mut txn, Some("cordial-meta")).unwrap();
    for (key, value) in entries {
        match value {
            Some(value) => db.put(&mut txn, key, value).unwrap(),
            None => {
                db.delete(&mut txn, key).unwrap();
            }
        }
    }
    txn.commit().unwrap();
}

#[test]
fn no_record_replays_the_full_archive() {
    let dir = tempfile::tempdir().unwrap();
    let repo = RSpaceBlocklaceRepository::open(dir.path(), MAP_SIZE).unwrap();
    let blocks = chain(3 * W);
    let mut reference = fresh();
    feed(&mut reference, &repo, &blocks);
    let mut recovered = restart(&repo).unwrap();
    assert_eq!(recovered.blocklace().dom().len(), blocks.len());
    assert!(recovered.blocklace().checkpoint().is_none());
    assert_order(&output(&mut recovered), &output(&mut reference));
    repo.close();
}

#[test]
fn recovery_skips_pruned_mappings_and_late_blocks_but_rejects_unknown_mappings() {
    let dir = tempfile::tempdir().unwrap();
    let repo = RSpaceBlocklaceRepository::open(dir.path(), MAP_SIZE).unwrap();
    let blocks = chain(3 * W);
    let mut live = fresh();
    feed(&mut live, &repo, &blocks);
    repo.put_native_block(&blocks[0], &[7; 32]).unwrap();
    let report = live.checkpoint_and_prune(W).unwrap().unwrap();
    assert!(report.removed.contains(&blocks[0].identity));
    repo.put_checkpoint(&record(&live), &report.removed)
        .unwrap();
    let late = block(100, HashSet::from([blocks[0].identity.clone()]));
    repo.put_block(&late).unwrap();
    let mut recovered = restart(&repo).unwrap();
    assert_eq!(recovered.native_identity_for_wire_hash(&[7; 32]), None);
    assert!(recovered.blocklace().get(&late.identity).is_none());
    assert!(repo.get_block(&late.identity).unwrap().is_some());
    assert_eq!(recovered.blocklace().dom().len(), report.retained_blocks);
    assert_order(&output(&mut recovered), &output(&mut live));
    let unknown = block(101, HashSet::new()).identity;
    let encoded = bincode::serialize(&unknown).unwrap();
    metadata(
        dir.path(),
        &[
            (
                [b"native_hash/".as_slice(), &[8; 32]].concat(),
                Some(encoded.clone()),
            ),
            (
                [b"cordial_id/".as_slice(), &encoded].concat(),
                Some(vec![8; 32]),
            ),
        ],
    );
    assert!(matches!(restart(&repo), Err(RepoError::NativeMapping(_))));
    repo.close();
}

#[test]
fn corrupt_record_and_missing_checkpoint_fail_closed() {
    let dir = tempfile::tempdir().unwrap();
    let repo = RSpaceBlocklaceRepository::open(dir.path(), MAP_SIZE).unwrap();
    metadata(
        dir.path(),
        &[(CHECKPOINT_RECORD_KEY.to_vec(), Some(vec![255; 4]))],
    );
    assert!(matches!(restart(&repo), Err(RepoError::Checkpoint(_))));
    let mut live = fresh();
    for block in chain(3 * W) {
        live.ingest_trusted_block(block).unwrap();
    }
    assert!(
        !live
            .checkpoint_and_prune(W)
            .unwrap()
            .unwrap()
            .removed
            .is_empty()
    );
    metadata(
        dir.path(),
        &[(
            CHECKPOINT_RECORD_KEY.to_vec(),
            Some(bincode::serialize(&record(&live)).unwrap()),
        )],
    );
    assert!(
        matches!(restart(&repo), Err(RepoError::Checkpoint(message)) if message.contains("missing"))
    );
    repo.close();
}

#[test]
fn failed_persistence_retries_all_removals_and_is_idempotent() {
    let dir = tempfile::tempdir().unwrap();
    let repo = RSpaceBlocklaceRepository::open(dir.path(), MAP_SIZE).unwrap();
    let prune = |live: &mut Ingress| live.checkpoint_prune_and_persist(W, &repo);
    let mut live = fresh();
    let blocks = chain(4 * W);
    feed(&mut live, &repo, &blocks[..1]);
    assert!(prune(&mut live).unwrap().is_none());
    assert_eq!(repo.checkpoint_record().unwrap(), None);
    feed(&mut live, &repo, &blocks[1..W as usize]);
    assert!(prune(&mut live).unwrap().unwrap().removed.is_empty());
    assert_eq!(repo.checkpoint_record().unwrap(), None);
    feed(&mut live, &repo, &blocks[W as usize..(3 * W) as usize]);
    metadata(
        dir.path(),
        &[(CHECKPOINT_RECORD_KEY.to_vec(), Some(vec![255; 4]))],
    );
    assert!(matches!(
        prune(&mut live),
        Err(LiveIngressError::Persistence(_))
    ));
    let first_depth = live.blocklace().checkpoint_depth().unwrap();
    assert!(live.blocklace().get(&blocks[0].identity).is_none());
    assert!(!repo.is_pruned(&blocks[0].identity).unwrap());
    feed(&mut live, &repo, &blocks[(3 * W) as usize..]);
    assert!(matches!(
        prune(&mut live),
        Err(LiveIngressError::Persistence(_))
    ));
    assert!(live.blocklace().checkpoint_depth().unwrap() > first_depth);
    // Repair only the deliberately corrupt metadata, then retry with no advance.
    metadata(dir.path(), &[(CHECKPOINT_RECORD_KEY.to_vec(), None)]);
    assert!(prune(&mut live).unwrap().is_none());
    assert_eq!(repo.checkpoint_record().unwrap().unwrap(), record(&live));
    for block in &blocks[..(3 * W) as usize] {
        assert!(repo.is_pruned(&block.identity).unwrap());
        assert!(repo.get_block(&block.identity).unwrap().is_some());
    }
    assert!(prune(&mut live).unwrap().is_none());
    assert_eq!(repo.checkpoint_record().unwrap().unwrap(), record(&live));
    let mut recovered = restart(&repo).unwrap();
    assert_eq!(
        recovered.blocklace().dom().len(),
        live.blocklace().dom().len()
    );
    assert_order(&output(&mut recovered), &output(&mut live));
    repo.close();
}
