//! LMDB-backed implementation of [`BlocklaceRepository`].
//!
//! ## On-disk layout
//!
//! One LMDB environment at `data_dir/blocklace/`, two named databases:
//!
//! - `cordial-blocks` — key: `bincode(BlockIdentity)`,
//!                      val: `bincode(Block)`
//! - `cordial-meta`   — key: `b"finalized_cursor"`,
//!                      val: `bincode(BlockIdentity)`
//!
//! ## Patterns borrowed from F1R3FLY
//!
//! - `Arc<Mutex<Database>>` for thread safety
//!   (mirrors `lmdb_key_value_store.rs` lines 10-13)
//! - `EnvOpenOptions` with `map_size` / `max_dbs` / `max_readers`
//!   (mirrors `rspace_store_manager.rs` lines 89-94)
//! - `create_dir_all` before opening the environment
//!   (mirrors `rspace_store_manager.rs` lines 55-65)
//! - `write_txn` + `commit()` for atomic writes
//!   (mirrors `lmdb_key_value_store.rs` lines 32-46)
//! - `read_txn` dropped before deserializing
//!   (mirrors `lmdb_key_value_store.rs` lines 16-30)

use std::path::Path;
use std::sync::{Arc, Mutex};

use heed::types::Bytes;
use heed::{Database, Env, EnvOpenOptions};
use tracing::{info, warn};

use cordial_miners_core::block::Block;
use cordial_miners_core::types::BlockIdentity;

use crate::error::RepoError;
use crate::repository::BlocklaceRepository;

// ── LMDB database names ───────────────────────────────────────────────────
const BLOCKS_DB:  &str  = "cordial-blocks";
const META_DB:    &str  = "cordial-meta";
const CURSOR_KEY: &[u8] = b"finalized_cursor";

// ── Struct ────────────────────────────────────────────────────────────────

/// LMDB-backed block storage.
///
/// Holds two named databases inside a single LMDB environment.
/// Construct with [`RSpaceBlocklaceRepository::open`].
pub struct RSpaceBlocklaceRepository {
    env:       Arc<Env>,
    blocks_db: Arc<Mutex<Database<Bytes, Bytes>>>,
    meta_db:   Arc<Mutex<Database<Bytes, Bytes>>>,
}

// ── Construction ──────────────────────────────────────────────────────────

impl RSpaceBlocklaceRepository {
    /// Open (or reopen) the LMDB environment at `data_dir/blocklace/`.
    ///
    /// - Creates the directory if it does not exist (fresh boot).
    /// - Reopens existing databases without data loss (restart).
    ///
    /// `map_size` — maximum size of the memory-mapped file:
    ///   - tests:      `10 * 1024 * 1024`        (10 MB)
    ///   - production: `10 * 1024 * 1024 * 1024` (10 GB)
    pub fn open(data_dir: &Path, map_size: usize) -> Result<Self, RepoError> {
        let db_path = data_dir.join("blocklace");
        std::fs::create_dir_all(&db_path)?;

        // Safety: heed requires unsafe for open() on some versions.
        // This mirrors the pattern in rspace_store_manager.rs.
        let env = unsafe {
            EnvOpenOptions::new()
                .map_size(map_size)
                .max_dbs(10)       // 2 used now; room for future indexes
                .max_readers(128)  // matches rspace_store_manager.rs
                .open(&db_path)?
        };

        // create_database is idempotent:
        // - fresh boot  → creates the database
        // - restart     → opens the existing database (no data lost)
        let mut wtxn = env.write_txn()?;
        let blocks_db = env.create_database(&mut wtxn, Some(BLOCKS_DB))?;
        let meta_db   = env.create_database(&mut wtxn, Some(META_DB))?;
        wtxn.commit()?;

        Ok(Self {
            env:       Arc::new(env),
            blocks_db: Arc::new(Mutex::new(blocks_db)),
            meta_db:   Arc::new(Mutex::new(meta_db)),
        })
    }

    // ── Boot recovery ─────────────────────────────────────────────────────

    /// Replay all persisted blocks into the in-memory `Blocklace` engine.
    ///
    /// Called **once at node startup**, before the node accepts any new
    /// gRPC blocks. Exported so `cordial-f1r3node-adapter` can call it
    /// from its startup sequence.
    ///
    /// # Steps
    /// 1. Read `finalized_cursor` from `cordial-meta`.
    /// 2. Full scan of `cordial-blocks` — corrupt entries are warned
    ///    and skipped, never panicked.
    /// 3. Sort ascending by predecessor-set size (depth proxy) so
    ///    `engine.insert` never sees a block before its predecessors.
    /// 4. Call `engine.insert` for each block.
    /// 5. Return the cursor so the caller can log the resume point.
    pub fn recover_into_engine(
        &self,
        engine: &mut cordial_miners_core::blocklace::Blocklace,
    ) -> Result<Option<BlockIdentity>, RepoError> {

        // Step 1 — read cursor
        let cursor = self.finalized_cursor()?;
        info!("Boot recovery starting. Finalized cursor: {:?}", cursor);

        // Step 2 — collect all blocks from LMDB
        // The read_txn is held only for the duration of the scan,
        // then dropped before we touch the engine.
        let all_blocks: Vec<Block> = {
            let db   = self.blocks_db.lock()?;
            let rtxn = self.env.read_txn()?;

            db.iter(&rtxn)?
                .filter_map(|entry| {
                    let (_k, v) = entry
                        .map_err(|e| warn!("LMDB iter error during recovery: {e}"))
                        .ok()?;
                    bincode::deserialize::<Block>(v)
                        .map_err(|e| {
                            warn!("Skipping corrupt block during recovery: {e}");
                        })
                        .ok()
                })
                .collect()
            // rtxn and db lock released here
        };

        info!("Boot recovery: {} blocks read from disk", all_blocks.len());

        // Step 3 — sort by predecessor count as depth proxy
        // Blocks with no predecessors (genesis) always go first.
        let mut sorted = all_blocks;
        sorted.sort_by_key(|b| b.content.predecessors.len());

        // Step 4 — replay into engine
        let mut replayed = 0usize;
        let mut skipped  = 0usize;

        for block in sorted {
            match engine.insert(block.clone()) {
                Ok(_) => replayed += 1,
                Err(e) => {
                    warn!(
                        "Engine rejected block {:?} during recovery: {:?}",
                        block.identity.content_hash, e
                    );
                    skipped += 1;
                }
            }
        }

        info!(
            "Boot recovery complete. replayed={replayed}, skipped={skipped}"
        );

        // Step 5 — return cursor to caller
        Ok(cursor)
    }
}

// ── BlocklaceRepository impl ──────────────────────────────────────────────

impl BlocklaceRepository for RSpaceBlocklaceRepository {

    fn put_block(&self, block: &Block) -> Result<(), RepoError> {
        // key = bincode-serialized BlockIdentity
        // val = bincode-serialized full Block
        let key: Vec<u8> = bincode::serialize(&block.identity)?;
        let val: Vec<u8> = bincode::serialize(block)?;

        let db = self.blocks_db.lock()?;
        let mut wtxn = self.env.write_txn()?;
        db.put(&mut wtxn, key.as_slice(), val.as_slice())?;
        wtxn.commit()?;
        // Atomic: crash before commit = nothing written (safe).
        // Crash after commit = block on disk before engine sees it (safe).

        Ok(())
    }

    fn get_block(&self, id: &BlockIdentity) -> Result<Option<Block>, RepoError> {
        let key: Vec<u8> = bincode::serialize(id)?;

        let db   = self.blocks_db.lock()?;
        let rtxn = self.env.read_txn()?;
        // Copy bytes out before dropping rtxn to release the read lock promptly.
        let bytes = db.get(&rtxn, key.as_slice())?.map(|b| b.to_vec());
        drop(rtxn);
        drop(db);

        match bytes {
            None    => Ok(None),
            Some(b) => Ok(Some(bincode::deserialize::<Block>(&b)?)),
        }
    }

    fn put_finalized_cursor(&self, id: &BlockIdentity) -> Result<(), RepoError> {
        let val: Vec<u8> = bincode::serialize(id)?;

        let db = self.meta_db.lock()?;
        let mut wtxn = self.env.write_txn()?;
        db.put(&mut wtxn, CURSOR_KEY, val.as_slice())?;
        wtxn.commit()?;

        Ok(())
    }

    fn finalized_cursor(&self) -> Result<Option<BlockIdentity>, RepoError> {
        let db   = self.meta_db.lock()?;
        let rtxn = self.env.read_txn()?;
        let bytes = db.get(&rtxn, CURSOR_KEY)?.map(|b| b.to_vec());
        drop(rtxn);
        drop(db);

        match bytes {
            None    => Ok(None),
            Some(b) => Ok(Some(bincode::deserialize::<BlockIdentity>(&b)?)),
        }
    }
}