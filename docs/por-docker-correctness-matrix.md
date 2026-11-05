# What the four-node PoR Docker run verifies

This is the claim-to-evidence contract for a four-validator demonstration. A
passing run establishes the listed observations for that run and its stated
window. It is not a proof of consensus safety or liveness for every execution.

The Docker image runs the native `f1r3node-rust` node. Cordial Miners orders a
mirror of its blocks in this repository's host-side adapter; keyed PoR shadow
observers derive and activate reputation weights in their own Cordial view.
PoR keeps Cordial's authorized validator set and supplies weights; it does not
select a committee. Neither observer changes the running validators' native
consensus weights.

## Terms and current checks

| Property | Current check | What a pass establishes | What it does not establish |
| --- | --- | --- | --- |
| Connected validator set | `verify-four-node-cluster.sh` | Four reachable nodes report the expected network, none explicitly reports `isValidator=false`, and none reports both zero peers and zero nodes. | Exact bonded membership: a missing `isValidator` field is accepted as `unknown`. Container health alone says even less. |
| Native finalized-state convergence | `verify-four-node-cluster.sh` | Each node reaches a fixed post-genesis target height, finalizes the reference anchor, and reports the same sorted **set** of finalized `(height, hash)` pairs over at most the last ten heights through that target. | A total order, agreement over the full chain, or continued progress after the snapshot. Same-height HTTP enumeration order is deliberately ignored. |
| Cordial window-order convergence | `verify-four-node-cluster-ordering.sh` | Four host-side mirrors export the same nonempty `xsort` hash sequence for a bounded recent-height window pinned to one target height. | Finalized `τ` agreement, full-history total ordering, append-only order across time, or enforcement inside the Docker validators. The default uses a trusted old-history boundary, a window fragment, and `--skip-http-compare`. |
| PoR observer agreement | `verify-four-node-por-shadow.py` | Four keyed observers share a finalized hash prefix and the same active PoR round, checkpoint, weight commitment, exact weight map, ejection set, retained reputation, and effective wave. Optional gates require distinct active weights, an ejected zero-weight key, and finalized progress under the effective wave. | That a candidate has activated unless `--min-por-round` requires its round; that the gate injects a fault; or that native validators use those weights. |
| Restart safety and progress | `verify-four-node-por-shadow.py` | After an observer restart, and a validator restart unless skipped, the old finalized prefix is retained, an unchanged round retains its projection, and every observer finalizes at least `--min-new-finalized-blocks` more blocks. | Unlimited recovery or liveness under arbitrary faults. The harness stops its observer processes when it exits. |
| PoR-weighted Cordial finality through a slash | `four_runtime_epochs_preserve_finality_through_slash_and_restart` in `test_por_checkpoint_penalties.rs` | A controlled four-runtime fixture applies signed checkpoint rounds, authorizes an equivocation penalty, activates four future weight epochs, checks identical Cordial finalized outputs and prefix preservation, and recovers after restart. | A live Docker equivocation or host-chain slash deploy. The live `τ` script uses genesis bonds, not these PoR epochs. |
| Fault response and numerical convergence | `test_por_checkpoint_penalties.rs`, `test_future_weight_epoch.rs`, and the focused PoR tests | Controlled fixtures check signed rating admission, deterministic checkpoint closure, equivocation penalties, ejection, retained reputation, future weight epochs, restart recovery, and the stated finite-round convergence bounds. | A live attack on the Docker validators, host-chain slash execution, or general collusion resistance. The dashboard's event replay is from these fixtures. |

Here, **node convergence** means agreement on finalized block identities at a
fixed point. **Total ordering** would additionally require the same complete,
append-only sequence of protocol-approved blocks across nodes and time. The
current ordering smoke checks equality of a bounded Cordial sequence at one
point; its PASS line should be called *window-order convergence*.

## Acceptance gates for a protocol correctness run

Keep the gates separate in the run report. A later gate cannot turn an earlier
failure into a pass.

1. **Input and finality:** Record the image digest, Git commit, authorized keys,
   pinned source height, and finalized `(height, hash)` set from each validator.
   Require the same finalized set and anchor over the tested window.
2. **Cordial ordering:** Mirror the same bounded DAG input on all four nodes.
   Require identical ordered hashes, no duplicates, every block approved by
   the tested final-leader chain exactly once, and predecessor-before-successor
   order when both blocks are emitted. Take a second
   snapshot after new finality and require the first ordered output to remain
   an exact prefix. The current ordering script checks sequence equality and
   nonemptiness only; the remaining assertions need implementation.
3. **PoR ratings and weights:** Require a complete signed checkpoint from every
   eligible rater, identical audited checkpoint and weight commitments, and an
   unchanged authorized key set. A zero-weight ejected key remains in that set.
   Require an active round beyond genesis, and record the scheduled effective
   wave and actual activation wave. A candidate alone is insufficient.
4. **Future-only activation:** Record the finalized prefix before activation.
   Require it to remain byte-for-byte identical after the future epoch becomes
   active; then require at least one newly finalized wave under the new shadow
   weights. A separate unequal-weight fixture must demonstrate that the
   weighted Cordial result can differ from the equal-weight result.
5. **Recovery and faults:** Restart an observer and a validator, require the
   previous prefix and active projection to recover, and require new finality.
   Run isolated and correlated equivocation fixtures separately and record
   exact penalty, retained balance, ejection, and zero-weight outcomes. These
   fixtures do not claim a live Docker slash.

A reproducible report should mark each gate `PASS`, `FAIL`, or `NOT RUN` and
retain its inputs, counts, hashes, commitments, weights, before/after snapshots,
and timeout. Set a unique evidence directory for each run; generated evidence
stays outside Git and can be attached to the run report.

## Current run commands

For the base Compose ports, the existing checks are:

```sh
just demo-cordial-four-node-cluster-verify
just demo-cordial-four-node-cluster-ordering
```

The keyed PoR smoke command and isolated Compose ports are in
[the shadow-runner guide](por-shadow-runner.md#parallel-keyed-por-smoke).
When using that port override, set `CORDIAL_ORDER_NODES_FILE` for the ordering
script; its default ports are `51401` through `54401`, while the isolated smoke
uses `61001` through `64001`. Use `--min-por-round 1` or higher to require an
activated PoR round, and `--min-new-finalized-blocks 1` or higher to require
post-restart progress.

To probe finalized `τ` with the same four-node script, set
`CORDIAL_ORDER_WINDOW_ORDERING_FRAGMENT=false` and leave
`CORDIAL_ORDER_FRAGMENT_ONLY=true`. This calls the finalized weighted ordering
export. A nonempty output is required; matching empty outputs are a failure.
The bounded trusted window is still an incomplete-history input, so a passing
probe would not establish full-history or prefix-over-time correctness.

For a two-snapshot, full-history host-side `τ` run with retained evidence, use
[`verify-four-node-cluster-tau.sh`](../docker/scripts/verify-four-node-cluster-tau.sh)
on a fresh cluster. It reads the configured public genesis bonds and requires
nonempty, equal `τ` outputs and exact prefix growth. The command and evidence
layout are in the [ordering guide](cordial-miners/integration/13-four-node-ordering-convergence.md#finalized-%CF%84-evidence-run).
The `τ` script and the `PorRuntime` slash fixture are separate checks. Run the
fixture with:

```sh
cargo test -j 1 -p cordial-f1r3node-adapter --test test_por_checkpoint_penalties \
  four_runtime_epochs_preserve_finality_through_slash_and_restart -- --exact
```

Neither check is a combined live fault-injection run.
