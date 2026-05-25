//! [`BlocklaceRepository`] implementation for [`RSpaceBlocklaceRepository`].
//!
//! Four methods, one responsibility each:
//!
//! | Method                  | Database   | Transaction |
//! |-------------------------|------------|-------------|
//! | `put_block`             | blocks_db  | write       |
//! | `get_block`             | blocks_db  | read        |
//! | `put_finalized_cursor`  | meta_db    | write       |
//! | `finalized_cursor`      | meta_db    | read        |
//!
//! ## Write-order invariant
//!
//! `put_block` must be called before `engine.insert()` in every
//! insertion path. A crash between the two is safe: recovery replays
//! from disk. A crash before `wtxn.commit()` leaves nothing written.

use cordial_miners_core::block::Block;
use cordial_miners_core::types::BlockIdentity;

use crate::error::RepoError;
use crate::repository::BlocklaceRepository;

use super::{CURSOR_KEY, RSpaceBlocklaceRepository};

impl BlocklaceRepository for RSpaceBlocklaceRepository {
    // ── put_block ─────────────────────────────────────────────────────────

    fn put_block(&self, block: &Block) -> Result<(), RepoError> {
        // Serialize before acquiring the lock — minimizes lock hold time.
        let key: Vec<u8> = bincode::serialize(&block.identity)?;
        let val: Vec<u8> = bincode::serialize(block)?;

        let db = self.blocks_db.lock()?;
        // LOCK ACQUIRED — blocks_db exclusively held by this thread
        let mut wtxn = self.env.write_txn()?;
        db.put(&mut wtxn, key.as_slice(), val.as_slice())?;
        wtxn.commit()?;
        // LOCK RELEASED — db drops here
        // Atomic: crash before commit = nothing written (safe).
        // Crash after commit = block durable before engine.insert (safe).
        Ok(())
    }

    // ── get_block ─────────────────────────────────────────────────────────

    fn get_block(&self, id: &BlockIdentity) -> Result<Option<Block>, RepoError> {
        let key: Vec<u8> = bincode::serialize(id)?;

        let db = self.blocks_db.lock()?;
        // LOCK ACQUIRED
        let rtxn = self.env.read_txn()?;
        // Copy bytes out before dropping — minimizes lock hold time.
        let bytes = db.get(&rtxn, key.as_slice())?.map(|b| b.to_vec());
        drop(rtxn); // read transaction closed — MVCC snapshot released
        drop(db); // LOCK RELEASED

        match bytes {
            None => Ok(None),
            Some(b) => Ok(Some(bincode::deserialize::<Block>(&b)?)),
        }
    }

    // ── put_finalized_cursor ──────────────────────────────────────────────

    fn put_finalized_cursor(&self, id: &BlockIdentity) -> Result<(), RepoError> {
        let val: Vec<u8> = bincode::serialize(id)?;

        let db = self.meta_db.lock()?;
        // LOCK ACQUIRED — meta_db exclusively held
        // Note: completely separate from blocks_db lock.
        // Both databases can be written concurrently by different threads.
        let mut wtxn = self.env.write_txn()?;
        db.put(&mut wtxn, CURSOR_KEY, val.as_slice())?;
        wtxn.commit()?;
        // LOCK RELEASED
        Ok(())
    }

    // ── finalized_cursor ──────────────────────────────────────────────────

    fn finalized_cursor(&self) -> Result<Option<BlockIdentity>, RepoError> {
        let db = self.meta_db.lock()?;
        // LOCK ACQUIRED
        let rtxn = self.env.read_txn()?;
        let bytes = db.get(&rtxn, CURSOR_KEY)?.map(|b| b.to_vec());
        drop(rtxn);
        drop(db);
        // LOCK RELEASED

        match bytes {
            None => Ok(None),
            Some(b) => Ok(Some(bincode::deserialize::<BlockIdentity>(&b)?)),
        }
    }
}
