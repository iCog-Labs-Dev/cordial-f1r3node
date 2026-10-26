# Cordial Tiered-Slashing Arithmetic

Lean module: `LeanVerification/SlashingArithmetic.lean`

Issue: [#282 — Prove Cordial tiered-slashing arithmetic](https://github.com/iCog-Labs-Dev/cordial-f1r3node/issues/282)

## Scope and relationship to evidence

This module formalizes only the numerical policy applied after an equivocation
has crossed the evidence, authentication, and authorization boundaries in
`LeanVerification.Slashing`. It does not decide whether evidence is valid or
which validator may be penalized.

The model follows these Rust functions:

- `cordial-por/src/config.rs::PorConfig::validate` for the relevant slashing
  fields;
- `cordial-por/src/penalties.rs::compute_slash_penalty_wide` for tier
  selection;
- `cordial-por/src/penalties.rs::apply_slash_to_reputation` for retained
  reputation.

## Configuration and well-formedness

```lean
structure SlashingConfig where
  scale : Nat
  correlationThreshold : Nat
  basePenalty : Nat
  fullSlash : Nat
```

Rust stores the first three values in `PorConfig`. Its full-slash value is
implicitly `config.scale`; the Lean model exposes `fullSlash` so that this
identity is an explicit proof obligation.

```lean
abbrev SlashingConfig.WellFormed (config : SlashingConfig) : Prop :=
  0 < config.scale ∧
    config.correlationThreshold ≤ config.scale ∧
    config.basePenalty ≤ config.scale ∧
    config.fullSlash = config.scale
```

The production configuration has scale `1_000_000_000`, threshold
`300_000_000`, base penalty `250_000_000`, and full slash
`1_000_000_000`. `defaultSlashingConfig_wellFormed` proves these fields meet
the contract.

Valid runtime inputs additionally require positive total active weight and
equivocating weight no greater than total active weight. These conditions are
checked by the executable model because they apply to a calculation rather
than to the configuration itself.

## Exact tier selection

The mathematical correlated ratio and configured threshold are rational
fractions:

```lean
def correlatedRatio (equivocatingWeight totalWeight : Nat) : ℚ :=
  equivocatingWeight / totalWeight

def correlationThresholdRatio (config : SlashingConfig) : ℚ :=
  config.correlationThreshold / config.scale
```

The executable branch uses the equivalent cross-product comparison:

```lean
equivocatingWeight * config.scale ≤
  totalWeight * config.correlationThreshold
```

`atOrBelowCorrelationThreshold_iff_ratio_le` proves equivalence to the
rational comparison whenever scale and total weight are positive.
`selectSlashPenalty_before_division` records that no rounded quotient is used
to choose the tier.

The one-unit-above example explains why that ordering matters:

```text
3_000_000_001 * 1_000_000_000 / 10_000_000_000 = 300_000_000
```

Division rounds this value down to the 30% threshold. Exact cross
multiplication still recognizes it as above 30% and selects a full slash.

## Penalty application and rounding

```lean
def retainedReputation
    (config : SlashingConfig) (oldWeight penalty : Nat) : Nat :=
  oldWeight * (config.scale - penalty) / config.scale
```

`Nat` division is floor division, matching Rust integer division. For example,
applying the 25% default penalty to weight `10` retains `7`, not a fractional
`7.5` or a rounded-up `8`.

The module proves that a valid penalty cannot underflow the survival fraction,
retained reputation never exceeds the old value, a full slash retains zero,
zero old reputation remains zero, and the floor-divided numerator bounds the
result exactly as expected.

## Checked fixed-width model

The pure definitions use unbounded numbers so protocol meaning is independent
of machine representation. `checkedSelectSlashPenalty` and
`checkedRetainedReputation` separately model Rust's `u64` values, `u128`
intermediates, validation failures, checked multiplication, and checked
conversion:

```lean
inductive SlashingArithmeticError where
  | invalidConfiguration
  | overflow
```

No invalid or overflowing operation silently produces a wrapped value.
`checkedSelectSlashPenalty_refines` and
`checkedRetainedReputation_refines` prove that the checked results equal the
pure mathematical definitions whenever every Rust precondition and width
bound holds.

Executable examples cover:

- zero equivocating weight with positive total weight;
- a ratio below 30%, exactly 30%, and the smallest represented value above
  30%;
- 100% equivocating weight;
- non-divisible and zero retained-reputation inputs;
- zero total weight and equivocating weight greater than total weight;
- zero scale, out-of-range threshold/base penalty, and a full-slash value that
  differs from scale;
- penalty greater than scale;
- an overflowing `u128` multiplication;
- maximum `u64` reputation and scale values that correctly fit in a `u128`
  intermediate.

## Key theorem-to-Rust mapping

| Lean definition or theorem | Rust counterpart / obligation |
|---|---|
| `SlashingConfig.WellFormed` | Relevant checks in `PorConfig::validate`; `fullSlash = scale` reflects the implicit full penalty |
| `atOrBelowCorrelationThreshold_iff_ratio_le` | Mathematical justification for the cross multiplication in `compute_slash_penalty_wide` |
| `selectSlashPenalty_before_division` | Tier comparison happens before division, so rounding cannot change the branch |
| `selectSlashPenalty_eq_base` | At-or-below-threshold branch returns `base_slash_penalty` |
| `selectSlashPenalty_eq_scale` | Above-threshold branch returns `config.scale`, meaning 100% |
| `default_exact_thirty_percent` | Rust's `<=` boundary keeps exactly 30% in the 25% tier |
| `default_smallest_above_thirty_percent` | A minimally above-threshold input receives a full slash |
| `selectSlashPenalty_le_scale` | A validated configuration cannot produce an excessive penalty |
| `survivalFraction_add_penalty` | `scale.checked_sub(penalty_ratio)` cannot underflow for a valid penalty |
| `retainedReputation_le` | Applying a valid penalty cannot increase reputation |
| `retainedReputation_full_slash` | A penalty equal to scale retains zero |
| `retainedReputation_floor_bound` | Rust integer division has the modeled floor behavior |
| `checkedSelectSlashPenalty_refines` | Checked wide tier selection agrees with the pure policy when it succeeds |
| `checkedRetainedReputation_refines` | Checked retained-reputation arithmetic agrees with the pure formula when it succeeds |

The module introduces no axioms or admitted proofs and does not alter the Rust
penalty policy.
