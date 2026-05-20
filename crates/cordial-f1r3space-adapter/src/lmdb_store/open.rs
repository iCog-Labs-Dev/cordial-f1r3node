//! Construction of [`RSpaceBlocklaceRepository`].
//!
//! One responsibility: open (or create) the LMDB environment and
//! the two named databases. All other behaviour lives in sibling modules.

use std::path::Path;
use std::sync::{Arc, Mutex};

use heed::EnvOpenOptions;

use crate::error::RepoError;

use super::{BLOCKS_DB, META_DB, RSpaceBlocklaceRepository};

impl RSpaceBlocklaceRepository {
    /// Open (or reopen) the LMDB environment at `data_dir/blocklace/`.
    ///
    /// ## Behaviour
    ///
    /// - **Fresh boot**: creates `data_dir/blocklace/` and both named
    ///   databases from scratch.
    /// - **Restart**: reopens the existing environment and databases.
    ///   `create_database` is idempotent — existing data is never lost.
    ///
    /// ## `map_size`
    ///
    /// Maximum size of the memory-mapped LMDB file:
    /// - Tests:      `10 * 1024 * 1024`        (10 MB)
    /// - Production: `10 * 1024 * 1024 * 1024` (10 GB)
    ///
    /// Mirrors `EnvOpenOptions` usage in F1R3FLY's
    /// `rspace_store_manager.rs` lines 89-94.
    pub fn open(data_dir: &Path, map_size: usize) -> Result<Self, RepoError> {
        let db_path = data_dir.join("blocklace");

        // Create directory if absent — mirrors rspace_store_manager.rs
        std::fs::create_dir_all(&db_path)?;

        // Open LMDB environment.
        // max_dbs(10): 2 used now, room for future indexes (Phase 4).
        // max_readers(128): concurrent read transactions allowed.
        let env = unsafe {
            EnvOpenOptions::new()
                .map_size(map_size)
                .max_dbs(10)
                .max_readers(128)
                .open(&db_path)?
        };

        // create_database is idempotent:
        //   fresh boot  → creates the named database
        //   restart     → opens the existing database (no data lost)
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
}