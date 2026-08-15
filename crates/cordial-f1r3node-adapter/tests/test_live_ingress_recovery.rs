//! Restart-recovery integration tests for `LiveIngress`.
//!
//! These exercise the full loop that issue #178 wires up:
//!
//!   ingest_and_persist -> (restart) -> with_persistent_store
//!
//! Every test writes to and reads from a real tempdir-backed LMDB
//! environment via `RSpaceBlocklaceRepository`. No mocking.
//!
//! Acceptance criteria covered:
//!   ✅ blocks written through `ingest_and_persist` survive a restart
//!   ✅ the finalized cursor written through `persist_finalized_cursor`
//!      survives a restart
//!   ✅ blocks persisted out of topological order are replayed correctly
//!      into the rehydrated mirror
//!   ✅ corrupt LMDB entries are skipped during recovery, not panicked on

use std::collections::{HashMap, HashSet};

use cordial_f1r3node_adapter::grpc_ingest::BlocklaceAdapter;
use cordial_f1r3node_adapter::live_ingress::{AlreadyValidatedVerifier, LiveIngress};
use cordial_f1r3node_adapter::repository::{BlocklaceRepository, RSpaceBlocklaceRepository};
use cordial_miners_core::Block;
use cordial_miners_core::crypto::hash_content;
use cordial_miners_core::types::{BlockContent, BlockIdentity, NodeId};
use tempfile::tempdir;

/// 10 MB — enough for tests, avoids large file allocation.
const MAP_SIZE: usize = 10 * 1024 * 1024;

/// Number of rounds in one consensus wave, matching `ES_WAVELENGTH` in
/// `snapshot.rs`.
const WAVELENGTH: u64 = 3;

// ── Test helpers ────────────────────────────────────────────────────────

#[derive(Default)]
struct TestAdapter;

impl BlocklaceAdapter<BlockIdentity> for TestAdapter {
    fn on_block(&mut self, _block: Block) -> anyhow::Result<()> {
        Ok(())
    }
}

fn open_repo(dir: &std::path::Path) -> RSpaceBlocklaceRepository {
    RSpaceBlocklaceRepository::open(dir, MAP_SIZE).expect("failed to open LMDB")
}

fn validator(tag: u8) -> NodeId {
    NodeId(vec![tag])
}

/// Build a block with zero or more known parent identities. Signatures are
/// dummy bytes: recovery in these tests always goes through
/// `AlreadyValidatedVerifier`, the same trust boundary `ingest_trusted_block`
/// uses elsewhere in this crate's test suite, so no real signing is needed.
fn make_block(creator: &NodeId, state_tag: u8, predecessors: &[BlockIdentity]) -> Block {
    let predecessors: HashSet<BlockIdentity> = predecessors.iter().cloned().collect();

    let content = BlockContent {
        payload: vec![state_tag],
        predecessors,
    };

    Block {
        identity: BlockIdentity {
            content_hash: hash_content(&content),
            creator: creator.clone(),
            signature: vec![state_tag; 64],
        },
        content,
    }
}

/// Build a single-validator chain where every block follows the one before it.
fn sequential_blocks(creator: &NodeId, count: usize) -> Vec<Block> {
    let mut blocks = Vec::with_capacity(count);

    for index in 0..count {
        let state_tag = u8::try_from(index + 1).expect("test block count should fit into u8");
        let predecessor = blocks.last().map(|block: &Block| block.identity.clone());
        let predecessors: Vec<BlockIdentity> = predecessor.into_iter().collect();

        blocks.push(make_block(creator, state_tag, &predecessors));
    }

    blocks
}

// ═══════════════════════════════════════════════════════════════════════
// Blocks survive restart
// ═══════════════════════════════════════════════════════════════════════

#[test]
fn live_ingress_blocks_survive_restart() {
    let dir = tempdir().unwrap();
    let creator = validator(1);
    let blocks = sequential_blocks(&creator, 5);

    {
        let repo = open_repo(dir.path());
        let (mut ingress, cursor) =
            LiveIngress::with_persistent_store(TestAdapter, &repo, &AlreadyValidatedVerifier)
                .expect("fresh store should hydrate cleanly");
        assert_eq!(cursor, None, "fresh store must have no finalized cursor");

        for block in &blocks {
            ingress
                .ingest_and_persist(block.clone(), &repo)
                .expect("block should persist and enter the mirror");
        }
        assert_eq!(ingress.blocklace().dom().len(), 5);
        // repo and ingress dropped here — LMDB env closed
    }

    {
        let repo = open_repo(dir.path());
        let (ingress, _cursor) =
            LiveIngress::with_persistent_store(TestAdapter, &repo, &AlreadyValidatedVerifier)
                .expect("reopened store should hydrate from disk");

        let dom: HashSet<BlockIdentity> = ingress.blocklace().dom().into_iter().cloned().collect();
        for block in &blocks {
            assert!(
                dom.contains(&block.identity),
                "block {:?} must survive restart",
                block.identity.content_hash
            );
        }
        assert_eq!(ingress.blocklace().dom().len(), 5);
    }
}

// ═══════════════════════════════════════════════════════════════════════
// Finalized cursor survives restart
// ═══════════════════════════════════════════════════════════════════════

#[test]
fn finalized_cursor_survives_restart() {
    let dir = tempdir().unwrap();
    let creator = validator(2);
    let blocks = sequential_blocks(&creator, 3);
    let mut bonds = HashMap::new();
    bonds.insert(creator.clone(), 100);

    let original_anchor = {
        let repo = open_repo(dir.path());
        let (mut ingress, _cursor) =
            LiveIngress::with_persistent_store(TestAdapter, &repo, &AlreadyValidatedVerifier)
                .expect("fresh store should hydrate cleanly");
        ingress.set_bonds(bonds.clone());

        for block in &blocks {
            ingress
                .ingest_and_persist(block.clone(), &repo)
                .expect("block should persist and enter the mirror");
        }

        let output = ingress
            .latest_finalized_ordered_output(WAVELENGTH)
            .expect("ordered output should be computable");
        let anchor = output
            .anchor
            .clone()
            .expect("three sequential blocks from one validator should finalize an anchor");

        ingress
            .persist_finalized_cursor(&repo)
            .expect("finalized cursor should persist");

        anchor
        // repo and ingress dropped here — LMDB env closed
    };

    let repo = open_repo(dir.path());
    let (_ingress, cursor) =
        LiveIngress::with_persistent_store(TestAdapter, &repo, &AlreadyValidatedVerifier)
            .expect("reopened store should hydrate from disk");

    assert_eq!(
        cursor,
        Some(original_anchor),
        "finalized cursor must survive restart and match the original anchor"
    );
}

// ═══════════════════════════════════════════════════════════════════════
// Recovery replays blocks in topological order into the mirror
// ═══════════════════════════════════════════════════════════════════════

#[test]
fn recovery_replays_in_topo_order_into_mirror() {
    // Build a small DAG and persist it deepest-first, directly through the
    // repository (bypassing any in-memory engine), to prove that
    // `with_persistent_store` — via `recover_into_engine`'s topological
    // sort — reconstructs the mirror correctly regardless of write order.
    let dir = tempdir().unwrap();
    let creator = validator(3);

    let genesis = make_block(&creator, 0x00, &[]);
    let block_a = make_block(&creator, 0x01, &[genesis.identity.clone()]);
    let block_b = make_block(&creator, 0x02, &[genesis.identity.clone()]);
    let block_c = make_block(&creator, 0x03, &[block_a.identity.clone()]);

    {
        let repo = open_repo(dir.path());
        // Intentionally deepest-first / out of order.
        repo.put_block(&block_c).unwrap();
        repo.put_block(&block_b).unwrap();
        repo.put_block(&block_a).unwrap();
        repo.put_block(&genesis).unwrap();
    }

    let repo = open_repo(dir.path());
    let (ingress, _cursor) =
        LiveIngress::with_persistent_store(TestAdapter, &repo, &AlreadyValidatedVerifier)
            .expect("recovery should replay out-of-order blocks");

    for block in [&genesis, &block_a, &block_b, &block_c] {
        assert!(
            ingress.blocklace().get(&block.identity).is_some(),
            "block {:?} should be present after topo-sorted replay",
            block.identity.content_hash
        );
    }
    assert!(
        ingress.pending_blocks().is_empty(),
        "no block should be left pending once all predecessors are known"
    );
}

// ═══════════════════════════════════════════════════════════════════════
// Corrupt LMDB entries are skipped, not panicked on
// ═══════════════════════════════════════════════════════════════════════

#[test]
fn recovery_skips_corrupt_lmdb_entries_without_panic() {
    use heed::EnvOpenOptions;
    use heed::types::Bytes;

    let dir = tempdir().unwrap();
    let db_path = dir.path().join("blocklace");
    std::fs::create_dir_all(&db_path).unwrap();

    let creator = validator(4);
    let good = make_block(&creator, 0xDD, &[]);

    {
        // Options must match RSpaceBlocklaceRepository::open() exactly —
        // see test_repository.rs for why each option matters.
        let env = unsafe {
            EnvOpenOptions::new()
                .map_size(MAP_SIZE)
                .max_dbs(10)
                .max_readers(128)
                .open(&db_path)
                .unwrap()
        };
        let mut wtxn = env.write_txn().unwrap();
        let db: heed::Database<Bytes, Bytes> = env
            .create_database(&mut wtxn, Some("cordial-blocks"))
            .unwrap();

        let good_key = bincode::serialize(&good.identity).unwrap();
        let good_val = bincode::serialize(&good).unwrap();
        db.put(&mut wtxn, &good_key, &good_val).unwrap();

        // Corrupt entry: valid key, garbage value.
        let bad_id = BlockIdentity {
            content_hash: [0xEEu8; 32],
            creator: NodeId(vec![0xEE]),
            signature: vec![0xEE],
        };
        let bad_key = bincode::serialize(&bad_id).unwrap();
        db.put(&mut wtxn, &bad_key, b"\xFF\xFF\xFF\xFF").unwrap();

        // Corrupt entry: completely invalid key and value.
        db.put(&mut wtxn, b"not_a_serialized_key", b"\x00\x01\x02")
            .unwrap();

        wtxn.commit().unwrap();
        // env dropped here — environment closed before the repository opens it
    }

    let repo = open_repo(dir.path());
    let (mut ingress, cursor) =
        LiveIngress::with_persistent_store(TestAdapter, &repo, &AlreadyValidatedVerifier)
            .expect("recovery must not panic or error on corrupt entries");

    assert_eq!(cursor, None);
    assert!(
        ingress.blocklace().get(&good.identity).is_some(),
        "valid block must survive recovery even when corrupt entries exist"
    );

    // The instance must remain fully usable after a recovery that hit
    // corruption — prove it by ingesting a fresh block.
    let next = make_block(&creator, 0x01, &[good.identity.clone()]);
    ingress
        .ingest_and_persist(next.clone(), &repo)
        .expect("live ingress must stay usable after recovering past corrupt entries");
    assert!(ingress.blocklace().get(&next.identity).is_some());
}
