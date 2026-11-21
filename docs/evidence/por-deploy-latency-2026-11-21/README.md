# Four-observer signed deploy latency, 2026-11-21

## Run

A fresh `por-latency` Docker project ran one bootstrap node and four native
validators with the documented development bootstrap wallet funded at genesis.
Four host-side PoR shadows mirrored those validators with signed checkpoint
ratings. Their round-1 gate and durable observer restart check passed. A later
checkpoint scheduling attempt reached its effective wave after that wave had
already been published; all four shadows exited with the same error captured
in `scheduling-failure.log`.

The four shadows then resumed from their existing durable directories in
observation mode. They recovered active PoR round 3 and the identical weight
commitment
`0a04b8ecdf70a2161b4ee0ee26d9dfd3f193c1247d4e30743b4750642c41db84`.
They continued mirroring and finalizing with those active weights during the
five-sample measurement. `pre-recovery-samples.json` preserves the two
successful samples collected before the scheduling failure.

## Measurement

Each sample submitted a unique, secp256k1-signed Rholang deploy through
validator 1's real gRPC `doDeploy`. The native HTTP deploy lookup mapped its
deploy ID to a containing block hash. A single host monotonic clock measured
from submitter launch until the first one-second poll where all four PoR
statuses placed that hash at the same finalized block index. The recorded
position is the deploy's containing block's place in the Cordial finalized
sequence. Each observer also reported the same active round and weight
commitment. A later independent check compared the complete finalized prefix
through each recorded block across all four observers.

| Sample | Native block first seen | Four-observer agreement | Finalized block position |
| --- | ---: | ---: | ---: |
| 1 | 5.112 s | 29.125 s | 268 |
| 2 | 4.103 s | 81.143 s | 276 |
| 3 | 2.091 s | 83.133 s | 298 |
| 4 | 6.101 s | 70.136 s | 323 |
| 5 | 4.106 s | 70.141 s | 343 |

**Result:** 5/5 accepted deploys reached the same finalized block position on
all four shadows. Median submit-to-agreement time was **70.141 seconds**;
the range was **29.125–83.133 seconds**. The nearest-rank p95 for this
five-sample set is 83.133 seconds. Median time to first observe native block
inclusion was 4.106 seconds. This small run describes the observed cluster
and fixed active round-3 projection.

`samples.json` records every deploy ID, block hash, native block number,
observer position, active PoR round, weight commitment, and per-sample timing.
`summary.json` gives the aggregates, including zero rejections and zero
timeouts. The four `final-status-validator-*.json` snapshots contain the
finalized sequences used to reproduce the full-prefix comparison for all five
samples. `SHA256SUMS` covers every file in this bundle. The
[runbook](../../por-deploy-latency.md) provides the commands.
