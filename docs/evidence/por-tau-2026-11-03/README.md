# Four-node finalized `τ` run — 2026-11-03 UTC

## Result

**PASS** for host-side, equal-weight Cordial `weighted_tau` convergence on a
fresh four-validator `f1r3node-rust` cluster. This is a finite live run, not a
general protocol proof or native PoR-weight enforcement test.
No PoR checkpoint was activated and no equivocation or slash was applied in
this run. Its four input weights came directly from the native genesis bonds.
The separate [PoR weighted slash fixture](../por-weighted-slash-2026-11-03/README.md)
exercises the `PorRuntime` weight cutover and penalty path.

| Check | First snapshot | Second snapshot |
| --- | --- | --- |
| Common mirrored target height | 23 | 26 |
| Native LFB on all four nodes | height 21, hash `12f52b5e6eea9ae489f9678c85b0b7d065214fde132b1143901c631b4ac2637a` | height 24, hash `5214b166b5f49636bbf6fe39cf7db200dd2c29b6f158f1311148e1c2ac9ddabb` |
| Cordial finalized anchor on all four mirrors | `dab12c98e6be1904ed30317a9b3b0eccc39a17175bef5f3da6e7a359f93d9750` | `81c27985a17cd3238f48c4a69c07c4229980af77bd764a4051e1f0a74f39b9f8` |
| Identical ordered hashes on all four mirrors | 80 | 92 |
| Exact prefix growth on every mirror | — | 80 → 92; all four `is_prefix=true` and `grew=true` |

The native cluster verifier first passed at finalized height 1. The two
snapshots each used the same four public genesis bonds, with weight 1 per
validator. The image ID recorded by every node is
`sha256:d996a47a49176aaa777930eebae83926f0d9202bed8b71eca36b84f91a210df7`.
The earlier `por-live-demo` containers were stopped without deleting their
volumes. This fresh `por-tau-evidence` project used new named volumes and was
stopped after the run, also without deleting volumes.

## Reproduce

The exact host-port list is in [first/nodes.txt](first/nodes.txt), and the
configured public bond table is in [first/bonds.txt](first/bonds.txt). After
starting a fresh isolated cluster and allowing it to finalize blocks, run:

```sh
CORDIAL_ORDER_NODES_FILE=/tmp/cordial-por-smoke-nodes.txt \
CORDIAL_ORDER_BONDS_FILE=docker/genesis/cordial-bonds.txt \
CORDIAL_TAU_EVIDENCE_DIR=/tmp/cordial-tau-fresh-verified-20261103 \
CORDIAL_TAU_MIN_NEW_HEIGHTS=3 \
  ./docker/scripts/verify-four-node-cluster-tau.sh
```

Use a new evidence directory for each run. The exact source commit and hashes
of the modified mirror and verifier are in each snapshot's `inputs.json`. The
runner script SHA-256 was
`2af64e1086f5118cde79553ef7c6aa990de03e0d7dfb36f11e9afd3808d3c26c`.
The run was made from an uncommitted worktree, so these source hashes matter in
addition to the Git commit.

## Evidence files

- [native-cluster-verify.log](native-cluster-verify.log): native HTTP finality
  convergence gate before the `τ` run.
- [first/inputs.json](first/inputs.json) and
  [second/inputs.json](second/inputs.json): pinned target heights, input mode,
  source hashes, and bond-file digest.
- `first/` and `second/`: each node's native LFB response, mirror log,
  ordered hash array, and summary with output digest, anchor, weight-table
  size, and image ID.
- [prefix-checks.json](prefix-checks.json): the four exact prefix and growth
  comparisons.
- [result.json](result.json): the runner's clean exit status.
- `SHA256SUMS`: integrity hashes for the preserved files.

The raw logs retain their original `/tmp/cordial-tau-fresh-verified-20261103`
paths because this directory is a copy of the completed evidence bundle.
