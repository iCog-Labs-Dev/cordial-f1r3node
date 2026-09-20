# 06 — Rust module boundary refactor

Issue #189. Reshapes `cordial-miners-core`'s interfaces so the contract the
Lean conformance checker validates is the contract the Rust code actually
exposes, rather than one inferred from outside.

No protocol logic changes. The arithmetic, the equivocation checks and the
ordering algorithm are untouched; what changes is which of the evidence they
already compute survives past the return statement.

## Why

Issues #183–#188 produced a working assurance loop: Lean proves the safety
properties, Rust emits a canonical trace, and CI replays that trace through an
executable model proved equivalent to the theorems. But the contract that loop
depends on lived only in the checker's expectations. Nothing in Rust's types
said "a finality decision carries a certificate" or "these three numbers
describe the same weight table". The checker inferred all of it, reconstructing
approximations from raw events — correct today, and one refactor away from
drifting silently.

## Status

| Gap | Area | Status |
|---|---|---|
| 1 | Finality/approval expose their evidence | **Done** |
| 2 | Weight/committee state captured as a snapshot | **Done** |
| 3 | `tau()` exposes a trace proof or prefix hash | **Deferred** — see below |
| 4 | Prefix-extension enforced at every emission point | **Deferred** — see below |

---

## Gap 1 — Certificate evidence

### Before

```rust
pub fn is_weighted_final_leader(...) -> bool
pub fn weighted_super_ratifies(...) -> bool
pub fn weighted_ratifies(...) -> bool
```

Each returned a conclusion. The evidence behind it — which validators
supported the block, with which blocks, for how much stake, against which
weight table — was assembled inside `#[cfg(feature = "trace")]` blocks purely
to populate a JSON event, then dropped. In a default build it was never
constructed at all.

Anyone wanting to check the decision had to re-derive the entire computation
from the blocklace and trust that their answer matched.

### After

```rust
pub fn weighted_final_leader_certificate(...)  -> Option<ThresholdCertificate>
pub fn weighted_super_ratifies_certificate(...) -> Option<ThresholdCertificate>
pub fn weighted_ratifies_certificate(...)       -> Option<ThresholdCertificate>
```

The predicates remain and delegate (`.is_some()`), so no caller changed. The
evidence is now assembled once, in one place, and **both** the returned value
and the trace event read from that object — the event is a serialization of
the evidence rather than a parallel reconstruction of it.

`ThresholdCertificate::verify_quorum` re-checks `3 * support > 2 * total` from
the carried numbers alone. It deliberately spells the rule out rather than
calling the consensus predicate, so it stays an independent check rather than
an echo of the code that produced the number.

### Field cross-reference against the 05a trace schema

`ThresholdCertificate` holds domain types; `ThresholdCertificateEvent`
(`05a`, `build_threshold_certificate`) holds their canonical encodings.
Serialization happens at the boundary, in `cordiality::emit_certificate`.

| `ThresholdCertificate` | Type | Trace field | Encoding |
|---|---|---|---|
| `kind` | `CertificateKind` | `kind` | `"ratification"` \| `"super_ratification"` |
| `leader` | `BlockIdentity` | `leader_hash` | lowercase hex of `content_hash` |
| `ratifier` | `Option<BlockIdentity>` | `ratifier_hash` | hex, `null` for super-ratification |
| `certificate_id` | `String` | `certificate_id` | carried verbatim |
| `approver_blocks` | `Vec<BlockIdentity>` | `approver_hashes` | hex each, canonical order |
| `approvers` | `Vec<NodeId>` | `approvers` | hex each, canonical order |
| `approver_count()` | `usize` (derived) | `approver_count` | `approvers.len()` |
| `approver_weight` | `u128` | `approver_weight` | numeric |
| `total_weight` | `u128` | `total_weight` | numeric |
| `weight_snapshot` | `WeightSnapshotId` | `weight_table_hash` | FNV-1a-64 hex |

**Two event fields are deliberately not on the certificate.** `node_id` is the
actor the event is attributed to, supplied by the emitter rather than by the
evidence; `wave` is always `null` at these call sites. A certificate describes
*what was decided and on what basis*, not *who recorded it*.

Lean's `Replay.checkCertificateEvent` already validates every field in the
right-hand column. Having Rust return the object makes it possible for replay
to validate the artifact the decision used, instead of an approximation
rebuilt from insert and approval events. That follow-up is not done here.

### Incidental corrections

* `compute_finality` events now carry the id of the certificate the decision
  actually rests on. They previously recomputed
  `certificate_id("super_ratification", block_hash, None)` — the same formula,
  derived a second time and assumed to agree.
* Witness ordering in super-ratification is no longer `cfg`-dependent. Default
  builds walked a randomized `HashSet` order while trace builds walked a sorted
  one. Both now sort. No semantic change — the result set is order-independent
  — but one fewer way for traced and untraced builds to differ.

---

## Gap 2 — Weight snapshot

### Before

```rust
bonds: &HashMap<NodeId, u64>
```

threaded through the decision path: a borrowed view of live, mutable state.
Three consequences:

1. A decision could not be re-verified later, because the table it was judged
   against may since have changed.
2. Within one decision the quorum check, the support total and the trace event
   each re-read the map independently; nothing guaranteed they saw the same
   thing.
3. A replay mismatch was ambiguous between a real divergence and a race
   between reading weights and deciding.

Two ad-hoc fingerprints of the same table already existed for different
purposes: `bonds_fingerprint` (a `DefaultHasher` value used as an ordering
cache key) and `trace::weight_table_hash` (the FNV-1a value in trace events).

### After

```rust
pub struct WeightSnapshotId(String);
pub struct WeightSnapshot {
    id: WeightSnapshotId,
    bonds: Arc<BTreeMap<NodeId, u64>>,
}
```

Public signatures still take `&HashMap<NodeId, u64>` and capture the snapshot
at the boundary, so no caller changed. Internally one captured table governs a
whole decision.

`NodeId` derives `Ord` over `Vec<u8>`, so `BTreeMap` iteration is the same
byte-lexicographic order `weight_table_hash` sorts into. The snapshot id is
therefore byte-identical to the trace fingerprint **by construction** —
asserted by `snapshot_id_matches_canonical_trace_fingerprint`. To keep that
true, `weight_table_hash` was split: it now sorts and delegates to
`weight_table_hash_sorted`, so both entry points run one fold rather than two
implementations that happen to agree.

`bonds_fingerprint` is gone; the ordering cache keys use `WeightSnapshotId`.
One notion of "which weight table" now spans consensus decisions, cache keys
and trace events, so the three cannot disagree about what they describe. The
removed helper was built on `DefaultHasher`, whose output is explicitly not
stable across Rust releases.

### Scope, and why

Applied to the **decision path** — `cordiality`, `finality`, `approval`,
`ordering`, `validation`, `pruning` — which is what acceptance criterion 2
asks for ("used at the relevant decision points"). `dissemination.rs` and the
adapter crate still take `&HashMap` and convert at the boundary; they are
transport and integration surfaces, not decision points, and migrating them
would have produced one large mechanical change with no checker-visible
benefit.

At the time of writing, weights do not yet change at runtime:
`live_ingress::set_bonds` has no callers and the `cordial-por` reputation crate
is not wired into any other crate. Both mechanisms are built and waiting, which
is precisely why the snapshot is cheap to introduce now and would be an
expensive retrofit later.

---

## Deferred, with reasons

Recorded here rather than left silent, per this issue's instruction.

### Gap 3 — `tau()` exposes a trace proof

`weighted_tau` still returns `Result<Vec<BlockIdentity>, OrderingError>`.
Checking append-only-ness therefore still means re-running τ on both blocklaces
and diffing.

**Deferred to coordinate with Issue #244** (Formal Proof of τ Ordering), which
will define `ValidAppend` and a proved `tauRef`. The shape a `TauOutput` should
carry is exactly what that theorem's hypotheses need; designing it first means
designing it twice. Note `trace::output_prefix_hash` already computes the
running prefix hash — the work is promoting it from a trace-only value into the
API, not inventing it.

### Gap 4 — Prefix-extension enforced in code

Partially present already, in the wrong layer:

| Path | Enforced? |
|---|---|
| `cordial-miners-core` — `tau`, `weighted_tau` | No; returns a bare `Vec` |
| `cordial-miners-core` — `pruning` checkpoint | No; prefixes are stored and replayed, never compared |
| adapter — `SharedOrderedOutput::update` | Yes; rejects with `PrefixViolation` |
| adapter — `live_ingress::latest_finalized_ordered_output` | Yes; propagates as `SnapshotError` |
| adapter — `live_ingress::ordered_finalized_blocks` | **No; bypasses the guard** |

So the invariant is enforced on one adapter export path, absent from core, and
bypassed by a sibling method on the same struct. Completing it means adding the
guard in core, extending it to the checkpoint path, and closing that bypass —
which wants the same `ValidAppend` shape as gap 3, so it is deferred with it.

The invariant is still only an axiom on the Lean side
(`Ordering.lean: tau_prefix_monotone`); #244 replaces that with a proof.

---

## Known boundary, unchanged by this issue

`node_id` on `compute_finality`, `run_tau_order` and `emit_output` is the
creator of the block being decided, not the node doing the deciding. The
canonical trace consequently describes a single global view, and replay
reconstructs one blocklace from all `insert_block` events regardless of which
node emitted them.

This issue does not change that. The certificate describes evidence for that
same single view. Attributing decisions to an evaluating node is a trace-schema
and replay-model change, not an interface-honesty one.

---

## Verification

Behaviour preservation is checked by Issue #188's own conformance gate: the
canonical fixtures are regenerated and byte-compared on every run.

```bash
just issue188-conformance
```

For this work specifically:

| Check | Result |
|---|---|
| `cargo test -p cordial-miners-core` | 0 failures |
| `cargo test -p cordial-miners-core --features trace` | 0 failures |
| `cargo clippy --all-targets --all-features -- -D warnings` | clean |
| `git diff --exit-code -- lean/traces` | **byte-identical** |

That last row is the argument. The certificate assembly and emission path was
restructured end to end and the canonical traces came out byte-for-byte
unchanged, so the evidence recorded before and after is provably the same
evidence.

New tests worth naming:

* `weight_snapshot::snapshot_id_matches_canonical_trace_fingerprint` — the
  snapshot id equals `trace::weight_table_hash` exactly.
* `certificate::quorum_verifies_from_the_certificate_alone` — constructs no
  blocklace; checks a finality decision from the carried fields. This is the
  capability gap 1 exists to create.
* `certificate::quorum_is_strict_at_the_exact_boundary` — six of nine equal
  validators gives `3 × 600 == 2 × 900`, which must fail.
