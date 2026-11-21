# Four-ingress equivocation and PoR cutover — 2026-11-21 UTC

## Result

**PASS** for a controlled four-runtime test of ingress-visible equivocation,
checkpoint PoR penalties, future weight activation, finalized Cordial ordering,
and restart. The test ran in 107.29 seconds (110.47 seconds including Cargo)
from an uncommitted worktree based on commit
`0ad4737aac0548f251dac4049f97ad6cf2f3c725` on
`demo/por-checkpoint-rating-audit`.

The test signed two distinct round-zero blocks for validator 2 and admitted
both through each of four `LiveIngress` instances. Each blocklace detected the
two-branch fork. Later signed fixture blocks carried the equivocation
authorization in a finalized checkpoint window. Each independent `PorRuntime`
used signed ratings and its own durable state, with different block delivery
orders. The test asserted equal finalized outputs and prefix preservation on
every layer, equal PoR state and weights at each activation, and recovery after
each restart.

| Active PoR round | Effective wave | Finalized hashes per observer | Common weights, validators 1–4 | Validator 2 |
| --- | ---: | ---: | --- | --- |
| 1 | 4 | 36 | `403, 403, 403, 403` | Active |
| 2 | 5 | 48 | `663, 0, 663, 663` | Ejected; 302 retained |
| 3 | 7 | 72 | `728, 0, 728, 728` | Ejected; 302 retained |
| 4 | 8 | 84 | `759, 0, 759, 759` | Ejected; 302 retained |

After the last restart, all four ingresses again held both fork branches and
reported the same finalized hash sequence. That sequence grew from 84 to 96
hashes. The test also observed a deferred epoch before its fixed activation
boundary. [result.json](result.json) records each observer's checkpoint hash,
penalty commitment, weight commitment, exact weights, and full finalized hash
sequence at every epoch, plus the post-restart sequences.

## Test setup and measured result

This is an **in-process four-ingress simulation** using real adapter, Cordial,
PoR, signing, and durable-state code. The two conflicting blocks serve as
verified proof inputs in each local Cordial blocklace. Signed fixture blocks
carry their authorization into a finalized checkpoint. The reported ordering
result compares blocks finalized **after** the fault and weight cutover.

The earlier [2026-11-03 slash bundle](../por-weighted-slash-2026-11-03/README.md)
records the preceding penalty fixture. This bundle records the ingress-injected
run. The [live four-node tau bundle](../por-tau-2026-11-03/README.md) records
honest-node ordering under genesis weights.

## Files and provenance

- [test.log](test.log): Cargo result, test duration, peak RSS, and exit status.
- [result.json](result.json): machine-readable per-observer protocol outcomes.
- [SHA256SUMS](SHA256SUMS): checksums for the files above and this README.

The tested `test_por_checkpoint_penalties.rs` had SHA-256
`c192fd67b1ae1f31f05d7203f9835d27509175acdc54485bd96c7a82b11caa12`.
The test wrote `result.json` only after every assertion succeeded. `test.log`
also reports `exit_code=0`.

## Reproduce

On a checkout containing the test changes, run from the repository root:

```sh
POR_INGRESS_EQUIVOCATION_OUTPUT=/tmp/por-ingress-equivocation.json \
  cargo test -j 1 -p cordial-f1r3node-adapter \
  --test test_por_checkpoint_penalties \
  four_runtime_epochs_preserve_finality_through_slash_and_restart -- --exact
```

The output path keeps the captured report in this bundle intact.
