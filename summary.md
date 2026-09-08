# Issue #188: Rust-to-Lean trace conformance

## What was implemented

Issue #188 now provides an end-to-end conformance system between the Rust
consensus implementation and the Lean formal model:

```text
real Rust consensus execution
            |
            v
canonical NDJSON trace + validator weights
            |
            v
strict typed Lean parser
            |
            v
reconstructed formal Blocklace
            |
            v
executable KR1/KR2/KR3/KR4 reference predicates
            |
            v
Lean result compared with Rust's reported result
```

The important property is that Lean does not accept a result merely because it
is syntactically valid. For example, `"finalized"` is only Rust's claim. Lean
independently reconstructs the DAG, approvals, weighted certificates, leader
and wave, evaluates the formal `FinalLeader` predicate, and rejects the trace
when the two results disagree.

The implementation includes:

- typed Rust and Lean representations for all 15 canonical trace events;
- feature-gated instrumentation at real Rust execution points;
- deterministic trace, weight-table, certificate, and output-prefix encoding;
- strict JSON parsing with errors for malformed, unknown, missing, mistyped,
  or extra fields;
- correct preservation of nullable fields as `Option` values;
- formal Blocklace reconstruction with predecessor and round validation;
- independent equivocation, approval, quorum, ratification,
  super-ratification, and finality checks;
- independent tau ordering and output-prefix checks;
- three deterministic positive scenarios, parser/semantic negative tests, and
  a genuine compiled Rust threshold mutation test;
- CI that runs the complete Rust-to-trace-to-Lean pipeline.

## Why it was implemented this way

The earlier replay code was primarily a structural trace validator. A finality
event passed when its decision was either `"finalized"` or
`"not_finalized"`, so the checker could not detect an incorrect Rust consensus
decision.

That is insufficient for formal conformance. Rust and Lean need independent
oracles:

- Rust executes the production consensus algorithm and reports what happened.
- Lean derives what should have happened from the formal KR models and trace
  evidence.
- Replay passes only when both answers are equal.

This separation detects implementation regressions such as weakening the
weighted threshold, accepting a false equivocation, using a missing parent,
building an invalid certificate, or producing the wrong ordering.

## Technical design

### Canonical Rust trace

[`trace.rs`](crates/cordial-miners-core/src/trace.rs) defines the complete event
schema:

1. `CreateBlock`
2. `ValidateBlock`
3. `InsertBlock`
4. `BufferBlock`
5. `ResolveMissingParent`
6. `DetectEquivocation`
7. `AcceptApproval`
8. `BuildThresholdCertificate`
9. `ComputeFinality`
10. `RunTauOrder`
11. `EmitOutput`
12. `SendPackage`
13. `DeliverPackage`
14. `SchedulerTick`
15. `RunWaveTask`

Instrumentation was placed in the corresponding blocklace, validation,
approval, cordiality, finality, ordering, networking, dissemination, and
scheduler code paths. These events are not synthesized to make the canonical
fixtures pass. Synthetic values are used only by the schema round-trip/parser
coverage test.

Tracing is controlled by the Rust `trace` feature. Expensive event construction
is behind `#[cfg(feature = "trace")]`, so normal builds retain their previous
behavior and do not perform meaningful trace work.

Set-valued fields are sorted before serialization. Scheduler events use a
logical tick rather than wall-clock time. Validator tables, certificates, and
output prefixes use documented FNV-1a-64 encodings implemented independently
in Rust and Lean. Generating the same scenario twice must therefore produce
byte-identical files.

### Validator weights

Each scenario has a `.weights.json` sidecar containing:

- the validator IDs and actual weights;
- wavelength and leader selection data;
- the expected canonical weight-table hash.

Lean sorts and validates this configuration, recomputes its hash, compares it
with every weighted trace event, and uses the actual weights for the strict
quorum calculation:

```text
3 * support > 2 * total
```

The low-stake fixture deliberately has enough validators by count but only
`1004 / 3004` stake. Rust reports `not_finalized`, and Lean independently
reaches the same result.

### Strict Lean schema parser

[`Trace.lean`](lean/LeanVerification/Trace.lean) uses Lean's JSON parser and a
separate typed payload structure for every Rust event. It rejects:

- malformed JSON;
- unknown event variants;
- missing required fields, including required nullable fields;
- incorrect JSON types;
- unexpected fields.

JSON `null` remains `None`; it is never silently converted into zero or an
empty string. NDJSON parse failures report the first one-based line number.

### Formal Blocklace replay

[`Replay.lean`](lean/LeanVerification/Replay.lean) maintains a real formal
`Blocklace` together with its `ValidBlocklace` proof. On every insertion it:

1. resolves all Rust parent hashes to formal block IDs;
2. rejects missing or repeated predecessors and duplicate hashes;
3. inserts through the KR1 `ValidBlocklace.insert` theorem;
4. independently derives and checks the block round and optional wave;
5. retains the new Blocklace and proof for later consensus checks.

The trace intentionally omits application payload and signature bytes. Replay
uses a compact injective insertion tag as the formal payload, then constructs
the formal id exactly as `hashContent creator content` and supplies the
original KR1 `Block.id_eq` proof by reflexivity. A separate checked
`ReplayedBlock` association maps that formal id to the concrete Rust digest.
This keeps the formal DAG executable without weakening KR1 or pretending that
Lean can reproduce a cryptographic digest from omitted data. Duplicate
external hashes and formal-id collisions are rejected.

Buffer events carry the complete missing-parent set. Resolution events must
refer to a parent that was genuinely missing, and replay fails if a buffered
block is inserted too early or remains unresolved at end of trace.

### Executable formal reference model

[`CMRef.lean`](lean/LeanVerification/CMRef.lean) provides finite executable
predicates backed by the earlier KR definitions:

- `checkObserves` for formal observation;
- `checkEquivocation` for KR2 same-creator, same-round, incomparable blocks;
- `checkApproves` for KR2 approval/exclusion;
- `checkStrictTwoThirds` for weighted quorum;
- `checkRatifies` and `checkSuperRatifies` for certificate evidence;
- `checkFinal` for KR4 `FinalLeader`;
- `computeTau` for the executable ordering reference model.

Certificate membership is accumulated with the earlier KR3 `WCert.accept`
operation. `buildWCert_invariant` proves that its cached weight is exactly the
weight of its deduplicated accepted set.

The Boolean functions are connected to their Prop-level definitions by proved
equivalence theorems. The central result is:

```lean
checkFinal bonds validators B hV wave wavelength sel candidate = true
  ↔ FinalLeader bonds validators B hV wave wavelength sel candidate
```

This theorem and its supporting equivalences contain no `sorry` and introduce
no replacement axiom.

The earlier abstract KR4 `tau_prefix_monotone` axiom remains documented as an
existing trust boundary. Trace acceptance does not use that axiom: it computes
the exact order with `CMRef.computeTau` and compares the complete result with
Rust.

### Approval and certificate evidence

`AcceptApproval` records the approver block, target, creator, round, and wave.
Replay verifies that the block belongs to the claimed creator and evaluates
the formal approval predicate.

For a ratification certificate, replay additionally verifies:

- every evidence block was previously recorded as an accepted approval;
- the ratifier observes every claimed evidence block;
- every evidence block formally approves the target;
- the unique creators and their weights match the certificate fields;
- support is strictly greater than two thirds of total stake;
- the certificate ID matches the canonical encoding;
- `CMRef.checkRatifies` accepts the certificate.

A super-ratification certificate must reference previously checked
ratification certificates, and its exact witness set must satisfy
`CMRef.checkSuperRatifies`. A finalized event must reference a matching checked
super-ratification certificate, but the final decision is still recomputed
directly with `CMRef.checkFinal`.

### Equivocation, ordering, and output

Equivocation replay checks that reported blocks are distinct, exist, belong to
the same claimed creator and round, and satisfy the formal KR2 incomparability
predicate. Different creators, different rounds, repeated hashes, and
comparable blocks are rejected.

For `RunTauOrder`, Lean selects the latest formally finalized leader, follows
earlier ratified leaders, determines approved blocks, and performs a canonical
topological sort using Rust hashes only as deterministic tie-break keys. It
compares the leader, ordered hashes, and length with Rust.

For `EmitOutput`, replay checks contiguous indexes, the exact block at that
position in Lean's order, and the independently recomputed running prefix hash.
The trace must finish with the entire tau order emitted.

## Test scenarios

The fixtures live in [`lean/traces`](lean/traces):

- `normal` exercises insertion, approvals, certificates, finality, tau, and
  output;
- `equivocation` proves a real equivocation is detected while finality remains
  safe;
- `low_stake` distinguishes stake-weighted quorum from validator count.

[`conformance_tests.lean`](lean/conformance_tests.lean) mutates parsed real
traces and requires Lean to reject:

- an incorrect finality decision;
- insufficient quorum;
- an invalid certificate;
- false equivocation;
- a missing predecessor;
- an incorrect tau order;
- an incorrect output prefix;
- a wrong weight table; and
- a false finalized claim over the low-stake execution.

[`scripts/issue188_mutation_test.sh`](scripts/issue188_mutation_test.sh) is the
independent mutation demonstration. It compiles the actual Rust quorum
predicate as `2 * support > total`, executes the real consensus path with four
of seven equally weighted validators, writes an ephemeral trace, and requires
the unchanged Lean CMRef to reject the bad certificate.

## See it in action

All commands below run from the repository root. Builds are limited to two
jobs, and expensive suites should be run sequentially.

First inspect available memory:

```bash
free -h
```

Run the complete reproducible pipeline:

```bash
just issue188-conformance
```

To run the stages separately, generate and verify the deterministic Rust
traces:

```bash
cargo test -j 2 -p cordial-miners-core --features trace \
  --test generate_trace_fixtures generate_all_fixtures -- \
  --exact --nocapture --test-threads=1
```

Inspect a canonical execution directly:

```bash
sed -n '1,12p' lean/traces/normal.json
sed -n '1,80p' lean/traces/normal.weights.json
```

Build and replay the positive scenarios:

```bash
cd lean
lake build replay_runner conformance_tests
lake exe replay_runner
cd ..
```

Expected output includes:

```text
[normal] CONFORMANT ✓
  events: 48
  finality checks: 1
  equivocation checks: 0
  tau checks: 1
  output checks: 1
```

Run parser and semantic negative tests:

```bash
cd lean
lake exe conformance_tests
cd ..
```

Expected output reports every malformed or deliberately broken case as
`rejected ✓`.

Run the actual Rust threshold mutation:

```bash
bash scripts/issue188_mutation_test.sh
```

Expected output includes a Lean rejection of event 20 with
`support=400, total=700` and `required 3*support > 2*total`.

Finally, verify that the formal build contains no `sorry` declarations:

```bash
just lean-check-sorry
```

CI performs the same sequence in [`.github/workflows/lean.yml`](.github/workflows/lean.yml),
including default Rust tests, trace-feature tests, deterministic fixture
regeneration, Lean replay, negative tests, the compiled Rust threshold
mutation, and the no-`sorry` check.

## Verification status

The completed local verification used two Cargo jobs and ran heavy stages
sequentially. The following passed:

```text
cargo build -j 2 --workspace
cargo test -j 2 --workspace
cargo test -j 2 --workspace --features trace
cargo clippy -j 2 -p cordial-miners-core --all-targets --all-features --no-deps -- -D warnings
cargo test -j 2 -p cordial-miners-core --features trace --test generate_trace_fixtures generate_all_fixtures -- --exact --nocapture --test-threads=1
lake build replay_runner conformance_tests
lake exe replay_runner
lake exe conformance_tests
bash scripts/issue188_mutation_test.sh
```

The Lean source/build scan found no `sorry` or `admit`. The only explicit
axiom in the verification tree is the pre-existing, documented KR4
`tau_prefix_monotone` trust boundary; replay does not use it to accept an
ordering. The hosted GitHub Actions run still occurs only after the changes
are committed/pushed, which this implementation step intentionally did not
do.

## Further documentation

- [`05a-trace-schema-and-instrumentation.md`](docs/lean/05a-trace-schema-and-instrumentation.md)
  describes every field and runtime emission site.
- [`05b-proof-to-test-mapping.md`](docs/lean/05b-proof-to-test-mapping.md)
  maps Rust behavior to its trace event, Lean predicate, theorem, and test.
- [`05c-conformance-ci-and-triage-log.md`](docs/lean/05c-conformance-ci-and-triage-log.md)
  documents CI, diagnostics, and mismatch triage.
