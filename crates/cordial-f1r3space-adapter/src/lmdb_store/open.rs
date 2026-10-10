//! Construction of [`RSpaceBlocklaceRepository`].
//!
//! One responsibility: open (or create) the LMDB environment and
//! the two named databases. All other behaviour lives in sibling modules.

use std::path::Path;
use std::sync::{Arc, Mutex};

use heed::EnvOpenOptions;

use crate::error::RepoError;

use super::{
    BLOCK_HASH_FORMAT_VERSION, BLOCK_HASH_FORMAT_VERSION_KEY, BLOCKS_DB, META_DB,
    RSpaceBlocklaceRepository,
};

impl RSpaceBlocklaceRepository {
    /// Open (or reopen) the LMDB environment at `data_dir/blocklace/`.
    ///
    /// ## Behaviour
    ///
    /// - **Fresh boot**: creates `data_dir/blocklace/` and both named
    ///   databases from scratch.
    /// - **Restart**: reopens an environment written with the current block
    ///   hash format. `create_database` is idempotent — existing data is never
    ///   lost.
    /// - **Unversioned non-empty store**: fails explicitly and requires a
    ///   resync. Such stores may contain blocks signed under the legacy hash
    ///   that included predecessor signatures; silently replaying and skipping
    ///   them would expose a partial DAG.
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
        let meta_db = env.create_database(&mut wtxn, Some(META_DB))?;

        match meta_db.get(&wtxn, BLOCK_HASH_FORMAT_VERSION_KEY)? {
            Some(encoded) => {
                let encoded: [u8; 4] = <[u8; 4]>::try_from(encoded).map_err(|_| {
                    RepoError::IncompatibleStorage(
                        "invalid block hash format marker; resynchronize the blocklace store"
                            .into(),
                    )
                })?;
                let found = u32::from_be_bytes(encoded);
                if found != BLOCK_HASH_FORMAT_VERSION {
                    return Err(RepoError::IncompatibleStorage(format!(
                        "block hash format version {found} is not supported; expected version \
                         {BLOCK_HASH_FORMAT_VERSION}. Resynchronize the blocklace store"
                    )));
                }
            }
            None if blocks_db.is_empty(&wtxn)? => {
                let version = BLOCK_HASH_FORMAT_VERSION.to_be_bytes();
                meta_db.put(&mut wtxn, BLOCK_HASH_FORMAT_VERSION_KEY, version.as_slice())?;
            }
            None => {
                return Err(RepoError::IncompatibleStorage(
                    "non-empty unversioned store may contain legacy hashes that included \
                     predecessor signatures; resynchronize the blocklace store"
                        .into(),
                ));
            }
        }
        wtxn.commit()?;

        Ok(Self {
            env: Arc::new(env),
            blocks_db: Arc::new(Mutex::new(blocks_db)),
            meta_db: Arc::new(Mutex::new(meta_db)),
        })
    }
}
