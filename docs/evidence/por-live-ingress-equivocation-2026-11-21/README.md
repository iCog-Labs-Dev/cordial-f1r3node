# Live-source Cordial ingress equivocation probe — 2026-11-21 UTC

## Result

**PASS** on the running `por-smoke` four-validator Docker cluster. The ignored
manual test read a bounded recent window over each validator's gRPC endpoint,
admitted every returned native block through its own `LiveIngress`, and checked
that the live block's four demo validator keys were bonded. It then injected
the same two locally signed, conflicting Cordial blocks under **bonded
validator 2's** key into all four ingresses. Each ingress contained both
branches at round 1, and the full equivocation proof verified against its
blocklace.

| Observer | Native blocks admitted | Latest native height | Injected branches verified |
| --- | ---: | ---: | ---: |
| 1 | 41 | 37206 | 2 |
| 2 | 41 | 23998 | 2 |
| 3 | 44 | 37205 | 2 |
| 4 | 41 | 37206 | 2 |

[result.json](result.json) contains the exact native hashes, source URLs,
bond check, and proof outcome for each observer. [test.log](test.log) records
the passing run. The source test is
[`test_live_ingress_equivocation.rs`](../../../crates/cordial-f1r3node-adapter/tests/test_live_ingress_equivocation.rs),
SHA-256 `0754eb96f9d66ac337e753d5f8a41d77ebace97e8663e70533d2a59b5ae78dad`.
The worktree was based on commit `0ad4737aac0548f251dac4049f97ad6cf2f3c725`.

## Test setup and measured result

The four **real** native validators supplied the block windows. The test
injected two locally signed conflicting Cordial blocks into four in-process
ingresses. The synthetic fork forms an independent Cordial branch. Each
ingress admitted its native window, detected both injected branches, and
verified the equivocation proof. The separate
[four-runtime fixture](../por-ingress-equivocation-2026-11-21/README.md)
exercises PoR penalties, weight cutover, finalized ordering, and restart with
controlled finalized authorization carriers.

The captured native heights were 37206, 23998, 37205, and 37206. The ingress
uses the trusted bounded-window path and treats earlier predecessors as a
window boundary.

## Reproduce

With the `por-smoke` Docker validators reachable on gRPC ports `61001`,
`62001`, `63001`, and `64001`, run from a checkout containing the test:

```sh
POR_LIVE_INGRESS_EQUIVOCATION_OUTPUT=/tmp/por-live-ingress-equivocation.json \
  cargo test -j 1 -p cordial-f1r3node-adapter \
  --test test_live_ingress_equivocation \
  four_live_sources_detect_injected_cordial_fork -- --ignored --exact --nocapture
```

The current run was captured with the JSON output pointed at this bundle and
stdout/stderr saved as `test.log`. The report is written only after all four
ingresses pass. Source heights and hashes will change on another run.
