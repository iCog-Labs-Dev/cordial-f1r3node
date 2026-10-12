# PoR-weighted node integration plan

## Goal and current boundary

Derive PoR weights from finalized, replayable evidence and make the running
Cordial consensus instance use those weights for its **existing authorized
validator set**. PoR does not select a committee or replace Cordial consensus.
The weight activation point must be deterministic and recoverable after a
restart.

Today, `live_por_shadow` connects to an upstream f1r3node over gRPC, mirrors
blocks, and reports a durable genesis PoR projection. It does not produce live
ratings or advance beyond round 0. The four-node cluster uses upstream
f1r3node consensus; host-side Cordial and PoR observers can check convergence,
but they cannot make that node use PoR weights. See
[`por-shadow-runner.md`](por-shadow-runner.md) for the current smoke test and
its long-history memory limit.

## Integration decision: keep upstream source unchanged if possible

Prefer to keep the sibling `f1r3node` checkout unchanged. That is a goal, not
an established integration capability. The current
[`casper_adapter.rs`](../crates/cordial-f1r3node-adapter/src/casper_adapter.rs)
implements local mirrors of the upstream Casper traits, and the Docker node
binary does not select that adapter. An observer cannot change consensus
weights in another process through the existing block-query API.

**First gate:** trace the upstream node startup path that constructs and owns
Casper, its runtime manager, block store, and authorized bonds. Establish
whether an integration binary in this repository can construct the real node
with our adapter using public upstream crates and trait interfaces. Record the
exact call sites, missing methods, and a minimal compile or startup proof.

- If that host is viable, implement it here and leave upstream source alone.
- If no suitable construction seam exists, describe the smallest upstream
  feature-gated startup hook or adapter-selection change required. Do not
  assume a sidecar or a configuration flag enforces weights without a test
  showing the running consensus instance consumes them.

This gate decides the implementation location before work on node enforcement.
It does not block the earlier PoR lifecycle and adapter tests.

## Delivery slices

| Slice | Work and starting files | Exit evidence |
| --- | --- | --- |
| 0. Locate the node seam | Inspect upstream startup and real Casper traits; compare with `crates/cordial-f1r3node-adapter/src/casper_adapter.rs` and the real RSpace bridge in `crates/cordial-f1r3space-adapter/src/lib.rs`. | A written integration choice, named call sites, missing interfaces, and a buildable proof for the preferred host. |
| 1. Advance live PoR rounds | Connect finalized waves and evidence to `por/finality.rs`, `por/interactions.rs`, `por/ratings.rs`, `por/collector.rs`, and `por/runtime.rs`. Reuse the existing transport and durable state boundaries. | Four observers derive the same non-genesis round, weight map, and commitment from the same finalized history; restart preserves the result. |
| 2. Activate committed weights locally | Use `por/runtime.rs`, `por/persistence.rs`, and `LiveIngress::apply_por_weights` in `live_ingress.rs`. Define the finalized-wave activation boundary and handle pending or rejected activation. | A nonuniform weight change is durable, keeps the authorized identities unchanged, preserves the published finalized prefix, and recovers after restart. |
| 3. Connect weights to the running node | Build the host selected in slice 0 and wire its block production, validation, finality, fork choice, and ordering to the same active Cordial weight projection. | A node-facing test proves that a controlled weight change affects the running consensus path. Reading a shadow status file alone does not satisfy this gate. |
| 4. Four-node functional smoke | Extend the connected-cluster harness with a fresh, isolated chain and per-node activation observations. Keep the long-running cluster as a separate stability and disk baseline. | Four nodes agree on active round, exact weights, commitment, and finalized output through observer and validator restarts; a controlled nonuniform-weight scenario demonstrates enforcement. |
| 5. Sustained-run gate | Repeat with longer histories after the shadow ordering, restart, and disk-growth issues are measured and addressed. | Recovery time, peak RSS, per-poll latency, and disk-growth rates are recorded against agreed limits. |

Slices 1 and 2 can progress while slice 0 is investigated. Slice 3 depends on
the result of slice 0 and on a safe activation boundary from slice 2. A fresh
four-node functional test can proceed before the long-history performance work
is complete; sustained operation cannot be claimed until slice 5 passes.

## Safety and proof requirements

- Take validator identities from f1r3node's authorized bonds. Reject an
  activation whose weight keys add or omit an authorized validator.
- Accept only deterministic, audited PoR transitions backed by finalized
  evidence. Persist the state and activation record before reporting weights
  as active. Recover the same projection before computing new finality.
- Activate a new projection at a defined finalized boundary. All participants
  must derive the same commitment for that boundary; missing or conflicting
  inputs must not silently cause nodes to use different weights.
- Preserve the already-published finalized prefix when weights change. Test
  the activation rejection and retry path as well as the successful path.
- Distinguish three outcomes in tests and reports: observers agree on weights,
  a local Cordial ingress uses weights, and the running f1r3node consensus
  uses weights. Only the third demonstrates node enforcement.

## Test and change discipline

Extend the focused tests in
`crates/cordial-f1r3node-adapter/tests/test_por_runtime.rs`,
`test_por_weight_activation.rs`, and `test_por_finality.rs` as the lifecycle
changes. Use `docker/scripts/verify-four-node-por-shadow.py` for observer
agreement, then add a separate integration assertion for actual node weight
consumption. Keep implementation, tests, and documentation in separate,
reviewable commits. Do not stage or commit changes automatically.

The current 25-hour cluster is useful for measuring f1r3node stability and
storage, but the present shadow runner replays its full history and has already
shown a large memory peak. Use a fresh isolated Compose project and separate
PoR data root for the initial functional smoke, as described in
[`por-shadow-runner.md`](por-shadow-runner.md#isolated-fresh-cluster-reproduction).
