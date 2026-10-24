/-
Cordial tiered-slashing arithmetic.

The pure definitions in this module use unbounded natural and rational
numbers.  A separate checked layer below models the fixed-width failure
conditions of the Rust implementation without making machine overflow part of
the mathematical policy.
-/

import Mathlib
import LeanVerification.Slashing

namespace CordialMiners

/-! ## Mathematical policy -/

/-- The four values that determine Cordial's tiered-slashing calculation.

Rust represents `fullSlash` implicitly as `PorConfig.scale`; keeping it
explicit here makes the equality an auditable well-formedness obligation.
-/
structure SlashingConfig where
  scale : Nat
  correlationThreshold : Nat
  basePenalty : Nat
  fullSlash : Nat
  deriving DecidableEq, Repr

/-- Preconditions validated by Rust before slashing arithmetic is evaluated. -/
abbrev SlashingConfig.WellFormed (config : SlashingConfig) : Prop :=
  0 < config.scale ∧
    config.correlationThreshold ≤ config.scale ∧
    config.basePenalty ≤ config.scale ∧
    config.fullSlash = config.scale

/-- The production defaults: a 30% boundary and a 25% base penalty. -/
def defaultSlashingConfig : SlashingConfig where
  scale := 1_000_000_000
  correlationThreshold := 300_000_000
  basePenalty := 250_000_000
  fullSlash := 1_000_000_000

theorem defaultSlashingConfig_wellFormed : defaultSlashingConfig.WellFormed := by
  norm_num [SlashingConfig.WellFormed, defaultSlashingConfig]

/-- The mathematical fraction of active weight that equivocated. -/
def correlatedRatio (equivocatingWeight totalWeight : Nat) : ℚ :=
  (equivocatingWeight : ℚ) / (totalWeight : ℚ)

/-- The configured correlation threshold interpreted as a rational fraction. -/
def correlationThresholdRatio (config : SlashingConfig) : ℚ :=
  (config.correlationThreshold : ℚ) / (config.scale : ℚ)

/-- Exact comparison used by Rust, before any integer division can round. -/
abbrev atOrBelowCorrelationThreshold
    (config : SlashingConfig) (equivocatingWeight totalWeight : Nat) : Prop :=
  equivocatingWeight * config.scale ≤
    totalWeight * config.correlationThreshold

/-- Select the base penalty at or below the threshold and a full slash above it. -/
def selectSlashPenalty
    (config : SlashingConfig) (equivocatingWeight totalWeight : Nat) : Nat :=
  if atOrBelowCorrelationThreshold config equivocatingWeight totalWeight then
    config.basePenalty
  else
    config.fullSlash

/-- Tier selection is definitionally the exact cross-product comparison; no
rounded quotient participates in the branch decision. -/
theorem selectSlashPenalty_before_division
    (config : SlashingConfig) (equivocatingWeight totalWeight : Nat) :
    selectSlashPenalty config equivocatingWeight totalWeight =
      if equivocatingWeight * config.scale ≤
          totalWeight * config.correlationThreshold then
        config.basePenalty
      else
        config.fullSlash :=
  rfl

/-- Cross multiplication expresses the ordinary rational comparison exactly. -/
theorem atOrBelowCorrelationThreshold_iff_ratio_le
    (config : SlashingConfig) (equivocatingWeight totalWeight : Nat)
    (hScale : 0 < config.scale) (hTotal : 0 < totalWeight) :
    atOrBelowCorrelationThreshold config equivocatingWeight totalWeight ↔
      correlatedRatio equivocatingWeight totalWeight ≤
        correlationThresholdRatio config := by
  rw [correlatedRatio, correlationThresholdRatio]
  rw [div_le_div_iff₀]
  · norm_cast
    simp only [atOrBelowCorrelationThreshold]
    rw [Nat.mul_comm totalWeight config.correlationThreshold]
  · exact_mod_cast hTotal
  · exact_mod_cast hScale

/-- At or below the exact threshold, the configured base tier is selected. -/
theorem selectSlashPenalty_eq_base
    (config : SlashingConfig) (equivocatingWeight totalWeight : Nat)
    (h : atOrBelowCorrelationThreshold config equivocatingWeight totalWeight) :
    selectSlashPenalty config equivocatingWeight totalWeight = config.basePenalty := by
  simp [selectSlashPenalty, h]

/-- Strictly above the exact threshold, the configured full-slash tier is selected. -/
theorem selectSlashPenalty_eq_full
    (config : SlashingConfig) (equivocatingWeight totalWeight : Nat)
    (h : ¬ atOrBelowCorrelationThreshold config equivocatingWeight totalWeight) :
    selectSlashPenalty config equivocatingWeight totalWeight = config.fullSlash := by
  simp [selectSlashPenalty, h]

/-- Under a valid configuration, the above-threshold tier is exactly the
fixed-point scale (a 100% penalty). -/
theorem selectSlashPenalty_eq_scale
    (config : SlashingConfig) (equivocatingWeight totalWeight : Nat)
    (hConfig : config.WellFormed)
    (h : ¬ atOrBelowCorrelationThreshold config equivocatingWeight totalWeight) :
    selectSlashPenalty config equivocatingWeight totalWeight = config.scale := by
  rw [selectSlashPenalty_eq_full config equivocatingWeight totalWeight h,
    hConfig.2.2.2]

/-- A valid policy never selects a penalty greater than its fixed-point scale. -/
theorem selectSlashPenalty_le_scale
    (config : SlashingConfig) (equivocatingWeight totalWeight : Nat)
    (hConfig : config.WellFormed) :
    selectSlashPenalty config equivocatingWeight totalWeight ≤ config.scale := by
  by_cases hTier : atOrBelowCorrelationThreshold config equivocatingWeight totalWeight
  · rw [selectSlashPenalty_eq_base config equivocatingWeight totalWeight hTier]
    exact hConfig.2.2.1
  · rw [selectSlashPenalty_eq_full config equivocatingWeight totalWeight hTier]
    exact hConfig.2.2.2.le

/-- Exactly 30% remains in the 25% tier under the production defaults. -/
theorem default_exact_thirty_percent :
    selectSlashPenalty defaultSlashingConfig 3 10 = 250_000_000 := by
  norm_num [selectSlashPenalty, atOrBelowCorrelationThreshold,
    defaultSlashingConfig]

/-- The smallest integer unit above 30% is a full slash, even though a rounded
division to fixed-point units could discard that unit. -/
theorem default_smallest_above_thirty_percent :
    selectSlashPenalty defaultSlashingConfig 3_000_000_001 10_000_000_000 =
      1_000_000_000 := by
  norm_num [selectSlashPenalty, atOrBelowCorrelationThreshold,
    defaultSlashingConfig]

/-- Integer division would round the preceding above-threshold example back to
the threshold.  Together with `default_smallest_above_thirty_percent`, this
records why tier selection must happen by cross multiplication first. -/
theorem rounded_ratio_loses_smallest_above_threshold :
    3_000_000_001 * defaultSlashingConfig.scale / 10_000_000_000 =
      defaultSlashingConfig.correlationThreshold := by
  norm_num [defaultSlashingConfig]

/-! ## Applying the selected penalty -/

/-- Reputation retained after applying a fixed-point penalty.  Natural-number
division is floor division, exactly as in Rust. -/
def retainedReputation
    (config : SlashingConfig) (oldWeight penalty : Nat) : Nat :=
  oldWeight * (config.scale - penalty) / config.scale

/-- A valid penalty ratio makes subtraction exact rather than underflowing. -/
theorem survivalFraction_add_penalty
    (config : SlashingConfig) (penalty : Nat) (hPenalty : penalty ≤ config.scale) :
    config.scale - penalty + penalty = config.scale :=
  Nat.sub_add_cancel hPenalty

/-- Applying a valid penalty can never increase reputation. -/
theorem retainedReputation_le
    (config : SlashingConfig) (oldWeight penalty : Nat)
    (hPenalty : penalty ≤ config.scale) :
    retainedReputation config oldWeight penalty ≤ oldWeight := by
  have hExact : config.scale - penalty + penalty = config.scale :=
    survivalFraction_add_penalty config penalty hPenalty
  have hSurvival : config.scale - penalty ≤ config.scale := by omega
  apply Nat.div_le_of_le_mul
  calc
    oldWeight * (config.scale - penalty) ≤ oldWeight * config.scale :=
      Nat.mul_le_mul_left oldWeight hSurvival
    _ = config.scale * oldWeight := Nat.mul_comm _ _

/-- A full slash retains no reputation. -/
theorem retainedReputation_full_slash
    (config : SlashingConfig) (oldWeight : Nat) :
    retainedReputation config oldWeight config.scale = 0 := by
  simp [retainedReputation]

/-- Zero reputation remains zero under every penalty. -/
theorem retainedReputation_zero
    (config : SlashingConfig) (penalty : Nat) :
    retainedReputation config 0 penalty = 0 := by
  simp [retainedReputation]

/-- The base-tier result is precisely the documented floor-divided formula. -/
theorem retainedReputation_base_penalty
    (config : SlashingConfig) (oldWeight : Nat) :
    retainedReputation config oldWeight config.basePenalty =
      oldWeight * (config.scale - config.basePenalty) / config.scale :=
  rfl

/-- The discarded remainder is the only difference from exact rational
multiplication: multiplying the retained integer by the scale cannot exceed
the pre-division numerator. -/
theorem retainedReputation_floor_bound
    (config : SlashingConfig) (oldWeight penalty : Nat) :
    retainedReputation config oldWeight penalty * config.scale ≤
      oldWeight * (config.scale - penalty) := by
  exact Nat.div_mul_le_self _ _

example : retainedReputation defaultSlashingConfig 1_000_000_000 250_000_000 =
    750_000_000 := by native_decide

example : retainedReputation defaultSlashingConfig 10 250_000_000 = 7 := by
  native_decide

/-! ## Checked fixed-width refinement -/

/-- Largest value representable by Rust's `u64`. -/
def u64Max : Nat := 2 ^ 64 - 1

/-- Largest value representable by Rust's `u128`. -/
def u128Max : Nat := 2 ^ 128 - 1

/-- Every relevant configuration value is represented by a Rust `u64`. -/
abbrev SlashingConfig.FitsU64 (config : SlashingConfig) : Prop :=
  config.scale ≤ u64Max ∧
    config.correlationThreshold ≤ u64Max ∧
    config.basePenalty ≤ u64Max ∧
    config.fullSlash ≤ u64Max

/-- Observable error classes of the Rust arithmetic. -/
inductive SlashingArithmeticError where
  | invalidConfiguration
  | overflow
  deriving DecidableEq, Repr

/-- Model `u128::checked_mul` using unbounded naturals plus an explicit bound. -/
def checkedMulU128 (left right : Nat) : Except SlashingArithmeticError Nat :=
  if left * right ≤ u128Max then
    .ok (left * right)
  else
    .error .overflow

/-- Model conversion of an unbounded result back to Rust's reputation `u64`. -/
def checkedCastU64 (value : Nat) : Except SlashingArithmeticError Nat :=
  if value ≤ u64Max then .ok value else .error .overflow

/-- Validate exactly the relevant slashing fields and their Rust widths. -/
def validateSlashingConfig
    (config : SlashingConfig) : Except SlashingArithmeticError Unit :=
  if config.WellFormed ∧ config.FitsU64 then
    .ok ()
  else
    .error .invalidConfiguration

/-- Checked executable model of `compute_slash_penalty_wide`.

The two weights model `u128` inputs because the transition aggregates active
reputation into wide intermediates before selecting a tier.
-/
def checkedSelectSlashPenalty
    (config : SlashingConfig) (equivocatingWeight totalWeight : Nat) :
    Except SlashingArithmeticError Nat := do
  validateSlashingConfig config
  if u128Max < equivocatingWeight ∨ u128Max < totalWeight then
    throw .overflow
  if totalWeight = 0 ∨ totalWeight < equivocatingWeight then
    throw .invalidConfiguration
  let equivocatingProduct ← checkedMulU128 equivocatingWeight config.scale
  let thresholdProduct ← checkedMulU128 totalWeight config.correlationThreshold
  if equivocatingProduct ≤ thresholdProduct then
    pure config.basePenalty
  else
    pure config.fullSlash

/-- Checked executable model of `apply_slash_to_reputation`. -/
def checkedRetainedReputation
    (config : SlashingConfig) (oldWeight penalty : Nat) :
    Except SlashingArithmeticError Nat := do
  validateSlashingConfig config
  if u64Max < oldWeight ∨ u64Max < penalty then
    throw .overflow
  if config.scale < penalty then
    throw .invalidConfiguration
  let product ← checkedMulU128 oldWeight (config.scale - penalty)
  checkedCastU64 (product / config.scale)

theorem checkedMulU128_eq_ok
    (left right : Nat) (hProduct : left * right ≤ u128Max) :
    checkedMulU128 left right = .ok (left * right) := by
  simp [checkedMulU128, hProduct]

theorem checkedCastU64_eq_ok (value : Nat) (hValue : value ≤ u64Max) :
    checkedCastU64 value = .ok value := by
  simp [checkedCastU64, hValue]

/-- Whenever all Rust preconditions and intermediate bounds hold, the checked
tier selector returns the pure mathematical result. -/
theorem checkedSelectSlashPenalty_refines
    (config : SlashingConfig) (equivocatingWeight totalWeight : Nat)
    (hConfig : config.WellFormed) (hConfigBound : config.FitsU64)
    (hEquivocatingBound : equivocatingWeight ≤ u128Max)
    (hTotalBound : totalWeight ≤ u128Max)
    (hTotalPositive : 0 < totalWeight)
    (hWeightOrder : equivocatingWeight ≤ totalWeight)
    (hEquivocatingProduct : equivocatingWeight * config.scale ≤ u128Max)
    (hThresholdProduct : totalWeight * config.correlationThreshold ≤ u128Max) :
    checkedSelectSlashPenalty config equivocatingWeight totalWeight =
      .ok (selectSlashPenalty config equivocatingWeight totalWeight) := by
  have hValidate : validateSlashingConfig config = .ok () := by
    simp [validateSlashingConfig, hConfig, hConfigBound]
  have hWidths : ¬(u128Max < equivocatingWeight ∨ u128Max < totalWeight) := by
    omega
  have hWeights : ¬(totalWeight = 0 ∨ totalWeight < equivocatingWeight) := by
    omega
  simp [checkedSelectSlashPenalty, hValidate, hWidths, hWeights,
    checkedMulU128, hEquivocatingProduct, hThresholdProduct,
    selectSlashPenalty, Bind.bind, Except.bind]
  change
    (if equivocatingWeight * config.scale ≤ totalWeight * config.correlationThreshold then
      Except.ok config.basePenalty else Except.ok config.fullSlash) = _
  by_cases hTier :
      equivocatingWeight * config.scale ≤ totalWeight * config.correlationThreshold <;>
    simp [hTier]

/-- Whenever all Rust preconditions and bounds hold, checked penalty
application returns the pure floor-divided mathematical result. -/
theorem checkedRetainedReputation_refines
    (config : SlashingConfig) (oldWeight penalty : Nat)
    (hConfig : config.WellFormed) (hConfigBound : config.FitsU64)
    (hOldBound : oldWeight ≤ u64Max) (hPenaltyBound : penalty ≤ u64Max)
    (hPenalty : penalty ≤ config.scale)
    (hProduct : oldWeight * (config.scale - penalty) ≤ u128Max) :
    checkedRetainedReputation config oldWeight penalty =
      .ok (retainedReputation config oldWeight penalty) := by
  have hResultBound :
      oldWeight * (config.scale - penalty) / config.scale ≤ u64Max :=
    le_trans (retainedReputation_le config oldWeight penalty hPenalty) hOldBound
  have hValidate : validateSlashingConfig config = .ok () := by
    simp [validateSlashingConfig, hConfig, hConfigBound]
  have hWidths : ¬(u64Max < oldWeight ∨ u64Max < penalty) := by omega
  have hPenaltyOrder : ¬config.scale < penalty := by omega
  simp [checkedRetainedReputation, hValidate, hWidths, hPenaltyOrder,
    checkedMulU128, hProduct, checkedCastU64, hResultBound,
    retainedReputation, Bind.bind, Except.bind]

/-! ## Executable boundary cases -/

example : checkedSelectSlashPenalty defaultSlashingConfig 0 1_000 =
    .ok 250_000_000 := by native_decide

example : checkedSelectSlashPenalty defaultSlashingConfig 29 100 =
    .ok 250_000_000 := by native_decide

example : checkedSelectSlashPenalty defaultSlashingConfig 30 100 =
    .ok 250_000_000 := by native_decide

example : checkedSelectSlashPenalty defaultSlashingConfig 3_000_000_001 10_000_000_000 =
    .ok 1_000_000_000 := by native_decide

example : checkedSelectSlashPenalty defaultSlashingConfig 100 100 =
    .ok 1_000_000_000 := by native_decide

example : checkedRetainedReputation defaultSlashingConfig 10 250_000_000 =
    .ok 7 := by native_decide

example : checkedRetainedReputation defaultSlashingConfig 0 250_000_000 =
    .ok 0 := by native_decide

example : checkedSelectSlashPenalty defaultSlashingConfig 0 0 =
    .error .invalidConfiguration := by native_decide

example : checkedSelectSlashPenalty defaultSlashingConfig 2 1 =
    .error .invalidConfiguration := by native_decide

example : checkedSelectSlashPenalty { defaultSlashingConfig with scale := 0 } 1 1 =
    .error .invalidConfiguration := by native_decide

example : checkedSelectSlashPenalty
    { defaultSlashingConfig with correlationThreshold := 1_000_000_001 } 1 10 =
    .error .invalidConfiguration := by native_decide

example : checkedSelectSlashPenalty
    { defaultSlashingConfig with basePenalty := 1_000_000_001 } 1 10 =
    .error .invalidConfiguration := by native_decide

example : checkedSelectSlashPenalty
    { defaultSlashingConfig with fullSlash := 999_999_999 } 1 10 =
    .error .invalidConfiguration := by native_decide

example : checkedRetainedReputation defaultSlashingConfig 100 1_000_000_001 =
    .error .invalidConfiguration := by native_decide

example : checkedMulU128 u128Max 2 = .error .overflow := by native_decide

example : checkedSelectSlashPenalty defaultSlashingConfig u128Max u128Max =
    .error .overflow := by native_decide

/-- Maximum `u64` inputs fit exactly in the `u128` intermediate, as in Rust. -/
example : checkedRetainedReputation
    { scale := u64Max, correlationThreshold := 0, basePenalty := 0,
      fullSlash := u64Max }
    u64Max 0 = .ok u64Max := by native_decide

end CordialMiners
