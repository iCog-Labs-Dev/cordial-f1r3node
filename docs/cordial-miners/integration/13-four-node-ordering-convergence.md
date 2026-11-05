# Four-Node Window-Order Convergence

This note documents the host-side verifier for checking bounded Cordial
window-order convergence across the real four-node `f1r3node` Docker cluster.

## Purpose

The existing cluster verifier checks that validators boot, connect, expose
finalized state, and share the same canonical HTTP-visible block view. This
ordering verifier goes one step further: it mirrors each validator through the
Cordial live adapter and compares a deterministic ordered view of the same
recent mirrored block window.

## Command

Start the real four-node cluster:

```bash
docker-compose --env-file docker/.env -f docker/four-node-cluster.yml up
```

Then run:

```bash
just demo-cordial-four-node-cluster-ordering
```

Or directly:

```bash
./docker/scripts/verify-four-node-cluster-ordering.sh
```

## What It Checks

The verifier runs `live_mirror_check` against each validator:

- `http://127.0.0.1:51401`
- `http://127.0.0.1:52401`
- `http://127.0.0.1:53401`
- `http://127.0.0.1:54401`

For each node, it mirrors live blocks, computes `xsort` over all blocks in the
mirrored recent window by default, writes the ordered block hashes to a
temporary JSON file, and compares each node against validator 1. This default
does not select blocks through finalized leaders as `weighted_tau` does.

By default the script uses a bounded recent-height window with a trusted window
boundary. This keeps the live check practical on long-running local clusters:
blocks inside the chosen window are mirrored and predecessors older than the
window are treated as already-trusted node history. The normal live ingress path
remains strict; this boundary mode is only for the host-side convergence
verifier.

The default mode is therefore a practical convergence smoke test for the live
mirror: all four validators must expose the same recent Cordial-ordered block
view. To exercise the finalized `weighted_tau` path, set
`CORDIAL_ORDER_WINDOW_ORDERING_FRAGMENT=false` while leaving
`CORDIAL_ORDER_FRAGMENT_ONLY=true`. The script requires a nonempty output, so
missing finality evidence fails the check rather than passing by comparing four
empty sequences. A bounded trusted window may not contain enough history for
this check to pass; a full-history `τ` test also needs complete predecessor
closure and the authorized validator weights.

Before exporting each node's order, the script queries every validator's recent
HTTP block view and pins the run to the lowest visible block height. This avoids
comparing moving windows when one validator has produced or observed a few more
blocks than another.

## Success Condition

The verifier passes only when all four validators produce the same bounded
mirrored Cordial window-order block hash sequence.

Expected success line:

```text
PASS: four real f1r3node validators produced the same bounded mirrored Cordial window order.
```

## Runtime Parameters

The script supports these optional environment variables:

- `CORDIAL_ORDER_NODES_FILE`: path to a four-line file with node name, gRPC URL,
  and HTTP URL on each line. Use it for an isolated cluster with different host
  ports. The default file uses the `514xx` through `544xx` validator ports.
- `CORDIAL_ORDER_DEPTH`: recent-block query depth used to discover each node's
  latest visible height. Use a small value such as `16` for fast local checks.
- `CORDIAL_ORDER_HEIGHT_BATCH_SIZE`: block-height bootstrap batch size. Increase
  it for fewer gRPC range calls; keep `64` for predictable local runs.
- `CORDIAL_ORDER_HEIGHT_WINDOW`: number of recent block heights to mirror. Use
  `256` for the default smoke test; increase it when you want a wider recent
  block view.
- `CORDIAL_ORDER_FRAGMENT_ONLY`: keep `true` for the default bounded-window
  export. Set to `false` only when intentionally testing the heavier full
  ordered-output path.
- `CORDIAL_ORDER_TRUSTED_WINDOW_BOUNDARY`: keep `true` for bounded-window runs.
  Set to `false` only when the mirrored window contains full predecessor
  closure, otherwise recent blocks may remain pending.
- `CORDIAL_ORDER_WINDOW_ORDERING_FRAGMENT`: set to `false` to compute the latest
  finalized support-predicate fragment instead of the bounded mirrored window.
  This is slower and should be used for focused finality debugging, not the
  default four-node smoke test.
- `CORDIAL_ORDER_NODE_TIMEOUT`: per-validator timeout in seconds. Increase this
  if running on a slow machine or against a large height window.

Example:

```bash
CORDIAL_ORDER_DEPTH=128 CORDIAL_ORDER_HEIGHT_WINDOW=256 \
  just demo-cordial-four-node-cluster-ordering
```

Recommended fast local check:

```bash
CORDIAL_ORDER_DEPTH=16 CORDIAL_ORDER_HEIGHT_WINDOW=256 \
  just demo-cordial-four-node-cluster-ordering
```

## Notes

This verifier is intentionally host-side because it runs the adapter binary from
this repository and connects to the validator gRPC ports exposed by Docker.

## Finalized `τ` evidence run

Use a fresh, short four-validator cluster for the stronger check. The long-running
cluster's bounded trusted window can omit finality evidence, and deriving bonds
from recent senders can omit a validator. The dedicated runner reads the same
public genesis bond table mounted into the Docker nodes, mirrors from height 0,
and calls `weighted_tau` through `latest_finalized_ordered_output`.

For the isolated `610xx` through `640xx` ports, stop any older project using
those ports while preserving its volumes, then start a fresh Compose project:

```bash
docker compose -p por-live-demo --env-file docker/.env \
  -f docker/four-node-cluster.yml -f docker/four-node-por-smoke-override.yml down
docker compose -p por-tau-evidence --env-file docker/.env \
  -f docker/four-node-cluster.yml -f docker/four-node-por-smoke-override.yml up -d
```

The first command is needed only if `por-live-demo` is actually using the ports;
replace that project name with the one shown by `docker inspect` for your
running containers. `down` here keeps the old named volumes. Wait until the
fresh nodes have finalized blocks, then create the host-port list and run:

```bash
cat > /tmp/cordial-por-smoke-nodes.txt <<'NODES'
por-smoke-validator-1 http://127.0.0.1:61001 http://127.0.0.1:61003
por-smoke-validator-2 http://127.0.0.1:62001 http://127.0.0.1:62003
por-smoke-validator-3 http://127.0.0.1:63001 http://127.0.0.1:63003
por-smoke-validator-4 http://127.0.0.1:64001 http://127.0.0.1:64003
NODES
CORDIAL_ORDER_NODES_FILE=/tmp/cordial-por-smoke-nodes.txt \
CORDIAL_ORDER_BONDS_FILE=docker/genesis/cordial-bonds.txt \
CORDIAL_TAU_EVIDENCE_DIR=/tmp/cordial-tau-run-1 \
  ./docker/scripts/verify-four-node-cluster-tau.sh
```

The evidence directory must be new. It keeps `first/` and `second/` snapshots,
each with `inputs.json`, the copied public bonds and node list, native LFB
responses, per-node mirror logs, ordered hash arrays, summaries, and a result.
The root also has `progress.tsv`, `prefix-checks.json`, and `result.json`. An
early failure retains the files collected up to that point. The input record
includes the Git commit plus hashes of the verifier and mirror source, so an
uncommitted test run remains identifiable. Per-node summaries include the
container image ID when the node name matches a local container.

A pass requires all four nodes to have similar visible heights, nonempty
finalized outputs, the same Cordial anchor and exact ordered hash sequence at
each snapshot, no duplicate hashes, and a later output that grows by exact
prefix extension on each node. The runner does not change Docker consensus or
activate PoR weights in the native validators. A pass demonstrates the host-side
equal-weight `τ` path over this finite run.

Keep each run's evidence directory outside the repository. The script writes
both snapshots, per-node logs and ordered hashes, prefix checks, and a final
`result.json` there for comparison or attachment to a run report.
