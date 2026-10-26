//! [`BlocklaceRepository`] implementation for [`RSpaceBlocklaceRepository`].
//!
//! Block, cursor, and native identity persistence:
//!
//! | Method                  | Database   | Transaction |
//! |-------------------------|------------|-------------|
//! | `put_block`             | blocks_db  | write       |
//! | `get_block`             | blocks_db  | read        |
//! | `put_finalized_cursor`  | meta_db    | write       |
//! | `finalized_cursor`      | meta_db    | read        |
//! | `put_native_block`      | both       | write       |
//! | `native_block_mappings` | meta_db    | read        |
//!
//! ## Write-order invariant
//!
//! `put_block` must be called before `engine.insert()` in ordinary persisted
//! insertion paths. The native path persists its block and identity mapping
//! together after the adapter callback and before mirror insertion. A crash
//! before transaction commit leaves neither durable; a crash after commit
//! leaves both for recovery.

use cordial_miners_core::block::Block;
use cordial_miners_core::types::BlockIdentity;

use crate::error::RepoError;
use crate::repository::BlocklaceRepository;

use super::{
    CORDIAL_ID_PREFIX, CURSOR_KEY, NATIVE_HASH_PREFIX, NATIVE_HEIGHT_CURSOR_KEY,
    RSpaceBlocklaceRepository,
};

impl RSpaceBlocklaceRepository {
    pub fn native_height_cursor(&self) -> Result<Option<i64>, RepoError> {
        let meta = self.meta_db.lock()?;
        let txn = self.env.read_txn()?;
        match meta.get(&txn, NATIVE_HEIGHT_CURSOR_KEY)? {
            Some(bytes) => {
                let height: i64 = bincode::deserialize(bytes)
                    .map_err(|_| RepoError::NativeCursor("stored value is corrupt".into()))?;
                if !(0..i64::MAX).contains(&height) {
                    return Err(RepoError::NativeCursor(
                        "stored value is out of range".into(),
                    ));
                }
                Ok(Some(height))
            }
            None => Ok(None),
        }
    }

    pub fn put_native_height_cursor(&self, height: i64) -> Result<(), RepoError> {
        if height < 0 || height == i64::MAX {
            return Err(RepoError::NativeCursor("height is out of range".into()));
        }
        let value = bincode::serialize(&height)?;
        let meta = self.meta_db.lock()?;
        let mut txn = self.env.write_txn()?;
        if let Some(bytes) = meta.get(&txn, NATIVE_HEIGHT_CURSOR_KEY)? {
            let previous: i64 = bincode::deserialize(bytes)
                .map_err(|_| RepoError::NativeCursor("stored value is corrupt".into()))?;
            if !(0..i64::MAX).contains(&previous) {
                return Err(RepoError::NativeCursor(
                    "stored value is out of range".into(),
                ));
            }
            if height < previous {
                return Err(RepoError::NativeCursor(
                    "height cannot move backward".into(),
                ));
            }
        }
        meta.put(&mut txn, NATIVE_HEIGHT_CURSOR_KEY, &value)?;
        txn.commit()?;
        Ok(())
    }

    /// Persist a verified native block and both identity directions atomically.
    /// Existing mappings may be replayed, but neither hash domain may be remapped.
    pub fn put_native_block(&self, block: &Block, wire_hash: &[u8; 32]) -> Result<(), RepoError> {
        let block_key = bincode::serialize(&block.identity)?;
        let block_value = bincode::serialize(block)?;
        let mut native_key = NATIVE_HASH_PREFIX.to_vec();
        native_key.extend_from_slice(wire_hash);
        let mut cordial_key = CORDIAL_ID_PREFIX.to_vec();
        cordial_key.extend_from_slice(&block_key);

        let blocks = self.blocks_db.lock()?;
        let meta = self.meta_db.lock()?;
        let mut txn = self.env.write_txn()?;
        if let Some(existing) = blocks.get(&txn, &block_key)?
            && existing != block_value
        {
            return Err(RepoError::NativeMapping(
                "Cordial identity already has different block contents".into(),
            ));
        }
        if let Some(existing) = meta.get(&txn, &native_key)?
            && existing != block_key
        {
            return Err(RepoError::NativeMapping(
                "native wire hash already maps to another Cordial identity".into(),
            ));
        }
        if let Some(existing) = meta.get(&txn, &cordial_key)?
            && existing != wire_hash
        {
            return Err(RepoError::NativeMapping(
                "distinct native wire hashes map to one Cordial identity".into(),
            ));
        }
        blocks.put(&mut txn, &block_key, &block_value)?;
        meta.put(&mut txn, &native_key, &block_key)?;
        meta.put(&mut txn, &cordial_key, wire_hash)?;
        txn.commit()?;
        Ok(())
    }

    /// Load mappings strictly: a damaged entry must not silently lose ancestry.
    pub fn native_block_mappings(&self) -> Result<Vec<([u8; 32], BlockIdentity)>, RepoError> {
        let meta = self.meta_db.lock()?;
        let txn = self.env.read_txn()?;
        let mut mappings = Vec::new();
        for entry in meta.iter(&txn)? {
            let (key, value) = entry?;
            if !key.starts_with(NATIVE_HASH_PREFIX) {
                continue;
            }
            let hash_bytes = &key[NATIVE_HASH_PREFIX.len()..];
            let wire_hash: [u8; 32] = hash_bytes.try_into().map_err(|_| {
                RepoError::NativeMapping("native wire hash has the wrong length".into())
            })?;
            let identity: BlockIdentity = bincode::deserialize(value).map_err(|_| {
                RepoError::NativeMapping("native wire hash has an invalid Cordial identity".into())
            })?;
            let mut cordial_key = CORDIAL_ID_PREFIX.to_vec();
            cordial_key.extend_from_slice(value);
            if meta.get(&txn, &cordial_key)? != Some(wire_hash.as_slice()) {
                return Err(RepoError::NativeMapping(
                    "native and Cordial identity mappings disagree".into(),
                ));
            }
            mappings.push((wire_hash, identity));
        }
        Ok(mappings)
    }
}

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
