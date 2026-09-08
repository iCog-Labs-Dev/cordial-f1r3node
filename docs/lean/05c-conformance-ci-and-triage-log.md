# 05c — Conformance CI and triage log

## Reproducible pipeline

Status: the executable replay gate exists; full Issue #188 acceptance remains
blocked on the KR4 ordering refinement proof. A green run checks independent
ordering comparison, not a theorem that `computeTau` implements opaque `tau`.

Run the complete local gate from the repository root:

```bash
just issue188-conformance
```

The workspace also needs a sibling `../f1r3node` checkout, including for
`cargo -p cordial-miners-core` (Cargo resolves all workspace manifests).
CI checks out this repository into `blocklace/` and upstream
`F1R3FLY-io/f1r3node` into `f1r3node/`, pinned to
`3507ce3daab1946cdda2050a6dbeedd0f0898575` (`rust-v0.4.15`). It checks the
layout with `cargo metadata --no-deps` before building. Lean action paths,
cache paths, and shell working directories use this same layout.

The gate executes these stages sequentially:

```bash
cargo build -j 2 -p cordial-miners-core
cargo test -j 2 -p cordial-miners-core
cargo test -j 2 -p cordial-miners-core --features trace
cargo test -j 2 -p cordial-miners-core --features trace \
  --test generate_trace_fixtures generate_all_fixtures -- \
  --exact --nocapture --test-threads=1

cd lean
lake build replay_runner conformance_tests
lake exe replay_runner
lake exe conformance_tests
cd ..
bash scripts/issue188_mutation_test.sh
```

Cargo is limited to two jobs and the three fixture scenarios share one test
thread because they switch `CORDIAL_TRACE_FILE`. Lake 5 has no `build -j`
flag; the package limits each Lean invocation using
`moreLeanArgs = ["-j", "2"]` in `lean/lakefile.toml`. Check available memory
before running the Rust and Lean builds together with other workloads.

## Trace generation and determinism

`generate_all_fixtures` truncates each destination, executes the actual Rust
consensus functions, and writes `lean/traces/{normal,equivocation,low_stake}.json`
plus their weight/leader sidecars. It immediately performs the same executions
a second time and compares all six files byte-for-byte. Set-valued fields are
sorted before serialization; ids and prefix hashes are deterministic; no wall
clock value is part of the semantic trace.

Both canonical and mutation generators enable `CORDIAL_TRACE_STRICT=1`.
Serialization/open/write/flush failures abort capture immediately; event-count
checks are not relied on to detect a partially written trace. Production
best-effort behavior is unchanged when strict mode is not enabled.

CI then runs `git diff --exit-code -- lean/traces`. A consensus or schema
change must therefore either preserve the canonical execution exactly or be
accompanied by a deliberately reviewed fixture/model update.

## What conformance means

A fixture is conformant only if:

1. every non-empty line parses as one known, fully typed event;
2. the external sidecar is canonical and its recomputed hash equals every
   weighted trace reference;
3. insertions construct a proof-valid predecessor-closed formal Blocklace;
4. the CMRef independently validates every reported equivocation, approval,
   certificate, finality decision, tau order, and emitted output item; and
5. scenario-specific non-vacuity requirements are met.

The positive runner prints counts such as:

```text
[normal] CONFORMANT ✓
  events: 48
  finality checks: 1
  equivocation checks: 0
  tau checks: 1
  output checks: 1
```

The expected outcomes are `CONFORMANT` for normal, equivocation, and
low-stake. Low-stake is a positive conformance fixture whose expected weighted
decision is `not_finalized`; accepting a trace does not mean every candidate
was finalized.

## Negative and mutation gates

`lake exe conformance_tests` checks all 15 parser variants and rejects:

- malformed JSON, unknown events, omitted required nullable fields, wrong
  types, and unknown fields;
- invalid predecessor closure;
- an incorrect finality decision;
- insufficient weighted quorum;
- an invalid certificate id/evidence;
- duplicate certificate approvers or evidence blocks;
- a false equivocation report;
- an incorrect tau order;
- an incorrect output-prefix hash; and
- a wrong weight-table hash.

The true mutation gate is separate. Cargo feature
`trace-threshold-mutation` compiles the Rust quorum predicate as
`2 * support > total`, executes the real approval/certificate/finality path
with four of seven equally weighted validators, and writes the trace to an
ephemeral directory. Unmodified Lean must reject it. A successful mutation
gate looks like:

```text
[mutation/weakened-threshold] actual Rust execution wrote /tmp/.../weakened_threshold.json
[weakened_threshold] MISMATCH ✗
  event 20 (build_threshold_certificate): insufficient quorum: support=400, total=700, required 3*support > 2*total
[mutation/weakened-threshold] rejected by independent Lean quorum ✓
```

The mutation is opt-in, test-only, and never changes committed fixtures. The
shell command fails if Rust does not produce the bad certificate, if Lean
accepts it, or if rejection occurs for an unrelated reason.

## CI behavior

`.github/workflows/lean.yml` runs on relevant pushes and pull requests. It:

0. checks the pinned sibling dependency layout and all 117 named KR theorem/
   lemma mapping rows, including exact Rust source and test links;

1. builds and tests the default, non-tracing core;
2. tests the trace-enabled core, including runtime instrumentation coverage;
3. regenerates and byte-compares canonical fixtures;
4. builds the Lean library and both executables;
5. runs all positive replays;
6. runs parser/semantic negative tests;
7. runs the actual weakened-Rust-threshold gate; and
8. fails if the Lean build reports `declaration uses 'sorry'`.

This makes conformance fail for an unmirrored schema change, nondeterministic
trace, invalid positive execution, accepted negative, accepted Rust mutation,
or a new proof hole. Default builds do not execute tracing work: without
feature `trace`, `trace::emit` is an inline no-op and event construction sites
are `cfg`-gated.

## First-mismatch triage

Parsing errors identify the one-based line. Semantic errors identify the
one-based event and canonical event kind. Replay stops at that first mismatch.

| Diagnostic | Initial classification and inspection point |
|---|---|
| `line N` JSON/schema error | trace/instrumentation bug: compare Rust `trace.rs` with Lean `Trace.lean` |
| missing/duplicate predecessor or wrong round | Rust insertion/instrumentation order versus `ReplayDag` |
| weight-table hash mismatch | configuration/adapter bug: sorted table and FNV encoding |
| approval/equivocation rejection | Rust KR2 behavior versus `CMRef` and formal `Equivocation`/`Approves` |
| certificate mismatch | evidence membership/deduplication, `WCert`, stake table, or certificate id |
| finality mismatch | approval → ratification → super-ratification evidence and leader/wave configuration |
| tau mismatch | latest finalized leader, prior-leader recursion, approval filter, or hash tie-break |
| output mismatch | tau result, output index, or FNV prefix encoding |

Classify a discrepancy as one of: Rust implementation bug, Lean model/proof
bug, trace/schema adapter bug, or an explicitly documented semantic boundary.
Never fix it by accepting both results, skipping a line, weakening a threshold,
or deriving the expected answer from the trace's result field.

## Safe fixture update procedure

1. Explain the intended protocol/schema change and identify its formal model.
2. Update Rust instrumentation and Lean typed schema together.
3. Update or prove the relevant executable-to-Prop bridge before replay logic.
4. Run the generator twice (the test does this automatically).
5. Inspect semantic and weight-sidecar diffs, not only JSON validity.
6. Run positive, negative, mutation, and no-`sorry` gates.
7. Record any genuine discrepancy below before accepting fixture changes.

## Running triage log

| Date | Observation | Classification | Resolution |
|---|---|---|---|
| 2026-09-07 | `computeTau` was called a refinement despite no theorem relating it to opaque `tau`; the formal signature omits canonical tie-break keys and the replay horizon. | Lean-spec problem | **OPEN / PR blocker.** Removed unsupported refinement/proof-sketch claims. Complete the KR4 ordering specification and prove executable correspondence; no replacement axiom added. |
| 2026-09-07 | Conformance CI checked out only this repository although Cargo resolves sibling `f1r3node` workspace path dependencies. | CI setup bug | Added a pinned sibling checkout and corrected all Lean/cache/working-directory paths. Hosted execution still requires a real CI run. |
| 2026-09-07 | Runtime trace emission silently discarded file-open/write errors. | trace instrumentation bug | Added fallible `try_emit`, opt-in strict emission, mandatory strict fixture capture, and subprocess tests for open and write failures. |
| 2026-09-07 | Mapping grouped declarations, referenced wildcard tests and nonexistent negative labels, and misidentified an output emission boundary. | documentation/test coverage bug | Added one row per named KR theorem/lemma, exact checked links, explicit proof-only coverage, correct negative labels, and actual duplicate-member/evidence negative tests. |
| 2026-09-07 | Set-valued parent/evidence fields could follow randomized `HashSet` traversal. | trace determinism bug | Centralized sorted hash/member encoding and made fixture generation compare two complete executions byte-for-byte. |
| 2026-09-07 | The former string-search parser could collapse `null` and missing numeric fields to defaults. | trace/schema adapter bug | Replaced it with Lean's JSON parser, strict field/type checks, and `Option Nat`; added malformed/unknown/missing/wrong-type/extra-field tests. |
| 2026-09-07 | A proposed mutation check only flipped a parsed finality result, so it did not prove CI caught broken Rust code. | test-oracle bug | Added opt-in compilation of the actual weaker Rust predicate and an ephemeral execution harness; Lean rejects its first bad certificate at support 400/700. |
| 2026-09-07 | Replaying an opaque Rust digest by assigning an arbitrary formal id would drop KR1's `Block.id_eq` premise. | Lean adapter/model bug | Restored `id_eq`, retained executable injective `hashContent`, and construct each replay block with `id := hashContent creator content`; Rust digest association stays separate and checked. |
| 2026-09-07 | The trace intentionally omits payload bytes and signatures. | documented boundary | Lean models payload with an injective opaque tag and verifies all DAG/consensus semantics. Rust remains responsible for cryptographic digest/signature validation; documentation does not claim otherwise. |

The GitHub workflow contains the mutation gate, but no throwaway remote PR is
created by this repository command. A hosted branch-protection demonstration
is an external release/operations step, not part of local replay execution.
