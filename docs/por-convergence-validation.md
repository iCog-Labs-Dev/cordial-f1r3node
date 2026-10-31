# PoR convergence validation

These changes separate numerical convergence, observer agreement, durable
recovery, and consensus activation. The test commands below have **not been run**
as part of this implementation.

## Scenarios

- The existing four-observer demo now independently collects and authorizes its
  first checkpoint in each observer, using different delivery orders.
- Observer exchanges start incomplete, must wait for peers, and receive
  reordered and duplicate signed rating envelopes.
- A 128-round runner covers uniform and unequal initial reputation, an isolated
  slash, and correlated full slashing at round 32. Every observer checks the same
  checkpoint and state each round and restarts at and after the shock.
- At scale 1000, numerical convergence requires at least eight consecutive final
  rounds with score delta at most 1, voting-share delta at most 1000 ppm, and
  unchanged deterministic ranking. The test asserts positive total voting weight;
  numerical collapse to an unusable all-zero vector is not a passing result.
  These are explicit scenario acceptance bounds, not a universal convergence
  theorem.
- Four-observer boundary scenarios use 299/1000, 300/1000, and 301/1000 offending
  reputation and verify exact integer retained balances.
- Four real adapter runtimes process signed DAG layers with different delivery
  orders, wait for a fixed future boundary, activate four epochs, slash in
  checkpoint two, restart after every activation, and require new finality
  after the last restart.
- Journal coverage checks two historical cutovers, rejection of history
  replacement, uncommitted-tail removal, corruption, and legacy v1 reading.

## Epoch journal

The existing epoch file now stores a checksummed sequence of individually
checksummed records, with atomic replacement and directory fsync. Appends must
preserve consecutive PoR rounds, increasing effective waves and prefix lengths,
identical membership, and continuity between old and new weight tables.
Previously written single-record files remain readable.

Recovery binds recorded checkpoint hashes to durable reputation history.
Historical epochs replay only up to the next recorded boundary; only the latest
epoch replays to the current head. New weights never recalculate an earlier
epoch's prefix. A durable proposal whose state never committed is removed only
from the journal tail.

The journal retains the existing file-size bound. It has no history-pruning
policy; deployments must retain the corresponding DAG and reputation history.
The fixed activation delay remains a shared protocol setting. Late arrivals
fail closed rather than silently changing the activation wave.

## Commands (run from the repository root in WSL)

Focused adapter and journal scenarios:

```bash
cargo test -j 1 -p cordial-f1r3node-adapter \
  --test test_por_checkpoint_penalties \
  --test test_por_weight_epoch_record \
  --test test_por_runtime \
  --test test_future_weight_epoch
```

Core ordering and PoR arithmetic regression suites:

```bash
cargo test -j 1 -p cordial-miners-core
cargo test -j 1 -p cordial-por
```

Python smoke-gate regressions:

```bash
python3 -B -m unittest \
  docker/scripts/test_verify_four_node_por_shadow.py \
  docker/scripts/tests/test_four_node_por_shadow.py
```

Export multi-round trajectories:

```bash
POR_CONVERGENCE_OUTPUT="$PWD/docs/demos/traces" \
  cargo test -j 1 -p cordial-f1r3node-adapter \
  --test test_por_checkpoint_penalties \
  multi_round_ranking_and_slashing_convergence -- --exact --ignored
```

Each scenario writes JSON with checkpoint identity, active and retained
reputation, exclusions, voting weights, total weight, score/share deltas, and
ranking stability. Rebuild the presentation fixture with:

```bash
bash docker/scripts/build-por-demo-fixture.sh
```

For a running isolated cluster, use the full command in the shadow-runner guide,
setting `--min-por-round 4 --min-new-finalized-blocks 1`, with committed checkpoint
ratings and a shared epoch delay. This launches observers and restarts a validator;
use the isolated cluster, not an unrelated existing deployment.

## Scope

The runtime scenario exercises LiveIngress and PorRuntime finality/activation.
It does not inject an attack into a running f1r3node cluster or execute a host-chain
slash deploy. Key rotation and authenticated inactivity remain separate host
integration scenarios. Passing these tests would establish the stated scenario
properties, not general collusion resistance or liveness beyond the fault model.

## Dashboard import

Open the [PoR observatory](demos/por-weighted-slashing.html) and select
**Import results**. The current exporter writes versioned
por_convergence_trace objects with explicit validator labels, convergence
bounds, observer agreement, slash/recovery markers, and a per-scenario
completion report. The UI also accepts earlier array-only exports, without
guessing missing label mappings or treating a missing report as a pass.

The dashboard is a reader, not a test runner. Files are written only after
the corresponding scenario assertions pass. No new run results were generated
during the UI implementation.
