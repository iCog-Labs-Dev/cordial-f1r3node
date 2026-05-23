//! Boot recovery for [`RSpaceBlocklaceRepository`].
//!
//! Two responsibilities:
//! 1. [`RSpaceBlocklaceRepository::recover_into_engine`] — reads all
//!    persisted blocks from LMDB and replays them into the in-memory
//!    consensus engine on node startup.
//! 2. [`topo_sort_blocks`] — sorts blocks by computed DAG height so
//!    predecessors are always inserted before their successors.
//!
! ## Crash safety

use std::collections::HashMap;

use tracing::{info, warn};

use cordial_miners_core::block::Block;
use cordial_miners_core::blocklace::Blocklace;
use cordial_miners_core::types::BlockIdentity;

use crate::error::RepoError;
use crate::repository::BlocklaceRepository;

use super::RSpaceBlocklaceRepository;

// ── Public: recover_into_engine ───────────────────────────────────────────

impl RSpaceBlocklaceRepository {
    /// Replay all persisted blocks into the in-memory `Blocklace` engine.
    ///
    /// Called **once at node startup**, before the node accepts any new
    /// gRPC blocks. The node must not accept blocks until this returns.
    ///
    /// ## Steps
    ///
    /// 1. Read `finalized_cursor` from `cordial-meta`.
    /// 2. Full scan of `cordial-blocks` — corrupt entries are warned
    ///    and skipped, never panicked.
    /// 3. Topological sort by computed DAG height — predecessors always
    ///    before successors, deterministic across runs.
    /// 4. `engine.insert` for each block — engine rejects bad blocks
    ///    with a warning, recovery continues.
    /// 5. Return the cursor so the caller logs the resume point.
    pub fn recover_into_engine(
        &self,
        engine: &mut Blocklace,
    ) -> Result<Option<BlockIdentity>, RepoError> {

        // Step 1 — read cursor
        let cursor = self.finalized_cursor()?;
        info!("Boot recovery starting. Finalized cursor: {:?}", cursor);

        // Step 2 — collect all blocks from LMDB
        // Lock is held only for the scan, released before engine work.
        let all_blocks: Vec<Block> = {
            let db   = self.blocks_db.lock()?;
            let rtxn = self.env.read_txn()?;

            db.iter(&rtxn)?
                .filter_map(|entry| {
                    let (_k, v) = entry
                        .map_err(|e| warn!("LMDB iter error during recovery: {e}"))
                        .ok()?;
                    bincode::deserialize::<Block>(v)
                        .map_err(|e| warn!("Skipping corrupt block: {e}"))
                        .ok()
                })
                .collect()
            // rtxn dropped — read lock released
            // db dropped   — Mutex released
            // put_block() can now proceed on other threads
        };

        info!("Boot recovery: {} blocks read from disk", all_blocks.len());

        // Step 3 — topological sort by DAG height
        let sorted = topo_sort_blocks(all_blocks);

        // Step 4 — replay into engine
        let mut replayed = 0usize;
        let mut skipped  = 0usize;

        for block in sorted {
            match engine.insert(block.clone()) {
                Ok(_)  => replayed += 1,
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

// ── Private: topo_sort_blocks ─────────────────────────────────────────────

/// Sort blocks in topological order by computed DAG height.
///
/// ## Correctness guarantee
///
/// Every block appears **after all of its predecessors** in the output.
/// Blocks at equal height retain their original relative order
/// (Rust's `sort_by_key` is stable).
///
/// ## Orphan handling
///
/// Blocks whose predecessors are absent from the input (partial
/// corruption, incomplete sync) are assigned height 0 and appended
/// early. `engine.insert` will reject them with `MissingPredecessor`,
/// which `recover_into_engine` catches and logs.

fn topo_sort_blocks(blocks: Vec<Block>) -> Vec<Block> {
    if blocks.is_empty() {
        return blocks;
    }

    // Index: content_hash → Block reference for predecessor lookup
    let block_map: HashMap<[u8; 32], &Block> = blocks
        .iter()
        .map(|b| (b.identity.content_hash, b))
        .collect();

    let mut height_cache: HashMap<[u8; 32], u64> = HashMap::new();

    // Compute height for every block via memoized recursion.
    for block in &blocks {
        compute_height(
            &block.identity.content_hash,
            &block_map,
            &mut height_cache,
        );
    }

    // Sort ascending by height — mirrors F1R3FLY's BTreeMap::range scan.
    // Stable sort preserves original order for equal-height blocks,
    // making recovery deterministic across runs.
    let mut sorted = blocks;
    sorted.sort_by_key(|b| {
        height_cache
            .get(&b.identity.content_hash)
            .copied()
            .unwrap_or(0)
    });

    sorted
}

/// Memoized height computation for a single block.
///
/// - genesis (no predecessors): height = 0
/// - all others: height = max(predecessor heights) + 1
/// - unknown predecessor (orphan): treated as height 0
fn compute_height(
    hash:         &[u8; 32],
    block_map:    &HashMap<[u8; 32], &Block>,
    height_cache: &mut HashMap<[u8; 32], u64>,
) -> u64 {
    if let Some(&cached) = height_cache.get(hash) {
        return cached;
    }

    let height = match block_map.get(hash) {
        None => 0,  // not in recovery set — orphan root
        Some(block) if block.content.predecessors.is_empty() => 0,  // genesis
        Some(block) => {
            let max_pred = block
                .content
                .predecessors
                .iter()
                .map(|pred| compute_height(
                    &pred.content_hash,
                    block_map,
                    height_cache,
                ))
                .max()
                .unwrap_or(0);
            max_pred + 1
        }
    };

    height_cache.insert(*hash, height);
    height
}