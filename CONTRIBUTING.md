# Contributing

Thanks for your interest in contributing. This guide covers what you need to set up, how to find work, and what we expect in a pull request.

If anything here is unclear or out of date, file an issue — we'd rather fix the docs than have you guess.

---

## Setting up your environment

### For the core consensus crate only

```bash
git clone https://github.com/iCog-Labs-Dev/cordial-f1r3node.git
cd cordial-f1r3node
cargo +nightly-2025-06-15 test -p cordial-miners-core
```

That's it. You only need a Rust toolchain (nightly recommended, see `Justfile` for pinned version).

### For the full workspace including f1r3node integration

You'll need three things:

1. **`protoc` (Protocol Buffers compiler)** — required to build f1r3node's `models` crate.
   - Linux: `sudo apt install protobuf-compiler`
   - Mac: `brew install protobuf`
   - Verify: `protoc --version`

2. **f1r3node checked out as a sibling directory.** The path dependencies in `crates/cordial-f1r3space-adapter/Cargo.toml` expect to find f1r3node at `../f1r3node` relative to this repo. So your layout should be:

   ```
   ~/your-projects/
     cordial-f1r3node/  ← this repo
     f1r3node/        ← https://github.com/F1R3FLY-io/f1r3node
   ```

   If you keep them somewhere else, edit the path deps in `crates/cordial-f1r3space-adapter/Cargo.toml` accordingly.

3. **Rust nightly toolchain.** This workspace uses nightly features. Use the pinned toolchain in `Justfile` (`nightly-2025-06-15`) or a recent nightly.

Then verify:

```bash
just build
just test
```

First build is slow (~5-7 min) because it compiles f1r3node's Rholang interpreter and RSpace tuplespace. Subsequent builds are fast.

### Common build issues

| Symptom | Cause and fix |
|---------|---------------|
| `tonic_prost_build` errors about `protoc` | Install `protoc` (see above) |
| `gxhash` errors about AES/SSE2 intrinsics | Make sure `.cargo/config.toml` exists at the repo root. It should be checked in — if it's missing, that's a bug |
| Stack overflow in Rholang tests | `RUST_MIN_STACK=8388608` is set via `.cargo/config.toml`. Verify it's there |
| `models` path dep "file not found" | f1r3node must be at `../f1r3node`. See setup step 2 |
| `smallvec` feature error on stable | Use nightly toolchain: `cargo +nightly-2025-06-15 ...` |

---

## Finding work

[`docs/INTEGRATION_NEXT_STEPS.md`](docs/INTEGRATION_NEXT_STEPS.md) lists open tasks ordered by impact. Each task documents its scope, difficulty, and the files you need to read. Pick one that fits your available time.

If you have an idea that isn't on the list, open a GitHub issue describing what you want to do *before* writing code. That avoids wasted work and gives maintainers a chance to flag conflicts with planned work.

For larger architectural changes, open an RFC-style issue first. "Larger" includes anything that changes a public API in `cordial-miners-core`, restructures the workspace, adds a new dependency, or shifts the layering between the three crates.

---

## Working on a change

### Branching

Branch off `main` (or `master` if that's the default). Use a descriptive name:

- `task1/e2e-rholang-test` for tasks from `docs/INTEGRATION_NEXT_STEPS.md`
- `fix/cordial-condition-empty-tips` for bug fixes
- `docs/clarify-snapshot-collision-note` for doc-only changes
- `feature/lmdb-storage-backend` for larger features

### Keep PRs scoped

One logical change per PR. If you're tempted to bundle "fix bug + add new feature + refactor a helper," split it into three PRs. Reviewers can handle small changes quickly; large mixed PRs sit in review for days.

A reasonable PR is roughly:
- Up to ~500 lines of code change
- One module or one cohesive feature
- A clear single-sentence description of what it does

If your change is larger than that, it's almost always a sign you should split it.

### Commit style

Each commit should be self-contained: it compiles, tests pass, and the message explains *why* the change was made. The "what" is usually visible in the diff; the "why" is what reviewers and future-you will need.

Good commit messages look like:

```
Wire compute_bonds into F1r3RspaceRuntime so new_bonds reflects post-state

Bonds in f1r3node are state-hash-addressable: after running deploys,
the post-state may have different bonds (e.g. if a Slash system deploy
removed a validator). Previously we echoed the request's bonds verbatim
in ExecutionResult, which is silently wrong.

Now execute_block calls runtime_manager.compute_bonds(post_hash) and
translates the result into our Bond type.

Test added in test_translation.rs covering the slash → updated bonds path.
```

That's three things: a one-line summary, a paragraph explaining why, and a note on what tests prove the change.

For multi-commit PRs, each commit should be reviewable on its own. Don't squash everything into a single commit unless the PR is genuinely one logical change.

### Testing

Before opening a PR:

```bash
just fmt
just clippy
just test
just check-core-boundaries
```

If any of those fail, fix it before pushing. Reviewers will assume green CI as a baseline.

When adding a feature, write tests for it. When fixing a bug, write a test that would have caught the bug. Don't add a test that's hard to read just to bump the coverage number — clear tests are more valuable than many tests.

For tests in `cordial-f1r3space-adapter` that touch real f1r3node types, prefer **unit tests on the translation helpers** over end-to-end tests that need a live `RuntimeManager`. The translation surface is what we own; the runtime is f1r3node's responsibility.

### Documentation

Update docs in the same PR as the code change, not in a follow-up:

- If you change a public API in `crates/cordial-miners-core`, update `docs/implementation.md`.
- If you complete a task from `docs/INTEGRATION_NEXT_STEPS.md`, mark it done there.
- If you add a new f1r3node-related capability, update `docs/cordial-miners-vs-cbc-casper.md`.
- If you change the build setup, update `README.md` and the troubleshooting table here.

If your change has tradeoffs that aren't obvious from reading the code, document them in a module-level comment or in the PR description. Future contributors will thank you.

---

## Architectural rules

These exist because the three-crate split is the project's main design decision. Violating them creates pain that's hard to undo.

### Don't let f1r3node types leak into `cordial-miners-core`

The `cordial-miners-core` crate has zero dependencies on f1r3node crates. Adding any will force every consumer of the standalone consensus library to compile f1r3node's tree.

If you find yourself wanting to import a type from `models` or `casper` into `cordial-miners-core`, that's a signal the abstraction is wrong. Discuss in an issue first.

**Enforcement:** The repo includes a CI guardrail script at `scripts/check_core_boundaries.sh` that greps for forbidden imports. Run `just check-core-boundaries` before pushing.

### Don't let f1r3node real crates leak into `cordial-f1r3node-adapter`

`cordial-f1r3node-adapter` uses *mirror types* of f1r3node's `BlockMessage`, `Body`, `CasperSnapshot`, etc. — plain Rust structs with the same shape. This lets the adapter build standalone, without f1r3node checked out.

If you want to use f1r3node's real types, do it in `cordial-f1r3space-adapter` (which path-depends on f1r3node) or in a new fourth crate. Don't add f1r3node path deps to `cordial-f1r3node-adapter`.

There is an open question about consolidating the mirror types — see Task 5 in `docs/INTEGRATION_NEXT_STEPS.md`. Until that's decided, mirrors stay.

### Be honest about caveats

If your code has a known limitation, document it in the module header or function docs. We have a culture of "honest caveats" — see the bottom of the f1r3node adapter's module docs and the RSpace runtime's `// Known caveats` section for examples.

This matters because integration code is full of subtle issues (signature algorithm mismatches, hash format differences, missing fields). A future contributor needs to know what's been deliberately left for later versus what's an unintentional bug.

---

## Submitting a pull request

1. Push your branch to your fork or to the main repo (if you have access).
2. Open the PR against `main` (or `master`).
3. In the description, link any related issues and include:
   - **Summary** of what the PR does, in 1-2 sentences
   - **Why** this change is needed (rationale, not just restating the diff)
   - **Test plan** — what you ran to verify it works
4. If the change touches integration code, mention which of the three crates it modifies and confirm you didn't violate the layering rules above.
5. Wait for review. Most PRs get a first response within a few days.
6. Address review feedback as new commits on the same branch (don't force-push during review unless asked — it makes the diff harder to follow).
7. Once approved, a maintainer merges. If you have merge access, you can do it yourself.

### What gets a PR rejected

- Failing tests or clippy warnings
- Mixing unrelated changes
- Breaking the architectural rules above without prior discussion
- Removing tests without explanation
- Adding dependencies without justification (transitive cost matters)
- Lack of docs for non-trivial changes

None of these are personal. Reviewers want to merge your PR — these rules just make it possible to keep the project maintainable.

---

## Code style

Follow standard Rust idioms:

- `just fmt` configures formatting; don't argue with the formatter.
- `just clippy` configures most lints; don't suppress warnings without a comment explaining why.
- Names: `snake_case` for functions and modules, `PascalCase` for types, `SCREAMING_SNAKE_CASE` for constants.
- Errors: prefer `Result<T, E>` with explicit error types over `unwrap()`. Test code can use `unwrap()` freely.
- Comments: explain *why*, not *what*. The code already shows the what.
- Module-level docs (`//!`) are appreciated for non-trivial modules.

---

## Useful commands

This repo includes a `Justfile` with common workflows:

```bash
just build          # cargo build --workspace
just test          # cargo test --workspace
just test-core     # cargo test -p cordial-miners-core
just test-adapter  # cargo test -p cordial-f1r3node-adapter
just fmt          # cargo fmt for workspace crates
just clippy       # cargo clippy --workspace -- -D warnings
just check-core-boundaries  # CI guardrail script
just ci           # fmt + clippy + build + test + check-core-boundaries
```

---

## Questions?

- **Implementation questions** — open a GitHub Discussion or comment on the relevant issue.
- **Design discussions** — open an issue with the `design` label.
- **Bug reports** — open an issue with a minimal reproduction.

Welcome aboard.