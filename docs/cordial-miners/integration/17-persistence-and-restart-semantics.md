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
| In-memory `Blocklace` DAG structure | ❌ No — reconstructed from LMDB blocks on boot | — |
| `OrderedFinalizedOutput` (full tau output) | ❌ No — recomputed from reconstructed blocklace | — |
| `OrderingCache` entries | ❌ No — ephemeral, warm up after first ordering call | — |
| Core `Blocklace` — native `BlocklaceStore` decision | 📌 Deferred — LMDB stays at the adapter level for now | — |

The design principle: LMDB stores the minimum durable facts (blocks and the
finalized cursor). Everything derived from those facts — the DAG structure,
ordering cache, and tau output — is cheap to recompute on boot and is never
itself written to disk. This keeps the write path narrow (two tables, two
methods) and avoids a second source of truth that could drift from the
blocklace's own consensus rules.

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

1. Reading every block out of `cordial-blocks`.
2. Topologically sorting them by computed DAG height (`recover_into_engine`,
   implemented in `cordial-f1r3space-adapter`), so predecessors are always
   replayed before their successors regardless of the order they were
   originally written in.
3. Replaying each block into the mirror's `Blocklace` through the supplied
   verifier. Corrupt or rejected entries are logged and skipped, never
   panicked on.
4. Returning the finalized cursor read from `cordial-meta`, if any, so the
   caller can log the resume point.

## 3. Startup Lifecycle

```rust,ignore
let repo = open_block_store(&data_dir)?;
let (mut ingress, cursor) =
    LiveIngress::with_persistent_store(adapter, &repo, &verifier)?;

// cursor is the last known finalized block identity — log it, or use it
// to decide which ranges are already known-finalized.
tracing::info!(?cursor, "resuming live ingress");

// Start accepting gRPC blocks:
ingress.ingest_and_persist(block, &repo)?;

// After each ordered-output advance:
if ingress.latest_finalized_ordered_output(WAVELENGTH)?.anchor.is_some() {
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
