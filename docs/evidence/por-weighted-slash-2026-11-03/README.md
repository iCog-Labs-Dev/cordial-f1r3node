# PoR weights, Cordial finality, and slashing fixture — 2026-11-03 UTC

These are **controlled fixture tests**, run against the current adapter and PoR
code. They test protocol transitions and the `PorRuntime` to Cordial handoff.
They do not claim that the live Docker validators produced an equivocation or
enforced PoR weights.

## Executed gates

| Gate | Result | Evidence |
| --- | --- | --- |
| Four durable `PorRuntime` observers, four future weight epochs, checkpoint-two equivocation penalty, identical finalized Cordial outputs, prefix preservation, and restart | PASS, 1 test in 94.19 s | [four-runtime-test.log](four-runtime-test.log); assertions in [test_por_checkpoint_penalties.rs](../../../crates/cordial-f1r3node-adapter/tests/test_por_checkpoint_penalties.rs) (`four_runtime_epochs_preserve_finality_through_slash_and_restart`) |
| Four independent PoR observers audit the same signed checkpoint ratings and finalized equivocation authorization, agree on the penalty and weights, then recover | PASS, 1 test in 1.20 s | [four-observer-test.log](four-observer-test.log), [four-observer-slash.json](four-observer-slash.json) |
| Future weight preview retains the published Cordial prefix and rejects invalid cutovers or membership changes | PASS, 2 tests | [future-weight-test.log](future-weight-test.log); [test_future_weight_epoch.rs](../../../crates/cordial-f1r3node-adapter/tests/test_future_weight_epoch.rs) |

The four-observer artifact records validator 2 as the offender. The first PoR
checkpoint gives weights `911, 0, 1000, 1000` for validators 1–4, a retained
reputation of `750` for validator 2, and the same checkpoint and weights on all
four observers. The next checkpoint keeps validator 2 at zero. The
four-runtime test checks that its weight is zero after the slash while Cordial
finalized output remains equal across observers and grows only by prefix
extension after future epoch activation and restart.

## Reproduce

```sh
cargo test -j 1 -p cordial-f1r3node-adapter --test test_por_checkpoint_penalties \
  four_runtime_epochs_preserve_finality_through_slash_and_restart -- --exact
POR_DEMO_OUTPUT=/tmp/four-observer-slash.json \
  cargo test -j 1 -p cordial-f1r3node-adapter --test test_por_checkpoint_penalties \
  four_observer_checkpoint_penalty_demo -- --exact
cargo test -j 1 -p cordial-f1r3node-adapter --test test_future_weight_epoch
```

Test source SHA-256 values at the time of this run:

- `test_por_checkpoint_penalties.rs`: `6e5545ebae645d90c9aa7ce0fff2ee9e08325c12668ccf272934114e8bee262f`
- `test_future_weight_epoch.rs`: `2fcc9284b94ae108ef4b3b884f6b54a802c13cba9a77413fcc29b5dc3c65c9ed`

The [live four-node `τ` run](../por-tau-2026-11-03/README.md) used equal genesis
weights and no PoR slash. A combined **live** gate still needs four keyed PoR
observers to activate an unequal checkpoint weight table, demonstrate a new
Cordial finalized suffix under its effective wave, and then exercise a
finalized equivocation authorization. The current Docker smoke does not inject
that live fault, and host-chain `SlashSystemDeploy` is a separate path.
