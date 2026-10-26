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

/// Named database for the finalized cursor and native identity mappings.
pub(crate) const META_DB: &str = "cordial-meta";

/// Fixed key under which the finalized cursor is stored in `META_DB`.
pub(crate) const CURSOR_KEY: &[u8] = b"finalized_cursor";

/// Last fully ingested native block height. A missing key starts at height zero.
pub(crate) const NATIVE_HEIGHT_CURSOR_KEY: &[u8] = b"native_height_cursor";

/// Prefixes for the two directions of the native wire-hash mapping in `META_DB`.
pub(crate) const NATIVE_HASH_PREFIX: &[u8] = b"native_hash/";
pub(crate) const CORDIAL_ID_PREFIX: &[u8] = b"cordial_id/";

/// Format marker for hashes stored in `cordial-blocks`.
///
/// Version 1/unversioned data included predecessor signature bytes in a
/// descendant's content hash. Version 2 uses signature-independent
/// predecessor identities. The formats cannot be mixed because previously
/// persisted signatures authenticate the version-1 hash. Version 3 binds the
/// block's creator into the hash (`BLOCK_HASH_DOMAIN`); v2 stores must be
/// resynchronized.
pub const BLOCK_HASH_FORMAT_VERSION: u32 = 3;

/// Metadata key containing [`BLOCK_HASH_FORMAT_VERSION`] as big-endian bytes.
pub const BLOCK_HASH_FORMAT_VERSION_KEY: &[u8] = b"block_hash_format_version";

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
