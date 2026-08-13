//! Node-level repository integration.
//!
//! This file wires `cordial-f1r3space-adapter`'s LMDB implementation
//! into the node adapter. All LMDB logic lives in the space adapter —

pub use cordial_f1r3space_adapter::{BlocklaceRepository, RSpaceBlocklaceRepository, RepoError};

use std::path::Path;

/// Production map size — 10 GB.
/// Override with a smaller value in tests.
pub const PRODUCTION_MAP_SIZE: usize = 10 * 1024 * 1024 * 1024;

/// Open the persistent block store at `data_dir/blocklace/`.
///
/// Call this once at node startup, then pass the returned repository
/// into [`CordialCasperAdapter`] and call
/// [`RSpaceBlocklaceRepository::recover_into_engine`] before accepting
/// any new gRPC blocks.
///
/// ## Startup lifecycle
///
/// 1. `let repo = open_block_store(&data_dir)?;`
/// 2. `let (ingress, cursor) = LiveIngress::with_persistent_store(adapter, bonds, shard_conf, shard_id, &repo, &verifier)?;`
/// 3. Log `cursor` — this is the last known finalized block identity.
/// 4. Start accepting gRPC blocks via `ingress.ingest_and_persist(block, &repo)`.
/// 5. After each ordered output advance, call `ingress.persist_finalized_cursor(&repo)`.
///
/// # Example
/// ```ignore
/// let repo = open_block_store(&data_dir)?;
/// let (ingress, cursor) = LiveIngress::with_persistent_store(
///     adapter, bonds, shard_conf, shard_id, &repo, &verifier,
/// )?;
/// ```
pub fn open_block_store(data_dir: &Path) -> Result<RSpaceBlocklaceRepository, RepoError> {
    RSpaceBlocklaceRepository::open(data_dir, PRODUCTION_MAP_SIZE)
}
