# Ordered Output Consumer Boundary

This note defines the first node-facing consumer boundary for exported
finalized ordered output. It answers the question: once
`OrderedFinalizedOutput` is produced, who reads it and under what contract?

This is the natural next step after
[13-ordered-output-export.md](./13-ordered-output-export.md) and follows the
implementation sequence recommended in
[12-ordered-output-reintegration.md](./12-ordered-output-reintegration.md).

## Goal

Define one concrete downstream consumer boundary so that:

- the export seam has a clear destination
- the consumer contract is documented before implementation
- the design stays consistent with sidecar / observer-first integration
- the next implementation issue can be opened from this result

## Why a consumer boundary comes before proposer work

Steps 1 and 2 of the reintegration sequence are complete:

1. ~~define an adapter-side ordered-output export type~~ — `OrderedFinalizedOutput`
2. ~~expose the latest finalized ordered fragment from live mirrored state~~
   — `LiveIngress::latest_finalized_ordered_output()`

Step 3 is: **document or prototype one node-facing consumer of that ordered
output**.

A consumer boundary keeps the integration incremental. It proves the export
seam is usable before committing to deeper node-facing changes like proposer
replacement or execution reordering.

## Identified consumer boundary: shared ordered output reader

The first consumer boundary is an **adapter-side shared state container** that
holds the latest `OrderedFinalizedOutput` and exposes it through a read-only
trait.

This is the simplest consumer that still demonstrates the full
produce → export → consume flow without modifying `f1r3node`.

### Consumer trait

```rust
/// Read-only access to the latest finalized ordered output.
///
/// Implemented by adapter-side state containers. Consumed by binaries,
/// tests, and future node-facing components that need the ordered
/// finalized prefix without recomputing it.
pub trait ReadOrderedOutput {
    /// Return the latest computed ordered output, or `None` if no
    /// finalized leader has been reached yet.
    fn latest(&self) -> Option<&OrderedFinalizedOutput>;

    /// Return the latest output's anchor hash, or `None` if empty.
    fn anchor_hash(&self) -> Option<Vec<u8>> {
        self.latest().and_then(|o| o.anchor_hash())
    }

    /// Return `true` if the latest output is older than `max_age_ns`
    /// nanoseconds compared to the current wall clock.
    fn is_stale(&self, max_age_ns: u128) -> bool {
        self.latest()
            .map(|o| {
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_nanos();
                now.saturating_sub(o.computed_at_ns) > max_age_ns
            })
            .unwrap_or(true)
    }
}
```

The trait is intentionally minimal. Consumers get a snapshot of the latest
output and basic staleness checks. There is no push/notification in the first
increment — consumers poll.

### Adapter-side container

```rust
/// Adapter-side container that holds the latest finalized ordered output.
///
/// Produced by the adapter's live-ingress pipeline. Read by consumers
/// through the `ReadOrderedOutput` trait.
pub struct SharedOrderedOutput {
    latest: Option<OrderedFinalizedOutput>,
}
```

The adapter updates this container each time
`LiveIngress::latest_finalized_ordered_output()` is called. Consumers read it
without recomputing ordering or touching the blocklace.

### Consumer contract

The consumer boundary has these guarantees:

| Property | Contract |
|----------|----------|
| **Prefix preservation** | Each successive output either extends or matches the previous. Blocks are never reordered or removed. |
| **Determinism** | Given the same blocklace state and bonds, the output is identical across calls. |
| **Monotonic anchor** | The anchor either stays the same or advances. It never regresses. |
| **Thread safety** | The container is updated by the adapter and read by consumers. Access is single-writer / single-reader in the first increment. |
| **Staleness** | Consumers compare `computed_at_ns` against wall clock. The container does not auto-expire. |

The consumer boundary does **not** guarantee:

- that the output will be non-empty — `anchor` is `None` until a finalized
  leader exists
- that updates arrive at any particular frequency — output is recomputed on
  demand or on a schedule chosen by the adapter
- that `f1r3node` will act on the output — this is an observer-side read
  boundary only

## Where the container lives

The `SharedOrderedOutput` container belongs in the adapter crate:

```
crates/cordial-f1r3node-adapter/src/shared_ordered_output.rs
```

This keeps it alongside the existing export type (`ordered_output.rs`) and
the production site (`live_ingress.rs`).

## How it connects to the existing pipeline

```
live block traffic
       |
       v
LiveIngress::ingest_block_message()
       |
       v
LiveBlocklaceMirror  (local blocklace state)
       |
       v
LiveIngress::latest_finalized_ordered_output()
       |
       v
OrderedFinalizedOutput  (stable export type)
       |
       v
SharedOrderedOutput  (consumer container)
       |
       v
ReadOrderedOutput trait  (consumer boundary)
       |
       v
[binaries, tests, sidecar consumers]
```

No part of this pipeline modifies `f1r3node`. The entire flow is adapter-side.

## Consistency with existing sidecar patterns

This boundary follows the same pattern as the deploy proxy from
[11-external-grpc-deploy-proxy.md](./11-external-grpc-deploy-proxy.md):

| Aspect | Deploy proxy | Ordered output reader |
|--------|-------------|----------------------|
| Direction | Inbound (deploy ingress) | Outbound (ordered output) |
| Sidecar model | External to f1r3node | External to f1r3node |
| Node changes | None | None |
| Contract | Observe + forward `doDeploy` | Expose + read `OrderedFinalizedOutput` |
| First increment | Observation only | Read-only consumption |

Both seams keep the adapter as an external observer that reads or writes data
at well-defined boundaries without altering node internals.

## Suggested implementation

The implementation for this consumer boundary should:

1. add `SharedOrderedOutput` as an adapter-side container
2. implement `ReadOrderedOutput` for it
3. wire it into the live-ingress pipeline so it is updated when new output
   is computed
4. expose it through the `LiveIngress` wrapper or as a standalone component
5. add a basic integration test that:
   - ingests blocks through the mirror
   - computes ordered output
   - reads it through the consumer trait
   - verifies prefix preservation and anchor behavior

### Files to create or modify

| File | Action |
|------|--------|
| `crates/cordial-f1r3node-adapter/src/shared_ordered_output.rs` | Create — container + trait impl |
| `crates/cordial-f1r3node-adapter/src/lib.rs` | Modify — register module |
| `crates/cordial-f1r3node-adapter/src/live_ingress.rs` | Modify — optional: convenience method that returns a `&SharedOrderedOutput` |
| `crates/cordial-f1r3node-adapter/tests/` | Create — integration test for the consumer boundary |

## What this issue does not include

- **Transport wiring** — serving `OrderedFinalizedOutput` over gRPC or HTTP
  is a follow-up step after the in-process boundary is proven
- **Push / notification** — consumers poll in the first increment; streaming
  or callback delivery can come later
- **Node execution integration** — the consumer boundary is read-only; it
  does not cause `f1r3node` to re-execute or reorder anything
- **Proposer replacement** — this remains out of scope until the consumer
  boundary is validated

## Suggested next implementation issue

`Add an adapter-side shared ordered output reader as the first consumer boundary`

That issue should focus on:

- defining the `ReadOrderedOutput` trait
- implementing `SharedOrderedOutput` as the adapter-side container
- wiring the container into the live-ingress pipeline
- adding integration tests for prefix preservation and staleness

## Files

| File | Role |
|------|------|
| `crates/cordial-f1r3node-adapter/src/ordered_output.rs` | Export type (unchanged) |
| `crates/cordial-f1r3node-adapter/src/live_ingress.rs` | Production site (minor additions) |
| `docs/cordial-miners/integration/12-ordered-output-reintegration.md` | Reintegration plan |
| `docs/cordial-miners/integration/13-ordered-output-export.md` | Export seam documentation |

## See Also

- [12-ordered-output-reintegration.md](./12-ordered-output-reintegration.md) —
  the reintegration plan that recommends this approach
- [13-ordered-output-export.md](./13-ordered-output-export.md) — the export
  seam this consumer boundary reads from
- [11-external-grpc-deploy-proxy.md](./11-external-grpc-deploy-proxy.md) —
  the analogous sidecar pattern on the deploy ingress side
