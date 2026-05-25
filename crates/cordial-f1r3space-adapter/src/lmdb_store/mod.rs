//! LMDB-backed implementation of [`BlocklaceRepository`].
//!
//! Split into three focused submodules:
//! - [`open`]             — environment construction
//! - [`recovery`]         — boot recovery and topological sort
//! - [`repository_impl`]  — [`BlocklaceRepository`] trait implementation

pub mod open;
pub mod recovery;
pub mod repository_impl;

use std::sync::{Arc, Mutex};

use heed::types::Bytes;
use heed::{Database, Env};

// ── Shared constants ──────────────────────────────────────────────────────
//
// Defined here so all three submodules import from one place.
// A change to a database name is made exactly once.

/// Named database for block storage.
/// key: `bincode(BlockIdentity)` — val: `bincode(Block)`
pub(crate) const BLOCKS_DB: &str = "cordial-blocks";

/// Named database for node metadata.
/// Currently holds one key: `CURSOR_KEY`.
pub(crate) const META_DB: &str = "cordial-meta";

/// Fixed key under which the finalized cursor is stored in `META_DB`.
pub(crate) const CURSOR_KEY: &[u8] = b"finalized_cursor";

// ── Shared struct ─────────────────────────────────────────────────────────
//
// Defined here so all three submodules can use it via `super::`.

/// LMDB-backed block storage.
///
/// Construct with [`open::RSpaceBlocklaceRepository::open`].
/// All public methods are implemented across the three submodules.
pub struct RSpaceBlocklaceRepository {
    pub(crate) env: Arc<Env>,
    pub(crate) blocks_db: Arc<Mutex<Database<Bytes, Bytes>>>,
    pub(crate) meta_db: Arc<Mutex<Database<Bytes, Bytes>>>,
}
