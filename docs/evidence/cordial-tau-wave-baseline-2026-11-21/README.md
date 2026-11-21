# Four-node Cordial τ wave baseline — 2026-11-21

**PASS:** A fresh native four-validator Docker cluster fed four independent
host-side Cordial mirrors. Each used the equal-weight genesis bond table
(weight 1 per validator). At three measured waves, all four mirrors emitted
the same finalized anchor, ordered-hash count, and τ sequence digest. The
probe also compared their full ordered-output prefixes when each wave was
complete.

| Cordial wave | Rounds | Finalized τ hashes | First wave observation to four-mirror agreement | Finalization spread |
| ---: | ---: | ---: | ---: | ---: |
| 10 | 30–32 | 114 | 34.878 s | 3.391 s |
| 11 | 33–35 | 125 | 37.254 s | 0.379 s |
| 12 | 36–38 | 137 | 44.313 s | 4.900 s |

Median observation-to-agreement time: **37.254 seconds**. The first wave
seen during mirror bootstrap was excluded. The four mirrors polled native
gRPC every second. The native validator image ID was
`sha256:d996a47a49176aaa777930eebae83926f0d9202bed8b71eca36b84f91a210df7`.

`inputs.json` records the Git commit, binary digest, genesis bond digest,
source digests, ports, polling interval, and full-history depth. `result.json`
records the timestamps and per-wave anchors and τ digests. The four
`validator-*.log` files preserve the timestamped raw wave events. The
[run guide](../../cordial-tau-wave-latency.md) gives the commands and timing
definition. `SHA256SUMS` covers this bundle.
