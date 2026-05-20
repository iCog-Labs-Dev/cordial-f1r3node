//! The storage contract for the blocklace DAG.
//!
//! This trait is the public API that `cordial-f1r3node-adapter`
//! depends on. The implementation lives in `lmdb_store.rs`.

use cordial_miners_core::block::Block;
use cordial_miners_core::types::BlockIdentity;

use crate::error::RepoError;

/// Four methods — maps directly to the GitHub issue acceptance criteria.
///
/// Implemented by [`crate::lmdb_store::RSpaceBlocklaceRepository`].
/// The trait is object-safe so it can be held as
/// `Arc<dyn BlocklaceRepository>` inside `CordialCasperAdapter`.
pub trait BlocklaceRepository: Send + Sync {
    /// Serialize and persist a [`Block`].
    ///
    /// **Must be called before `engine.insert()`** — the write-order
    /// invariant guarantees disk is always ahead of memory. A crash
    /// between `put_block` and `engine.insert` is safe: recovery
    /// replays from disk on restart.
    fn put_block(&self, block: &Block) -> Result<(), RepoError>;

    /// Retrieve a [`Block`] by its [`BlockIdentity`].
    ///
    /// Returns `Ok(None)` when the block is not found.
    /// Returns `Err` only for I/O failures or corrupt data.
    fn get_block(&self, id: &BlockIdentity) -> Result<Option<Block>, RepoError>;

    /// Advance the finalized-cursor bookmark.
    ///
    /// Called every time the Casper layer detects a new last-finalized
    /// block. This is the crash-safe resume point: on restart the node
    /// knows where finality was before the crash.
    fn put_finalized_cursor(&self, id: &BlockIdentity) -> Result<(), RepoError>;

    /// Read the finalized cursor.
    ///
    /// Returns `Ok(None)` on first boot — no prior state on disk.
    fn finalized_cursor(&self) -> Result<Option<BlockIdentity>, RepoError>;
}