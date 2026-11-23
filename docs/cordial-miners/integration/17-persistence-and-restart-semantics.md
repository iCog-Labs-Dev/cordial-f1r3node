# 17 — Persistence and Restart Semantics

This document explains what `LiveIngress` persists to LMDB, what stays
in-memory only, and the exact startup sequence a caller uses to resume a
node's live mirror after a restart.

It closes the gap tracked in
[issue #178](https://github.com/iCog-Labs-Dev/cordial-f1r3node/issues/178):
`cordial-f1r3space-adapter`'s `RSpaceBlocklaceRepository` was implemented and
unit-tested in isolation, but `LiveIngress` — the live runtime path — only
ever constructed an in-memory `LiveBlocklaceMirror::default()`. Nothing
hydrated it from disk, so a process restart silently lost every mirrored
block and the finalized cursor.

---

## 1. What Survives Restart, What Doesn't

| State | Persisted? | Where |
|---|---|---|
| Raw blocks (block body + identity) | ✅ Yes | `cordial-blocks` LMDB database |
| Finalized cursor (latest anchor `BlockIdentity`) | ✅ Yes | `cordial-meta` LMDB database |
| Versioned checkpoint record (identity, origin, depth, both ordering prefixes) | ✅ When checkpoint persistence writes removed IDs | `cordial-meta`, key `checkpoint_record`, bincode version 1 |
| Pruned identities | ✅ With the checkpoint record | `cordial-meta`, key `pruned/` + bincode(identity), empty value |
| Native identity mappings and height cursor | ✅ Through the native persistence APIs | `cordial-meta` |
| In-memory `Blocklace` DAG structure | ❌ No — restored from checkpoint state and unmarked archive blocks, or the full archive | — |
| `OrderedFinalizedOutput` container | ❌ No — recomputed from restored blocklace state | — |
| `OrderingCache` entries | ❌ No — ephemeral, warm up after first ordering call | — |
| Core `Blocklace` — native `BlocklaceStore` decision | 📌 Deferred — LMDB stays at the adapter level for now | — |

The block archive is never deleted. Checkpoint metadata stores the logical
boundary and ordering identity prefixes established by core pruning. Recovery
uses this state to avoid loading marked block bodies; ordering caches and
ordered output are recomputed. Without a record, recovery replays the full
archive as before.

### Block-hash format compatibility

`cordial-meta` stores `block_hash_format_version = 3`. Version 3 binds native
block hashes to their creator via domain-separated `cordial-block-hash-v3` with
a length-prefixed creator `NodeId` ahead of the block content. Earlier version 2
stores hashed predecessor blocks by `(content_hash, creator)` but did not bind
the block's own creator into its native hash.

Opening a non-empty store without the version marker, or with an older version
(such as version 2), fails closed with `RepoError::IncompatibleStorage`. The
operator must remove and resynchronize that adapter-level blocklace store.
Recovery deliberately fails before replay instead of silently skipping legacy
blocks and exposing an incompatible or partial DAG. Empty stores are initialized
with the current marker (version 3), and current versioned stores reopen normally.

---

## 2. The Restart Loop

```text
ingest_and_persist  ──▶  (restart)  ──▶  with_persistent_store
```

Every live block goes through `LiveIngress::ingest_and_persist`, which
writes to LMDB **before** inserting into the in-memory mirror — mirroring
the write-order invariant `RSpaceBlocklaceRepository::put_block` already
documents: a crash between the disk write and the mirror insert is safe,
because recovery replays from disk on the next boot. A crash before the
LMDB write commits leaves nothing written, which is equally safe.

On restart, `LiveIngress::with_persistent_store` rebuilds a fresh mirror by:

1. Reading the cursor and checkpoint record. Corrupt or unknown-version records
   fail closed. With a record, the checkpoint block must exist in the archive;
   `Blocklace::restore_from_checkpoint` verifies its signature and restores its
   origin, depth and prefixes without requiring pruned predecessors.
2. Scanning `cordial-blocks`. With a record, the checkpoint and marked entries
   are skipped by key before block-body deserialization. Without a record,
   every entry follows the existing full replay path.
3. Topologically sorting surviving blocks and inserting them through the caller's
   verifier. Corrupt entries and rejected blocks are warned and skipped.
4. Rebuilding native mappings, excluding marked identities. An unmarked mapping
   to an unrecovered block still fails closed.
5. Returning the finalized cursor so the caller can log the resume point.

### Checkpoint writes and crash points

`checkpoint_prune_and_persist` prunes RAM first, then writes the record and all
queued removed identities in one transaction. Failed writes keep those IDs for
retry; no pending removals means no record write, even after a zero-removal
advance. Lower depth and a different checkpoint at equal depth are rejected;
rewriting the same checkpoint at equal depth is idempotent and adds markers.
A crash before commit restores the older record, or none, and loads more blocks.
A crash after commit restores the new boundary and markers together.
Late archived blocks that reference pruned predecessors are rejected by `insert`
during recovery, warned and skipped. Their archive entries remain intact.

## 3. Startup Lifecycle

```rust,ignore
let repo = open_block_store(&data_dir)?;
let (mut ingress, cursor) =
    LiveIngress::with_persistent_store(
        adapter, bonds, shard_conf, "root", &repo, &verifier,
    )?;

// cursor is the last known finalized block identity — log it, or use it
// to decide which ranges are already known-finalized.
tracing::info!(?cursor, "resuming live ingress");

// Start accepting gRPC blocks:
ingress.ingest_and_persist(block, &repo)?;

// After each ordered-output advance:
if ingress.latest_finalized_ordered_output(WAVELENGTH)?.anchor.is_some() {
    ingress.checkpoint_prune_and_persist(WAVELENGTH, &repo)?;
    ingress.persist_finalized_cursor(&repo)?;
}
```

The node must not accept new gRPC blocks until step 1 (`with_persistent_store`)
returns — the same rule `RSpaceBlocklaceRepository::recover_into_engine`
already documents for the lower-level recovery path.

## 4. Why the Verifier Differs From Live Ingestion

`with_persistent_store` replays disk-sourced blocks through the caller's
real `CryptoVerifier`, not `AlreadyValidatedVerifier`. Blocks on disk were
written by a prior process and haven't been re-checked since; recovery is
the one path where Cordial re-validates signatures before trusting old
state. Live ingestion through `ingest_and_persist` / `ingest_trusted_block`
continues to use the already-validated trust boundary, since those blocks
were checked at the gRPC/adapter boundary moments earlier.

## 5. Related Work

- `docs/cordial-miners/integration/03-live-blocklace-mirror.md` — the
  in-memory mirror this document adds persistence around.
- `docs/cordial-miners/integration/18-pruning-and-cache-invalidation-policy.md`
  — what happens to in-memory state *after* it's no longer needed, which is
  the natural follow-up to this document's "what's kept" question.
- Task 5 in the historical `INTEGRATION_NEXT_STEPS.md` task list — the open
  question about consolidating LMDB into a native `BlocklaceStore` decision,
  intentionally left deferred here (see the "Core `Blocklace`" row above).
