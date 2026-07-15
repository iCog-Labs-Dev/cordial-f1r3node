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

For each node, it mirrors live blocks, computes a deterministic ordering over
the mirrored recent window by default, writes the ordered block hashes to a
temporary JSON file, and compares each node against validator 1.

By default the script uses a bounded recent-height window with a trusted window
boundary. This keeps the live check practical on long-running local clusters:
blocks inside the chosen window are mirrored and predecessors older than the
window are treated as already-trusted node history. The normal live ingress path
remains strict; this boundary mode is only for the host-side convergence
verifier.

The default mode is therefore a practical convergence smoke test for the live
mirror: all four validators must expose the same recent Cordial-ordered block
view. To force the heavier finalized-fragment path, set
`CORDIAL_ORDER_WINDOW_ORDERING_FRAGMENT=false`.

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
