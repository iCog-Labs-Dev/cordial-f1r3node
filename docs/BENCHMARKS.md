# Benchmarking

Criterion measures component elapsed time. The separate multi-node harness measures
client-observed deploy completion under a specified offered load. Neither replaces
correctness, safety, or conformance testing. Prometheus/Grafana monitoring remains
outside this change.

## Requirements and modes

Run from the repository root in Ubuntu/WSL. Use the repository's Rust toolchain,
the sibling `../f1r3node-rust` checkout required by workspace path dependencies, and
`protoc` for adapter builds. Python tools use Python 3.10+ and the standard library.
The live harness also requires Docker CLI access to the containers being measured.

The wrapper records the actual dependency path, commit, and dirty state. Missing
tools/checkouts are recorded as failed provenance commands instead of crashing
before metadata can be written. Cargo still requires the checkout configured by
the workspace's path dependencies. General dependency setup and compiler
requirements are maintained separately from this benchmark update.

Routine measurement fixtures are the default. `--large` sets `CORDIAL_BENCH_LARGE=1`.
`--smoke` sets `CORDIAL_BENCH_SMOKE=1` and passes Criterion's `--test`: it checks
every workload family on smaller fixtures, without warmup or statistical sampling.
It retains fixture assertions and real signature verification. Validation uses
8/16-block histories; other shared fixtures use their smallest default size.
Traversal uses 100 blocks/four validators, proposals four validators, translation
0/10 deploys, application replay 10 events, and optional RSpace execution one deploy.
Routine signed-validation measurements use 16/32-block histories. The 100,
1,000 and 4,000-block histories are explicitly selected with `--large` because
strict validation scales steeply with history depth. Smoke mode uses 8/16 blocks.
Other benchmark families retain their documented routine sizes.

The legacy `-- --test` command enables the same reduced mode. Direct
`cargo bench ... -- --test` also reduces fixtures. Smoke and `--large` cannot be
combined in the wrapper. Smoke IDs may differ from measurement IDs; smoke runs
do not create performance baselines. An exact filter ending in `/100$`, for
example, will not match the 8/16-block smoke validation fixtures.
Real RSpace execution is independently opt-in with `--rspace`, which sets
`CORDIAL_BENCH_RSPACE=1`. Enabling large fixtures alone never bootstraps RSpace.
Without the wrapper, set those environment variables explicitly.

## Compile, smoke-check, and measure

Run these checks against the exact source and dependency revisions intended for
the PR. A successful ordinary smoke run excludes the opt-in RSpace execution
path and does not establish a statistical performance baseline.

```bash
# Formatting and compilation
cargo fmt -p cordial-miners-core -p cordial-por -p cordial-app-runtime -p cordial-f1r3node-adapter -p cordial-f1r3space-adapter -- --check
cargo bench --workspace --no-run

# Run fixture preflight assertions without Criterion statistical measurements
python3 scripts/benchmark.py --smoke

# First check the formerly slow target alone
python3 scripts/benchmark.py --smoke --package cordial-miners-core --bench block_validation

# Offline benchmark-runner regression checks; never launch Cargo or benchmarks
python3 -m unittest discover -s scripts/tests -p 'test_benchmark.py'

# Offline load-harness behavior tests; no nodes or Docker are contacted
python3 -m unittest discover -s scripts/tests -p 'test_multi_node_load.py'

# Existing crate correctness suites
cargo test -p cordial-miners-core -p cordial-por -p cordial-app-runtime
cargo test -p cordial-f1r3node-adapter --tests

# Relevant lint checks (compile targets but do not execute them)
cargo clippy --workspace --all-targets --no-deps -- -D warnings

# Measure all four component packages, retaining console logs and provenance
python3 scripts/benchmark.py

# Select a package/target
python3 scripts/benchmark.py --package cordial-miners-core --bench block_validation

# Explicit expensive graph/storage workloads
python3 scripts/benchmark.py --large --package cordial-f1r3node-adapter --bench lmdb_persistence

# Real interpreter/genesis preflight, then measurement
python3 scripts/benchmark.py --smoke --rspace --package cordial-f1r3node-adapter --bench execution
python3 scripts/benchmark.py --rspace --package cordial-f1r3node-adapter --bench execution
```

The first smoke run can still spend significant time compiling optimized binaries.
With those binaries cached, smoke mode should avoid the previous large-graph
validation stall; its elapsed time has not yet been measured. The runner prints
each target before launching it, flushes logs continuously, and records a running
target in metadata before setup starts. Ctrl+C stops Cargo and its benchmark
children and records an interrupted result. On Ubuntu/WSL, run the wrapper in
the foreground so it receives the terminal interrupt.

Genesis setup may take tens of seconds, and the real interpreter benchmark uses
temporary LMDB storage. It signs funded development-key deploys, replays from the
same genesis pre-state, checks successful processing and reproducible post-state,
and measures the production RSpace adapter. It is not a sustained-chain benchmark.

## Coverage and measurement boundaries

| Target | Cases and default inputs | Measured work |
| --- | --- | --- |
| core `block_validation` | Exact four-validator histories of 16/32 blocks (100/1,000/4,000 additionally in large mode); signed default/strict, signature-disabled comparison, insertion | Production validation including Secp256k1 when enabled; insertion into fresh history |
| core `leader_lookup` | 1/3 waves; 4/10 validators; uniform/skewed weights | Latest weighted final leader; expected identity checked before timing |
| core `tau_ordering` | 1/3 waves; cold/warm cache; new block, new finalized wave, retained-reference checkpoint, changed weights; pending/finalized future-only epochs; actual reclamation and subsequent finalized wave on 2/3 waves; sort 50/500 blocks | Ordering after a change; mutations and warming are setup |
| core `height_bootstrap` | 100/1,000 blocks, strict or retained window | Actual Blocklace ingestion; canonical content preserved; no benchmark-local mirror or sort |
| core `dag_traversal` | 100/1,000 deep; 4/10/50 validators across exactly 11 layers | Inclusive ancestry, reachability and closure |
| core `proposer_selection` | 4/20/100 validators, five layers | Predecessor selection and BlockContent assembly; payload clone included, hashing/signing excluded |
| core `consensus_scenarios` | 3/9 layers: sparse references, missing leader, equivocation, insufficient support | Weighted ordering on non-ideal graphs; delayed arrival is covered by production startup buffering |
| core `pruning_gc` | 3/10 waves; three growth/checkpoint cycles for retained-reference and reclaimable graphs | Checkpoint operations and periodic growth; retained payload and Linux process RSS/HWM logged outside timing |
| core `deploy_pool` | 100/1,000 deploys | Batch admission, ancestry-filtered selection, expiry pruning; bookkeeping signatures are synthetic IDs |
| adapter `ordered_output` | 100/1,000 identities; append 10; snapshots of 24/72 blocks with equal content across four creators | Publication, access/encoding/construction, and production snapshot decoding/indexes/ordering/translation |
| adapter `block_translation` | 0/10/100/500 deploys; separately 10 deploys with 1/4 signed-proof authorization carriers | Model conversion/internal serialization and hashing; no signature verification, wire byte encoding, transport or execution |
| adapter `startup_recovery` | 100/500-block imports; persisted future epochs with 30/90 retained blocks, sealed and later heads | Production mirror imports; LMDB window lookup; epoch file reopen/read/checksum validation, replay and installation |
| adapter `lmdb_persistence` | 100/500 blocks at 256 bytes; separately signed 100-block recovery; three checkpoint cycles | Real per-block transactions, warm lookups, production sorting/recovery/startup, durable growth/cursor commits |
| adapter `execution` | Opt-in 1/10 deploys | Real Rholang execution and post-state replay, including translation, signature checks and storage |
| adapter `por_checkpoint` | Four validators, 36 signed blocks, first 16-block finalized checkpoint, 1/2 equivocators | Real finalized penalty authorization; authenticated penalty replay; v3 checkpoint reputation audit |
| PoR `reputation` | 10/100 participants, four sparse incoming ratings each | Normalization, Liquid Rank and no-penalty transition replay; rating authentication is an upstream boundary |
| app `event_processing` | 100/1,000 finalized events, four apps, every fifth rejected | Routing, duplicate checks, receipts, cursor and replay; local app counts bytes rather than implementing business logic |

Large mode extends sizes in the shared fixtures; the exact numeric benchmark IDs
and configuration are retained in logs. Existing traversal, proposal and translation
cases are already bounded and do not change with large mode. RSpace large mode
also measures 32 deploys per block.

Inputs, keys, signatures and expected outcomes are constructed/verified before
timing. Validation constructs fixtures inside Criterion's selected-case callback,
before the iteration loop, so filtered-out validation cases perform no setup or
preflight validation. Mutable graphs and containers use `iter_batched_ref` so fixture destruction
is outside timing. The replaced *old* output prefix is destroyed during publication,
which is part of the production update operation. Owned results returned to
Criterion are dropped after the timed routine where the selected iteration API
supports that; read-loop operation costs can include returned-value disposal.
Sustained growth includes block cloning, payload ownership, checkpoint-prefix
materialization and per-block writes because those are part of the workload.

Shared graph/crypto fixtures live in `cordial-miners-core/src/bench_support.rs`
behind the optional `bench-support` feature. Benchmark consumers enable this
feature through development dependencies; normal builds leave it disabled.
The execution benchmark keeps its genesis setup inside `execution.rs` and calls
public host genesis/storage/runtime APIs, with temporary LMDB storage retained
until the benchmark exits. No separate fixture crate or setup folder is needed.

Graph fixtures explicitly use a non-cryptographic verifier; signed validation and
signed recovery are separately named. HashMap iteration remains randomized by Rust;
fixed fixture inputs do not eliminate all process-to-process variability.

Checkpoint growth cases distinguish two reference patterns. Fully populated
four-validator rounds keep older ancestors referenced by checkpoint peers; these
cases assert zero removals and retain 12 blocks per wave. Reclaimable cases have
one rotating leader at each wave boundary and four validators in each following
round. After each checkpoint, all earlier waves can be reclaimed, leaving the
latest nine-block wave. Preflight checks assert exact removed/retained counts and
the final checkpoint. The LMDB variants retain all disk records while pruning
only the in-memory graph; throughput uses the actual persisted block count.
Memory reporting still includes saved ordering prefixes and allocator retention,
so nine retained blocks does not imply bounded total process memory.

The pruning case names now include the reference pattern. Establish new baselines
for these cases; their growth fixtures differ from the original suite.

The ordering target also measures `checkpoint_retained_references` separately
from `pruning_reclaims_history`. The latter warms the ordering cache, reclaims
exactly `9 * (waves - 1)` blocks during setup, and checks that cached and uncached
ordering preserve the full finalized prefix. Only the first ordering call after
reclamation is timed. `new_finalized_leader_after_pruning` then appends another
wave during untimed setup and checks the resulting order against an unpruned
reference DAG, including preservation of the saved prefix.

The direct pruning fixture selects the rotating leader for its checkpoint wave
and proves weighted finality before calling the low-level pruning API.
`weight_change` measures raw cache invalidation. The separate
`future_weight_epoch_pending` and `future_weight_epoch_finalized` cases call
production `weighted_tau_after_epoch_boundary`, preserving the old sealed prefix
when the next wave is pending and extending it when a new leader is final.
Boundary checks and epoch ordering are timed; graph construction and weight-table
preparation are excluded. These fixtures have full participation under both weight
tables; they do not cover a finality outcome that changes solely because of weights.

### Coverage added by subsequent production changes

The suite compiles against the creator-bound `block_hash` API (native hash format
v3). Shared signed and structural fixtures use this production hash.

The additional paths use these measurement boundaries:

- `por_checkpoint/authenticate_finalized_penalties` authenticates real conflicting
  Secp256k1 blocks and the signed carrier published in actual production finalized
  order. Signing, graph construction, rating exchange and cloning the closed round
  are setup. It calls `with_finalized_equivocations`; no permissive authorization
  policy substitutes for production checks.
- `por_checkpoint/penalty_transition_replay` starts with the authenticated
  checkpoint ratings and penalty set. One of four equivocators exercises the base
  slash; two exercise the correlated full slash. Preflight checks exact offenders,
  ejection and retained capital, and rejects an omitted or invalid proof.
- `por_checkpoint/checkpoint_reputation_audit` checks the v3 source/commitments and
  replays the transition with those penalties. Ratings have already been
  cryptographically authenticated by the real exchange closure during setup.
  Preflight rejects a changed finalized source. This case excludes durable state
  application and transport.
- `startup_recovery/persisted_epoch` writes a prepared record before measurement.
  `reopen_and_restore_record` times production store reopening (including its
  directory synchronization), file reading and checksum/structure validation.
  `restore_replay_install_sealed_head` and `restore_replay_install_later_head`
  additionally verify the retained history and install the epoch into fresh
  ingress, checking weights and exact prefix preservation. Rebuilding the retained
  graph is setup. The fixed external commitment represents the hash supplied by
  audited reputation state; this case does not benchmark recovery of that separate
  state or the entire node startup. Filesystem caches are not flushed.
- `snapshot/equal_content_creators` builds four blocks per layer with identical
  payload/predecessor bytes and different creators. Preflight checks all hashes,
  block-number/height/child indexes and latest messages remain distinct. Timing
  includes the complete production `build_snapshot` operation; hashing and graph
  insertion are setup.
- `block_translation/por_authorizations` measures both conversion directions with
  actual encoded signed proofs in the versioned carrier. Preflight checks exact
  payload, identity and carrier round trips. It times serialization and hashing,
  not proof authentication. Byte throughput refers to authorization carrier bytes;
  each block also contains ten deploys.

Smoke mode uses one offender, a 30-block epoch history, a 24-block snapshot and
one authorization carrier. Defaults add two offenders, 90-block epoch histories,
72-block snapshots and four carriers. Large mode adds three offenders, 300-block
epoch histories, 240-block snapshots and sixteen carriers. None requires RSpace
or a running cluster.

~~~bash
# Fixture assertions only; no statistical measurements
python3 scripts/benchmark.py --smoke --package cordial-f1r3node-adapter --bench por_checkpoint
python3 scripts/benchmark.py --smoke --package cordial-f1r3node-adapter --bench startup_recovery -- 'persisted_epoch/'
python3 scripts/benchmark.py --smoke --package cordial-f1r3node-adapter --bench ordered_output -- 'snapshot/'
python3 scripts/benchmark.py --smoke --package cordial-f1r3node-adapter --bench block_translation -- 'por_authorizations/'

# Example statistical baseline for the new authentication case
python3 scripts/benchmark.py --package cordial-f1r3node-adapter --bench por_checkpoint -- --save-baseline por-checkpoint-v1 'authenticate_finalized_penalties/1$'
~~~

The runner discovers the added `por_checkpoint` target from Cargo metadata, so
the package-wide smoke command and adapter benchmark CI include it automatically.

## Recovery trust boundary

Full LMDB recovery uses the real strict production recovery path, including its
existing sorting implementation. Missing historical predecessors are accepted only
through production APIs explicitly named `insert_with_trusted_boundary` or
`ingest_with_trusted_boundary`. These preserve original block content and record
boundary metadata. The persisted-window case writes the retained blocks and cursor,
then imports blocks in predecessor-first manifest order through the production
mirror API.

A trusted window is a bounded inspection/import view, not a fully closed consensus
DAG or cryptographic proof of a checkpoint. The benchmark does not relax strict
database recovery or invent support for restoring protocol checkpoint metadata.
The `trusted_partial_window` cases explicitly assert missing history, an open
partial view, and the absence of a restored protocol checkpoint. Production
`recover_into_engine` uses strict insertion but is best effort: it skips orphaned
blocks and can still return the stored cursor. Preflight verifies that this path
does not admit the partial window; the cursor is not treated as proof of complete
recovery. Full protocol checkpoint restoration is outside this component-benchmark
change.

Large deep-history recovery may expose recursion/stack limits in production sorting;
the suite intentionally does not substitute a different algorithm to hide them.

## Baselines and provenance

```bash
python3 scripts/benchmark.py --package cordial-miners-core --bench block_validation -- --save-baseline signed-v1
# After the next implementation change, on comparable hardware/toolchain:
python3 scripts/benchmark.py --package cordial-miners-core --bench block_validation -- --baseline signed-v1
```

Keep the Criterion target directory between those runs. Some workloads were renamed
or changed, so previous fixtures are not comparable: establish fresh baselines.
After the creator-bound hash migration, establish fresh baselines for signed
validation and graph/storage fixtures: identities and deterministic ordering
tie-breaks have changed. The direct-pruning fixture now uses the actual final
leader, so its old results also describe a different fixture.
New benchmark IDs have no prior baseline. A baseline name alone does not identify
the compiler/hardware; use the saved metadata as well.

Each wrapper invocation creates `target/benchmark-runs/<UTC-time>-<id>/` with:

- `metadata.json`: commit, dirty-tree state/diff fingerprint, hashes of untracked
  sources, sibling dependency commit/state, Rust/Cargo versions, machine/CPU/memory,
  selected workloads/environment, exact commands and exit codes.
- Per-target console logs and elapsed time/status, including interrupted targets.
- For measurement runs, a copy of `target/criterion/`, including HTML, statistics and retained baselines.
  This tree can contain older cases; logs identify workloads executed by this run.
  Smoke runs retain metadata/logs only and skip copying existing Criterion reports.

The wrapper supports `CARGO_TARGET_DIR`; it records this setting and archives its
Criterion directory. It does not support custom Criterion output-directory options.
An externally set `CRITERION_HOME` must be unset when using this wrapper.
Use a quiet, comparable environment and a pinned toolchain for comparisons.
No automatic performance threshold is introduced. CI records results; performance
gates should follow measured noise and an agreed regression policy.

## PR validation evidence

Run the following after the compile/lint/correctness checks above. Store the
reported run directories in the PR description.

~~~bash
# Check fixtures, including actual ordering reclamation.
python3 scripts/benchmark.py --smoke

# Separately enable the real interpreter; ordinary smoke skips this target.
python3 scripts/benchmark.py --smoke --rspace --package cordial-f1r3node-adapter --bench execution

# Small statistical measurements with provenance; filters avoid the full suite.
python3 scripts/benchmark.py --package cordial-miners-core --bench block_validation -- --save-baseline review-v1 'validate_default_signed/16$'
python3 scripts/benchmark.py --package cordial-miners-core --bench tau_ordering -- --save-baseline review-v1 'pruning_reclaims_history/2$'
python3 scripts/benchmark.py --package cordial-f1r3node-adapter --bench startup_recovery -- --save-baseline review-v1 'trusted_window_from_store/100$'
~~~

Compare later changes with `--baseline review-v1` on the same environment.
Create a new baseline for the renamed partial-window and reclamation workloads.
Do not reuse the old 100/1,000-block validation results for 16/32-block cases.

A short live multi-node run must also finish with `complete: true`,
`successful_execution_finalized == offered_deploys`, zero finalization backlog,
nonempty latency distributions, and resource samples for every configured
container. Keep `summary.json` and the raw records. Offline mocked tests verify
harness behavior but do not validate the live HTTP schema, funding, genesis,
network propagation, or node execution. If the configured cluster is unavailable,
record this as outstanding validation rather than declaring end-to-end coverage
verified.

## Multi-node load benchmark

Start the existing development cluster and wait for node readiness. Ensure block
production/heartbeat is active and the development deployer is funded. Read the
current finalized height and pass it to the generator; old valid-after heights
may cause deployments to expire. This workload spends development-chain phlo.

```bash
just demo-cordial-four-node-cluster-up
just demo-cordial-four-node-cluster-wait

# Inspect container names and the finalized block number.
docker compose --env-file docker/.env -f docker/four-node-cluster.yml ps
curl -fsS http://127.0.0.1:51403/api/last-finalized-block

# Replace 123 with the block number reported above.
cargo run -p cordial-f1r3node-adapter --bin benchmark_deploys -- \
  --count 100 --valid-after-block-number 123 > /tmp/cordial-load.jsonl

# Replace container names with the actual names/IDs shown by compose ps.
python3 scripts/multi_node_load.py \
  --requests /tmp/cordial-load.jsonl --count 100 --rate 2 \
  --node http://127.0.0.1:51403 --container cordial.validator1 \
  --node http://127.0.0.1:52403 --container cordial.validator2 \
  --node http://127.0.0.1:53403 --container cordial.validator3 \
  --node http://127.0.0.1:54403 --container cordial.validator4 \
  --settle-seconds 120 --poll-interval 1 --concurrency 16
```

The generator writes fresh signed requests without submitting them. Its default
development key matches the host's funded test fixture; use a prepared JSONL corpus
signed by a funded account if your cluster genesis differs. Never use that key as
a production credential. Each corpus must contain distinct deploy signatures and
must be newly submitted. The harness refuses inputs already included on the first
node, but operators must also avoid reusing pending deployments.

The harness distributes submissions round-robin at `--rate` requests/second.
HTTP concurrency is bounded. Actual submission scheduling lag and latency from the
planned arrival time are reported separately, so an overloaded client cannot
silently make the offered rate appear sustainable. Finality requires the exact
signature to be reported in the same finalized block by **every** configured node.
The node response must explicitly contain `isFinalized: true` and an execution
outcome. Acceptance or block inclusion alone never counts as finalization.

Outputs under `target/load-runs/<UTC-time>-<id>/`:

- `summary.json`: provenance/configuration, acceptance/finalization counts,
  successful versus errored execution, median/p95/p99 observed latency,
  per-node latency, observed throughput during the offered window and including
  drain, end backlog, per-container CPU/memory summaries, and resource-collection errors.
- `deploys.jsonl`: per-deploy scheduling, acceptance, per-node finalization,
  timeout/error and execution outcome.
- `backlog.jsonl`: accepted-but-not-finalized workload samples.
- `resources.jsonl`: Docker CPU percentage and memory usage per node/container.

Latency is a client-observed upper bound including network and polling delays,
not an exact internal consensus timestamp. Polling resolution and HTTP query load
affect observations. Percentiles describe completed deployments only; always read
timeout/failure counts alongside them. A submission error can include an ambiguous
HTTP timeout after node acceptance; it is never counted as a confirmed completion. Docker memory is Docker-reported usage,
not process RSS; CPU may exceed 100% on multiple cores. Pruning RSS is process-wide
and includes allocator retention, fixtures, indexes and saved finalized prefixes.
LMDB growth checkpoints persist cursors but do not delete historical disk records.

Temporary LMDB write/growth fixtures explicitly close each repository before
removing its directory. Heed 0.20 retains environments in a process-wide registry;
dropping an ordinary handle alone does not release those resources. Criterion
per-iteration batching keeps the close/wait and directory cleanup outside timing.
The signed recovery throughput is the actual number of recovered blocks, rather
than the preceding case's workload count.

For resource-lifecycle changes, run the focused regression check and then the full
LMDB measurement target. A smoke run alone does not exercise repeated environment
creation during warmup and sampling:

~~~bash
cargo test -p cordial-f1r3node-adapter --test test_repository explicit_close_releases_environment_and_preserves_persisted_data
python3 scripts/benchmark.py --package cordial-f1r3node-adapter --bench lmdb_persistence
~~~


Exit status: 0 means all deploys executed successfully and finalized consistently
on all nodes with resource collection intact; 2 means incomplete/failed workload;
1 means configuration, readiness, input or other fatal error. Raw records are
retained for incomplete runs. Prometheus/Grafana installation and automatic cluster
startup/shutdown are deliberately outside the harness.
