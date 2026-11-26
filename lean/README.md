# Lean Verification of Cordial Miners + PoR

This folder is the home of the Lean 4 formal-verification effort for the Cordial
Miners + Proof-of-Reputation consensus protocol implemented in
`crates/cordial-miners-core/`.

**Scope:** We are proving a Lean model of the protocol safe and
checking that this Rust node's real behavior conforms to that proven model —
via trace replay, not by formally verifying the Rust source itself. We are
**not** generating Rholang or MeTTa-IL code from these proofs. That is a
separate, later, and currently deferred initiative , and is
explicitly out of scope for every issue listed here.

## Why this exists

`cordial-miners-core` has unit tests, and they pass. But tests only tell you
the system behaved correctly on the scenarios someone thought to write — they
cannot rule out a bug that only shows up in a rare weight distribution or an
unusual equivocation pattern. In a consensus protocol, that gap can mean two
honest nodes finalize two different, conflicting transaction histories. We are
closing that gap with machine-checked proofs, **without standing up a second,
independent implementation of the protocol.** `cordial-miners-core` remains
the main implementation substrate throughout.

## The agreed plan

1. Instrument `cordial-miners-core` to emit canonical traces — block
   creation, validation, insertion, buffering, equivocation detection,
   finality, ordering, dissemination, and scheduler events. (Issue 05)
2. Build the Lean model/checker around the protocol subset we validate
   first: weighted certificates, blocklace validity, equivocation, finality,
   ordering, and prefix safety. (Issues 01–04)
3. Replay Rust traces against the Lean model, triaging every failure as a
   Rust bug, a Lean-spec problem, or an explicit documented modeling
   mismatch — never assumed to be "Rust is wrong" by default. (Issue 05)
4. Add conformance CI, so Rust changes that violate the formal trace model
   are caught automatically, on every relevant PR. (Issue 05)
5. Refactor Rust module boundaries toward the formal interfaces —
   certificate evidence, snapshots, ordering, and output-prefix discipline.
   (Issue 06)
6. Only later, as a separate initiative, consider CMIR-to-Rholang/MeTTa-IL
   or other generated-code paths. Deferred, and not tracked by any issue in
   this folder..

## Branching

All work for this initiative targets the **`lean-q1`** branch — never `dev`,
never `master`. `lean-q1` does not exist yet; Issue 00 creates it. Every
subsequent issue branches off `lean-q1` and opens its PR back into `lean-q1`.
`lean-q1` gets merged into `dev` as a single reviewed unit once the quarter's
scope is complete — that merge is out of scope for these issues and will be
handled separately.

## Building and running checks

Install [elan](https://github.com/leanprover/elan) (Lean's toolchain
manager, the Lean equivalent of `rustup`) — it reads `lean/lean-toolchain`
and transparently installs the pinned Lean version. Then, from the repo
root:

```bash
just lean-build         # cd lean && lake build
just lean-check-sorry   # lake build, then fail if any declaration uses `sorry`
```

The first build fetches [Mathlib](https://github.com/leanprover-community/mathlib4)
and downloads its precompiled `.olean` cache (several hundred MB, a few
minutes); every build after that only recompiles `lean/LeanVerification/*`
against the cached Mathlib and finishes in seconds. `lake build`'s exit
code does **not** fail on `sorry` (it's a warning, not an error), which is
why `just lean-check-sorry` greps the build log explicitly — always run it
before opening or updating a PR, not just `just lean-build`.

## What each issue's PR should include

- The Lean module(s) the issue owns, building with zero errors and zero
  `sorry` (`just lean-check-sorry` must pass).
- Every lemma/theorem fully proved — no partial proofs, no `sorry`,
  no admitted goals.
- A `docs/lean/NN-<issue-name>.md` writeup covering: the issue's
  definitions in plain English alongside their Lean signatures,
  statements (not full proofs — link to the Lean source for that) of the
  issue's key theorems each with a short paragraph on why they're true,
  and a table mapping each Lean theorem to the Rust function it justifies.
- Lean CI (`.github/workflows/lean.yml`) green on the PR.
- Atomic, semantic commits — one logical unit per commit (a definition
  group, a proved lemma, a doc file), not one squashed diff.

## Required reading before touching any issue

1. `docs/cordial-miners/09-ratification-math.md`, `10-leader-finality.md`,
   `11-tau-ordering.md` — how the Rust side already documents these concepts.
2. `crates/cordial-miners-core/src/consensus/cordiality.rs`,
   `finality.rs`, `ordering.rs`, `blocklace.rs` — the code being verified.

Each individual issue lists additional issue-specific reading.

## PoR slashing conformance

`just slashing-conformance` validates deterministic production Rust fixtures and
replays them through `SlashingRef`, including schema and result mutation tests.
See [the versioned schema and trust boundary](../docs/lean/11-slashing-conformance.md).
These fixtures cover structural penalty transitions; they do not verify semantic
evidence authentication or the rating/blend/clamp pipeline in Lean.
