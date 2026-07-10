# Ordered Output Export Seam

This note documents the stable export seam for finalized ordered output
produced by the Cordial adapter. It is the concrete realization of the
recommendation in
[12-ordered-output-reintegration.md](./12-ordered-output-reintegration.md).

## Purpose

The adapter can already compute finalized ordered output from the live
mirrored blocklace. This note explains how that output is represented,
exposed, and intended to be consumed.

This seam is the bridge from observer mode toward node-facing behavior.
It exists so that downstream consumers have a single, self-describing
type to read rather than recomputing ordering or interpreting bare
hash lists.

## What Is Exported

The export type is `OrderedFinalizedOutput`, defined in:

- `crates/cordial-f1r3node-adapter/src/ordered_output.rs`

It is a `Serialize`/`Deserialize` struct carrying these fields:

| Field                   | Type                      | Purpose                                              |
|-------------------------|---------------------------|------------------------------------------------------|
| `blocks`                | `Vec<BlockIdentity>`      | Ordered finalized block sequence (tau order)         |
| `anchor`                | `Option<BlockIdentity>`   | Latest weighted final leader anchoring this fragment |
| `wavelength`            | `u64`                     | Consensus wave size used for finality                |
| `bond_count`            | `usize`                   | Number of bonded validators at computation time      |
| `total_mirrored_blocks` | `usize`                   | Total blocks in the mirror (not just finalized)      |
| `computed_at_ns`        | `u128`                    | Wall-clock timestamp for staleness inspection        |

Every entry in `blocks` is a full `BlockIdentity` (content hash,
creator, signature), not a bare hash. Consumers have everything
needed without extra lookups into the blocklace.

### Ordering invariant

`blocks` is in deterministic topological order (weighted tau order):
predecessor-first, tie-broken by `BlockIdentity`'s natural ordering.
Every block in the list is finalized according to the current bonded
validator set and consensus parameters.

### Relation to `CasperSnapshot`

`CasperSnapshot` carries `ordered_finalized_blocks: Vec<Vec<u8>>`
as one field among many. That type is tied to f1r3node's snapshot
shape and is not a stable export seam. `OrderedFinalizedOutput` is
the adapter-side replacement: self-describing, includes full block
identities and consensus metadata, and decoupled from f1r3node's
snapshot layout.

## Where It Is Produced

The method that produces this output is:

```rust
LiveIngress::latest_finalized_ordered_output(wave_length: u64)
    -> Result<OrderedFinalizedOutput, SnapshotError>
```

Located at `crates/cordial-f1r3node-adapter/src/live_ingress.rs:411`.

This method:

1. Finds the latest finalized block ID from the mirrored blocklace
2. Computes weighted tau ordering over the finalized prefix
3. Resolves each bare content hash back to its full `BlockIdentity`
   from the mirror
4. Assembles `OrderedFinalizedOutput` with consensus metadata

When no finalized leader exists yet, `anchor` is `None` and
`blocks` is empty.

### Helper methods on `OrderedFinalizedOutput`

- `block_hashes()` — bare content hashes as `Vec<Vec<u8>>`
- `len()` / `is_empty()` — block count queries
- `anchor_hash()` — anchor content hash or `None`
- `computed_at()` — interprets `computed_at_ns` as `SystemTime`
- `with_timestamp(ns)` — override timestamp (for tests/replay)

## How Tooling Reads It

### Live mirror check harness

The harness at `crates/cordial-f1r3node-adapter/src/bin/live_mirror_check.rs`
already exercises this seam. Relevant flags:

- `--write-ordered-file <path>`
  - Serializes the current `OrderedFinalizedOutput` (or its hash list)
    to a JSON file for baseline capture
- `--compare-ordered-file <path>`
  - Compares a new run against a previously saved baseline
  - Reports `MATCH`, `MISMATCH`, or prefix/divergence relationship

See [08-live-mirror-check-harness.md](./08-live-mirror-check-harness.md)
for full parameter and output documentation.

### Programmatic access

Any adapter-side code holding a `&mut LiveIngress` can call:

```rust
let output = ingress.latest_finalized_ordered_output(wave_length)?;
// output.blocks contains the ordered finalized BlockIdentity sequence
// output.anchor identifies the final leader
// output.computed_at_ns enables staleness checks
```

The `OrderedFinalizedOutput` type implements `Serialize`, so it
can be persisted, sent over IPC, or logged directly.

## What Is Out Of Scope

This note documents the export seam only. The following are **not**
part of this seam:

- **Node-facing consumers** — how `f1r3node` components read and
  act on this output (future reintegration work)
- **Proposer influence** — using ordered output to affect block
  construction or deploy selection
- **Transport wiring** — how `OrderedFinalizedOutput` reaches
  consumers over gRPC or other channels

These belong to later integration steps after the export seam is
stable and validated.

## Connection To The Reintegration Plan

[12-ordered-output-reintegration.md](./12-ordered-output-reintegration.md)
identified three candidate reintegration seams and recommended
starting with **post-finality ordered-output export**.

This note documents that export seam. The recommended sequence
from doc 12 was:

1. define an adapter-side ordered-output export type
2. expose the latest finalized ordered fragment from live mirrored state
3. document or prototype one node-facing consumer of that ordered output
4. only after that, consider whether proposer-side integration is needed

Steps 1 and 2 are complete: `OrderedFinalizedOutput` is defined and
`LiveIngress::latest_finalized_ordered_output()` exposes it. This
note fulfills the documentation requirement. Step 3 (a consumer
prototype) is the natural next step, tracked separately.

## Files

| File | Role |
|------|------|
| `crates/cordial-f1r3node-adapter/src/ordered_output.rs` | Export type definition |
| `crates/cordial-f1r3node-adapter/src/live_ingress.rs` | Production via `latest_finalized_ordered_output()` |
| `crates/cordial-f1r3node-adapter/src/bin/live_mirror_check.rs` | Harness that exercises the seam |
| `docs/cordial-miners/integration/12-ordered-output-reintegration.md` | Reintegration plan that recommends this seam |
| `docs/cordial-miners/integration/08-live-mirror-check-harness.md` | Harness parameter and output docs |
