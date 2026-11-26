/-
Penalty-aware Proof-of-Reputation state transitions.

This module models the state-changing boundary in `cordial-por`: callers may
submit only already-authorized equivocation events, validation is performed
against the complete previous snapshot, and a successor is exposed only after
validation and arithmetic preconditions have succeeded.  Correlation tiers
are computed exclusively from previous-round active reputation.
-/

import Mathlib
import LeanVerification.SlashingArithmetic

namespace CordialMiners

/-! ## Reputation state and exported weight -/

/-- The three reputation fields stored for one validator key. -/
structure ValidatorReputation where
  activeReputation : Nat
  retainedReputation : Nat
  excluded : Bool
  deriving DecidableEq, Repr

/-- The complete penalty-relevant PoR state.

`validatorIds` is the modeled counterpart of the canonically ordered Rust
entry list.  Keeping lookup as a function makes the transition definitions
pointwise and prevents accidental insertion of a validator by a penalty.
-/
structure PenaltyReputationState where
  round : Nat
  validatorIds : List NodeId
  account : NodeId → ValidatorReputation

/-- State invariants enforced by Rust's canonical entry validation: strictly
increasing (hence unique) identities, zero active reputation for excluded keys,
and zero retained reputation for active keys. -/
def PenaltyReputationState.WellFormed (state : PenaltyReputationState) : Prop :=
  state.validatorIds.Pairwise (· < ·) ∧
    ∀ id, id ∈ state.validatorIds →
      ((state.account id).excluded = true →
        (state.account id).activeReputation = 0) ∧
      ((state.account id).excluded = false →
        (state.account id).retainedReputation = 0)

instance instDecidableWellFormed (state : PenaltyReputationState) :
    Decidable state.WellFormed := by
  unfold PenaltyReputationState.WellFormed
  infer_instance

theorem wellFormed_uniqueValidatorIds
    (state : PenaltyReputationState) (hState : state.WellFormed) :
    state.validatorIds.Nodup :=
  hState.1.nodup

/-- Consensus weight exported for an authorized identity.

Unknown and permanently excluded keys both fail closed to zero in this pure
projection.  The Rust wrapper additionally rejects missing identities and a
zero total authorized weight.
-/
def exportedActiveWeight (state : PenaltyReputationState) (id : NodeId) : Nat :=
  if id ∈ state.validatorIds then
    if (state.account id).excluded then 0 else (state.account id).activeReputation
  else
    0

theorem excluded_exportsZeroWeight
    (state : PenaltyReputationState) (id : NodeId)
    (hExcluded : (state.account id).excluded = true) :
    exportedActiveWeight state id = 0 := by
  simp [exportedActiveWeight, hExcluded]

/-! ## Authorized transition inputs -/

/-- The fixed cryptographic verifier and protocol policy used by a transition.
Evidence may come from different validated blocklace views, but a caller cannot
substitute a different verifier or policy for an individual event. -/
structure PenaltyAuthorizationContext where
  crypto : CryptographicVerificationAssumption
  policy : PenaltyAuthorizationPolicy

/-- A transition event retains the complete authorization witness from #281,
bound to every field consumed by replay. Nonempty bytes alone cannot construct
this value. The witness is a proof, so executable comparison uses only data. -/
structure AuthorizedEquivocationEvent (context : PenaltyAuthorizationContext) where
  offender : NodeId
  evidence : List UInt8
  offenseRound : Nat
  applicationRound : Nat
  authorization :
    ∃ (B : Blocklace) (hV : ValidBlocklace B)
      (source : AuthorizedPenaltyEvent B hV context.crypto context.policy),
      source.offender = offender ∧
      source.evidence.proofMaterial = evidence ∧
      source.offenseRound = offenseRound ∧
      source.applicationRound = applicationRound
  deriving DecidableEq, Repr

variable {context : PenaltyAuthorizationContext}

/-- Retain the full evidence/crypto/policy witness and bind its projections. -/
def AuthorizedEquivocationEvent.ofAuthorizedPenaltyEvent
    (B : Blocklace) (hV : ValidBlocklace B)
    (event : AuthorizedPenaltyEvent B hV context.crypto context.policy) :
    AuthorizedEquivocationEvent context where
  offender := event.offender
  evidence := event.evidence.proofMaterial
  offenseRound := event.offenseRound
  applicationRound := event.applicationRound
  authorization := ⟨B, hV, event, rfl, rfl, rfl, rfl⟩

/-- Every event accepted by the typed transition still contains a structurally
valid, authenticated, protocol-authorized source for the exact consumed data. -/
theorem transitionEvent_has_authorizedSource
    (event : AuthorizedEquivocationEvent context) :
    ∃ (B : Blocklace) (hV : ValidBlocklace B)
      (source : AuthorizedPenaltyEvent B hV context.crypto context.policy),
      source.offender = event.offender ∧
      source.evidence.proofMaterial = event.evidence ∧
      source.offenseRound = event.offenseRound ∧
      source.applicationRound = event.applicationRound :=
  event.authorization

/-- The policy witness remains bound to the exact offender, evidence bytes,
offense depth, and application round used by the transition. -/
theorem transitionEvent_policy_bound
    (event : AuthorizedEquivocationEvent context) :
    ∃ (evidence : RawEquivocationEvidence) (decision : PenaltyAuthorizationDecision),
      evidence.claimedOffender = event.offender ∧
      evidence.offenseRound = event.offenseRound ∧
      evidence.proofMaterial = event.evidence ∧
      PenaltyApplicationAuthorized context.policy decision evidence event.applicationRound := by
  obtain ⟨B, hV, source, hOffender, hBytes, hOffense, hRound⟩ := event.authorization
  refine ⟨source.evidence, source.authorizationDecision,
    source.offenderBound.symm.trans hOffender,
    source.offenseRoundBound.symm.trans hOffense, hBytes, ?_⟩
  rw [← hRound]
  exact source.authorized

/-- A policy denying authorization cannot admit any transition event, even
when an attacker can supply arbitrary nonempty proof bytes. -/
theorem deniedPolicy_noTransitionEvent
    (hDenied : ∀ decision evidence round,
      ¬ context.policy decision evidence round)
    (event : AuthorizedEquivocationEvent context) : False := by
  obtain ⟨_, _, source, _⟩ := event.authorization
  exact hDenied _ _ _ source.authorized

/-- A verifier denying authentication likewise cannot admit transition events. -/
theorem deniedAuthentication_noTransitionEvent
    (hDenied : ∀ B evidence, ¬ context.crypto.verifies B evidence)
    (event : AuthorizedEquivocationEvent context) : False := by
  obtain ⟨_, _, source, _⟩ := event.authorization
  exact hDenied _ _ source.authenticatedEvidence

/-- Inactivity is included to model cross-penalty overlap rejection.  Its
reputation decay is outside the slashing invariants proved by this module. -/
structure TransitionInactivityEvent where
  offender : NodeId
  missedRounds : Nat
  deriving DecidableEq, Repr

/-- Finalized penalty events presented for exactly one successor round. -/
structure PenaltyTransitionEvents (context : PenaltyAuthorizationContext) where
  round : Nat
  equivocations : List (AuthorizedEquivocationEvent context)
  inactivity : List TransitionInactivityEvent
  deriving DecidableEq, Repr

def allPenaltyOffenders (events : PenaltyTransitionEvents context) : List NodeId :=
  events.equivocations.map (·.offender) ++ events.inactivity.map (·.offender)

/-- All structural validation performed before any state update becomes
visible.  In particular, every offender is looked up in the previous state,
and an already-ejected key can never be penalized or reactivated. -/
def ValidPenaltyEvents
    (previous candidate : PenaltyReputationState)
    (events : PenaltyTransitionEvents context) : Prop :=
  events.round = previous.round + 1 ∧
    candidate.round = events.round ∧
    candidate.validatorIds = previous.validatorIds ∧
    (allPenaltyOffenders events).Nodup ∧
    events.equivocations.Forall (fun event =>
      event.applicationRound = events.round ∧
      event.evidence ≠ [] ∧
      event.offender ∈ previous.validatorIds ∧
      (previous.account event.offender).excluded = false) ∧
    events.inactivity.Forall (fun event =>
      event.missedRounds = 1 ∧
      event.offender ∈ previous.validatorIds ∧
      (previous.account event.offender).excluded = false)

instance instDecidableValidPenaltyEvents
    (previous candidate : PenaltyReputationState)
    (events : PenaltyTransitionEvents context) :
    Decidable (ValidPenaltyEvents previous candidate events) := by
  unfold ValidPenaltyEvents
  infer_instance

/-! ## Previous-round correlation and staged successor -/

/-- Sum of all active previous-round reputation.  Excluded entries contribute
zero, matching the Rust `filter(|entry| !entry.is_excluded)` aggregation. -/
def previousActiveTotal (state : PenaltyReputationState) : Nat :=
  ∑ id ∈ state.validatorIds.toFinset,
    if (state.account id).excluded then 0 else (state.account id).activeReputation

def isEquivocationOffender
    (events : Finset (AuthorizedEquivocationEvent context)) (id : NodeId) : Bool :=
  decide (∃ event ∈ events, event.offender = id)

/-- Correlated weight for `id`'s offense round, using only previous-round
active weights.  Valid input has one event per offender, so the outer sum has
exactly one term.  Defining it over a `Finset` also makes event order
irrelevant by construction. -/
def previousCorrelatedWeightFor
    (previous : PenaltyReputationState)
    (events : Finset (AuthorizedEquivocationEvent context))
    (id : NodeId) : Nat :=
  ∑ target ∈ events.filter (fun event => event.offender = id),
    ∑ event ∈ events.filter (fun event => event.offenseRound = target.offenseRound),
      if (previous.account event.offender).excluded then
        0
      else
        (previous.account event.offender).activeReputation

private def slashedAccount
    (config : SlashingConfig)
    (previous : PenaltyReputationState)
    (events : Finset (AuthorizedEquivocationEvent context))
    (id : NodeId) : ValidatorReputation :=
  let old := previous.account id
  let penalty := selectSlashPenalty config
    (previousCorrelatedWeightFor previous events id)
    (previousActiveTotal previous)
  { activeReputation := 0
    retainedReputation := retainedReputation config old.activeReputation penalty
    excluded := true }

/-- Pure staged successor.  Existing exclusions take priority over the new
candidate snapshot, so neither ratings nor later direct assignment can restore
an ejected key. -/
def penaltySuccessor
    (config : SlashingConfig)
    (previous candidate : PenaltyReputationState)
    (round : Nat)
    (events : Finset (AuthorizedEquivocationEvent context)) : PenaltyReputationState where
  round := round
  validatorIds := previous.validatorIds
  account := fun id =>
    if (previous.account id).excluded then
      previous.account id
    else if isEquivocationOffender events id then
      slashedAccount config previous events id
    else
      candidate.account id

/-- Preconditions for the checked fixed-width Rust slashing arithmetic.
Configuration validation is unconditional; weight aggregation and slash-only
checks run only when equivocations are present. The checked cross products are
correlated weight × scale and total weight × threshold, exactly as in Rust. -/
def PenaltyArithmeticReady
    (config : SlashingConfig)
    (previous : PenaltyReputationState)
    (events : Finset (AuthorizedEquivocationEvent context)) : Prop :=
  config.WellFormed ∧
    config.FitsU64 ∧
    (events.Nonempty →
      0 < previousActiveTotal previous ∧
      previousActiveTotal previous ≤ u128Max ∧
      previousActiveTotal previous * config.correlationThreshold ≤ u128Max ∧
      ∀ event ∈ events,
        previousCorrelatedWeightFor previous events event.offender ≤
          previousActiveTotal previous ∧
        previousCorrelatedWeightFor previous events event.offender * config.scale ≤ u128Max ∧
        (previous.account event.offender).activeReputation ≤ u64Max ∧
        (previous.account event.offender).activeReputation *
          (config.scale - selectSlashPenalty config
            (previousCorrelatedWeightFor previous events event.offender)
            (previousActiveTotal previous)) ≤ u128Max)

instance instDecidablePenaltyArithmeticReady
    (config : SlashingConfig)
    (previous : PenaltyReputationState)
    (events : Finset (AuthorizedEquivocationEvent context)) :
    Decidable (PenaltyArithmeticReady config previous events) := by
  unfold PenaltyArithmeticReady
  infer_instance

/-- Empty and inactivity-only rounds require configuration validation, but no
positive total weight or slash products. -/
theorem noEquivocations_arithmeticReady
    (config : SlashingConfig) (previous : PenaltyReputationState) :
    PenaltyArithmeticReady config previous
      (∅ : Finset (AuthorizedEquivocationEvent context)) ↔
      config.WellFormed ∧ config.FitsU64 := by
  simp [PenaltyArithmeticReady]

/-- Readiness implies success of the checked tier selector proved in #282. -/
theorem arithmeticReady_checkedSelect
    (config : SlashingConfig) (previous : PenaltyReputationState)
    (events : Finset (AuthorizedEquivocationEvent context))
    (hReady : PenaltyArithmeticReady config previous events)
    (event : AuthorizedEquivocationEvent context) (hEvent : event ∈ events) :
    checkedSelectSlashPenalty config
      (previousCorrelatedWeightFor previous events event.offender)
      (previousActiveTotal previous) =
      .ok (selectSlashPenalty config
        (previousCorrelatedWeightFor previous events event.offender)
        (previousActiveTotal previous)) := by
  have hWeights := hReady.2.2 ⟨event, hEvent⟩
  have hOffender := hWeights.2.2.2 event hEvent
  exact checkedSelectSlashPenalty_refines _ _ _ hReady.1 hReady.2.1
    (hOffender.1.trans hWeights.2.1) hWeights.2.1 hWeights.1
    hOffender.1 hOffender.2.1 hWeights.2.2.1

/-- Readiness also implies success of checked retained-reputation arithmetic. -/
theorem arithmeticReady_checkedRetained
    (config : SlashingConfig) (previous : PenaltyReputationState)
    (events : Finset (AuthorizedEquivocationEvent context))
    (hReady : PenaltyArithmeticReady config previous events)
    (event : AuthorizedEquivocationEvent context) (hEvent : event ∈ events) :
    let penalty := selectSlashPenalty config
      (previousCorrelatedWeightFor previous events event.offender)
      (previousActiveTotal previous)
    checkedRetainedReputation config
      (previous.account event.offender).activeReputation penalty =
      .ok (retainedReputation config
        (previous.account event.offender).activeReputation penalty) := by
  have hWeights := hReady.2.2 ⟨event, hEvent⟩
  have hOffender := hWeights.2.2.2 event hEvent
  have hPenalty := selectSlashPenalty_le_scale config
    (previousCorrelatedWeightFor previous events event.offender)
    (previousActiveTotal previous) hReady.1
  exact checkedRetainedReputation_refines _ _ _ hReady.1 hReady.2.1
    hOffender.2.2.1 (hPenalty.trans hReady.2.1.1) hPenalty hOffender.2.2.2

inductive PenaltyTransitionError where
  | invalidState
  | invalidEvents
  | arithmeticFailure
  deriving DecidableEq, Repr

/-- Validate both state snapshots before event validation and calculation.
No successor is exposed for a malformed previous or candidate snapshot. -/
def calculatePenaltyTransition
    (config : SlashingConfig)
    (previous candidate : PenaltyReputationState)
    (events : PenaltyTransitionEvents context) :
    Except PenaltyTransitionError PenaltyReputationState :=
  if previous.WellFormed ∧ candidate.WellFormed then
    if ValidPenaltyEvents previous candidate events then
      let eventSet := events.equivocations.toFinset
      if PenaltyArithmeticReady config previous eventSet then
        .ok (penaltySuccessor config previous candidate events.round eventSet)
      else
        .error .arithmeticFailure
    else
      .error .invalidEvents
  else
    .error .invalidState

/-- Atomic state-level commit.  Any validation or arithmetic error returns the
entire prior state, including its round, validator set, retained balances, and
exclusion flags. -/
def commitPenaltyTransition
    (config : SlashingConfig)
    (current candidate : PenaltyReputationState)
    (events : PenaltyTransitionEvents context) : PenaltyReputationState :=
  match calculatePenaltyTransition config current candidate events with
  | .ok successor => successor
  | .error _ => current

/-! ## Slashing and atomicity invariants -/

theorem slash_zeroes_activeReputation
    (config : SlashingConfig)
    (previous candidate : PenaltyReputationState)
    (round : Nat) (events : Finset (AuthorizedEquivocationEvent context))
    (id : NodeId)
    (hActive : (previous.account id).excluded = false)
    (hOffender : isEquivocationOffender events id = true) :
    ((penaltySuccessor config previous candidate round events).account id
      ).activeReputation = 0 := by
  simp [penaltySuccessor, hActive, hOffender, slashedAccount]

theorem slash_setsExcluded
    (config : SlashingConfig)
    (previous candidate : PenaltyReputationState)
    (round : Nat) (events : Finset (AuthorizedEquivocationEvent context))
    (id : NodeId)
    (hActive : (previous.account id).excluded = false)
    (hOffender : isEquivocationOffender events id = true) :
    ((penaltySuccessor config previous candidate round events).account id
      ).excluded = true := by
  simp [penaltySuccessor, hActive, hOffender, slashedAccount]

theorem isolatedSlash_setsRetainedReputation
    (config : SlashingConfig)
    (previous candidate : PenaltyReputationState)
    (round : Nat) (events : Finset (AuthorizedEquivocationEvent context))
    (id : NodeId)
    (hActive : (previous.account id).excluded = false)
    (hOffender : isEquivocationOffender events id = true)
    (hIsolated : atOrBelowCorrelationThreshold config
      (previousCorrelatedWeightFor previous events id)
      (previousActiveTotal previous)) :
    ((penaltySuccessor config previous candidate round events).account id
      ).retainedReputation =
        retainedReputation config (previous.account id).activeReputation
          config.basePenalty := by
  simp [penaltySuccessor, hActive, hOffender, slashedAccount,
    selectSlashPenalty_eq_base _ _ _ hIsolated]

theorem correlatedSlash_zeroesRetainedReputation
    (config : SlashingConfig)
    (previous candidate : PenaltyReputationState)
    (round : Nat) (events : Finset (AuthorizedEquivocationEvent context))
    (id : NodeId)
    (hConfig : config.WellFormed)
    (hActive : (previous.account id).excluded = false)
    (hOffender : isEquivocationOffender events id = true)
    (hCorrelated : ¬ atOrBelowCorrelationThreshold config
      (previousCorrelatedWeightFor previous events id)
      (previousActiveTotal previous)) :
    ((penaltySuccessor config previous candidate round events).account id
      ).retainedReputation = 0 := by
  simp [penaltySuccessor, hActive, hOffender, slashedAccount,
    selectSlashPenalty_eq_scale _ _ _ hConfig hCorrelated,
    retainedReputation_full_slash]

theorem slash_preserves_nonOffenders
    (config : SlashingConfig)
    (previous candidate : PenaltyReputationState)
    (round : Nat) (events : Finset (AuthorizedEquivocationEvent context))
    (id : NodeId)
    (hActive : (previous.account id).excluded = false)
    (hInnocent : isEquivocationOffender events id = false) :
    (penaltySuccessor config previous candidate round events).account id =
      candidate.account id := by
  simp [penaltySuccessor, hActive, hInnocent]

theorem excluded_persistent
    (config : SlashingConfig)
    (previous candidate : PenaltyReputationState)
    (round : Nat) (events : Finset (AuthorizedEquivocationEvent context))
    (id : NodeId)
    (hExcluded : (previous.account id).excluded = true) :
    (penaltySuccessor config previous candidate round events).account id =
      previous.account id := by
  simp [penaltySuccessor, hExcluded]

theorem retainedReputation_persistent
    (config : SlashingConfig)
    (previous candidate : PenaltyReputationState)
    (round : Nat) (events : Finset (AuthorizedEquivocationEvent context))
    (id : NodeId)
    (hExcluded : (previous.account id).excluded = true) :
    ((penaltySuccessor config previous candidate round events).account id
      ).retainedReputation = (previous.account id).retainedReputation := by
  rw [excluded_persistent config previous candidate round events id hExcluded]

/-- Slashing preserves the state representation invariants when both the prior
and pre-penalty candidate snapshots are well formed. -/
theorem penaltySuccessor_wellFormed
    (config : SlashingConfig)
    (previous candidate : PenaltyReputationState)
    (round : Nat) (events : Finset (AuthorizedEquivocationEvent context))
    (hPrevious : previous.WellFormed)
    (hCandidate : candidate.WellFormed)
    (hIds : candidate.validatorIds = previous.validatorIds) :
    (penaltySuccessor config previous candidate round events).WellFormed := by
  constructor
  · exact hPrevious.1
  · intro id hKnown
    change id ∈ previous.validatorIds at hKnown
    by_cases hPriorExcluded : (previous.account id).excluded = true
    · rw [excluded_persistent config previous candidate round events id hPriorExcluded]
      exact hPrevious.2 id hKnown
    · have hPriorActive : (previous.account id).excluded = false := by
        simpa using hPriorExcluded
      by_cases hOffender : isEquivocationOffender events id = true
      · simp [penaltySuccessor, hPriorActive, hOffender, slashedAccount]
      · have hInnocent : isEquivocationOffender events id = false := by
          simpa using hOffender
        rw [slash_preserves_nonOffenders config previous candidate round events id
          hPriorActive hInnocent]
        exact hCandidate.2 id (by simpa [hIds] using hKnown)

/-- A successful calculation establishes well-formedness itself; callers need
no separate state-invariant hypotheses to use this result. -/
theorem calculatePenaltyTransition_ok_wellFormed
    (config : SlashingConfig)
    (previous candidate successor : PenaltyReputationState)
    (events : PenaltyTransitionEvents context)
    (hCalculated : calculatePenaltyTransition config previous candidate events =
      .ok successor) :
    successor.WellFormed := by
  by_cases hStates : previous.WellFormed ∧ candidate.WellFormed
  · by_cases hEvents : ValidPenaltyEvents previous candidate events
    · by_cases hArithmetic : PenaltyArithmeticReady config previous events.equivocations.toFinset
      · have hSuccessor :
            penaltySuccessor config previous candidate events.round
              events.equivocations.toFinset = successor := by
          simpa [calculatePenaltyTransition, hStates, hEvents, hArithmetic] using hCalculated
        rw [← hSuccessor]
        exact penaltySuccessor_wellFormed config previous candidate events.round
          events.equivocations.toFinset hStates.1 hStates.2 hEvents.2.2.1
      · simp [calculatePenaltyTransition, hStates, hEvents, hArithmetic] at hCalculated
    · simp [calculatePenaltyTransition, hStates, hEvents] at hCalculated
  · simp [calculatePenaltyTransition, hStates] at hCalculated

/-- Every committed state remains well formed if the current state is well
formed, including when a malformed candidate is rejected. -/
theorem commitPenaltyTransition_wellFormed
    (config : SlashingConfig)
    (current candidate : PenaltyReputationState)
    (events : PenaltyTransitionEvents context)
    (hCurrent : current.WellFormed) :
    (commitPenaltyTransition config current candidate events).WellFormed := by
  cases hCalculated : calculatePenaltyTransition config current candidate events with
  | ok successor =>
    simpa [commitPenaltyTransition, hCalculated] using
      calculatePenaltyTransition_ok_wellFormed config current candidate successor events hCalculated
  | error error =>
    simpa [commitPenaltyTransition, hCalculated] using hCurrent

theorem invalidState_rejected
    (config : SlashingConfig)
    (previous candidate : PenaltyReputationState)
    (events : PenaltyTransitionEvents context)
    (hInvalid : ¬ (previous.WellFormed ∧ candidate.WellFormed)) :
    calculatePenaltyTransition config previous candidate events = .error .invalidState := by
  simp [calculatePenaltyTransition, hInvalid]

/-- Rejecting either input preserves the complete current state, even when
that current state was itself malformed. Validation never partially repairs it. -/
theorem invalidState_preserve_entireState
    (config : SlashingConfig)
    (current candidate : PenaltyReputationState)
    (events : PenaltyTransitionEvents context)
    (hInvalid : ¬ (current.WellFormed ∧ candidate.WellFormed)) :
    commitPenaltyTransition config current candidate events = current := by
  simp [commitPenaltyTransition, invalidState_rejected config current candidate events hInvalid]

theorem invalidEvents_preserve_entireState
    (config : SlashingConfig)
    (current candidate : PenaltyReputationState)
    (events : PenaltyTransitionEvents context)
    (hInvalid : ¬ ValidPenaltyEvents current candidate events) :
    commitPenaltyTransition config current candidate events = current := by
  by_cases hStates : current.WellFormed ∧ candidate.WellFormed <;>
    simp [commitPenaltyTransition, calculatePenaltyTransition, hStates, hInvalid]

theorem arithmeticFailure_preserves_entireState
    (config : SlashingConfig)
    (current candidate : PenaltyReputationState)
    (events : PenaltyTransitionEvents context)
    (hValid : ValidPenaltyEvents current candidate events)
    (hArithmetic : ¬ PenaltyArithmeticReady config current
      events.equivocations.toFinset) :
    commitPenaltyTransition config current candidate events = current := by
  by_cases hStates : current.WellFormed ∧ candidate.WellFormed <;>
    simp [commitPenaltyTransition, calculatePenaltyTransition, hStates, hValid, hArithmetic]

theorem duplicateOffender_rejected
    (previous candidate : PenaltyReputationState)
    (events : PenaltyTransitionEvents context)
    (hDuplicate : ¬ (allPenaltyOffenders events).Nodup) :
    ¬ ValidPenaltyEvents previous candidate events := by
  intro hValid
  exact hDuplicate hValid.2.2.2.1

theorem unknownOffender_rejected
    (previous candidate : PenaltyReputationState)
    (events : PenaltyTransitionEvents context)
    (event : AuthorizedEquivocationEvent context)
    (hEvent : event ∈ events.equivocations)
    (hUnknown : event.offender ∉ previous.validatorIds) :
    ¬ ValidPenaltyEvents previous candidate events := by
  intro hValid
  have hForall := hValid.2.2.2.2.1
  rw [List.forall_iff_forall_mem] at hForall
  exact hUnknown (hForall event hEvent).2.2.1

theorem emptyEquivocationEvidence_rejected
    (previous candidate : PenaltyReputationState)
    (events : PenaltyTransitionEvents context)
    (event : AuthorizedEquivocationEvent context)
    (hEvent : event ∈ events.equivocations)
    (hEmpty : event.evidence = []) :
    ¬ ValidPenaltyEvents previous candidate events := by
  intro hValid
  have hForall := hValid.2.2.2.2.1
  rw [List.forall_iff_forall_mem] at hForall
  exact (hForall event hEvent).2.1 hEmpty

theorem overlappingPenalty_rejected
    (previous candidate : PenaltyReputationState)
    (events : PenaltyTransitionEvents context)
    (id : NodeId)
    (hEquivocation : id ∈ events.equivocations.map (·.offender))
    (hInactivity : id ∈ events.inactivity.map (·.offender)) :
    ¬ ValidPenaltyEvents previous candidate events := by
  intro hValid
  have hNodup := hValid.2.2.2.1
  simp only [allPenaltyOffenders, List.nodup_append] at hNodup
  exact (hNodup.2.2 id hEquivocation id hInactivity) rfl

theorem alreadyExcluded_rejected
    (previous candidate : PenaltyReputationState)
    (events : PenaltyTransitionEvents context)
    (event : AuthorizedEquivocationEvent context)
    (hEvent : event ∈ events.equivocations)
    (hExcluded : (previous.account event.offender).excluded = true) :
    ¬ ValidPenaltyEvents previous candidate events := by
  intro hValid
  have hForall := hValid.2.2.2.2.1
  rw [List.forall_iff_forall_mem] at hForall
  have hActive := (hForall event hEvent).2.2.2
  simp [hExcluded] at hActive

/-- A valid event set is semantically unordered.  Reversing or otherwise
permuting it produces the identical complete successor state. -/
theorem eventPermutation_invariant
    (config : SlashingConfig)
    (previous candidate : PenaltyReputationState)
    (round : Nat)
    (first second : List (AuthorizedEquivocationEvent context))
    (hPermutation : first.Perm second) :
    penaltySuccessor config previous candidate round first.toFinset =
      penaltySuccessor config previous candidate round second.toFinset := by
  have hSets : first.toFinset = second.toFinset := by
    ext event
    simp only [List.mem_toFinset]
    exact hPermutation.mem_iff
  rw [hSets]

/-! ## Complete bounded reference-transition contract (#284)

The earlier slashing theorems deliberately leave inactivity decay outside
their scope. This extension specifies that stage using the same floor-divided
arithmetic, without changing the meaning of `penaltySuccessor`.
-/

/-- Rust's per-evidence byte limit and event-count limit. Wire encoding and
byte-valued node identities are outside this Nat-identity model. -/
def maxPenaltyEvidenceLength : Nat := 64 * 1024
def maxPenaltyEventCount : Nat := 1_000_000

def BoundedPenaltyEvidence (events : PenaltyTransitionEvents context) : Prop :=
  events.equivocations.length + events.inactivity.length ≤ maxPenaltyEventCount ∧
    ∀ event ∈ events.equivocations, event.evidence.length ≤ maxPenaltyEvidenceLength

instance (events : PenaltyTransitionEvents context) :
    Decidable (BoundedPenaltyEvidence events) := by
  unfold BoundedPenaltyEvidence
  infer_instance

/-- Width checks on finite input fields not already covered by the checked
slash arithmetic. Unknown account-function values are never serialized. -/
def BoundedPenaltyInputs (previous candidate : PenaltyReputationState)
    (events : PenaltyTransitionEvents context) : Prop :=
  previous.round ≤ u64Max ∧ candidate.round ≤ u64Max ∧ events.round ≤ u64Max ∧
    (∀ event ∈ events.equivocations,
      event.offenseRound ≤ u64Max ∧ event.applicationRound ≤ u64Max) ∧
    (∀ state ∈ [previous, candidate], ∀ id ∈ state.validatorIds,
      (state.account id).activeReputation ≤ u64Max ∧
      (state.account id).retainedReputation ≤ u64Max)

instance (previous candidate : PenaltyReputationState)
    (events : PenaltyTransitionEvents context) :
    Decidable (BoundedPenaltyInputs previous candidate events) := by
  unfold BoundedPenaltyInputs
  infer_instance

/-- Apply #283's event checks to the previous identity domain. The full
candidate is independently validated; this projection only removes the old
static-set requirement from event validation. -/
def penaltyValidationCandidate (previous candidate : PenaltyReputationState) :
    PenaltyReputationState :=
  { candidate with validatorIds := previous.validatorIds }

/-- A blend/clamp candidate cannot eject a new or previously active key.
Prior exclusions are restored by the successor even if the candidate omits
their flags, matching audited Rust replay. Unknown account-function values
outside the previous identity domain establish no exclusion provenance. -/
def ValidPenaltyCandidate (previous candidate : PenaltyReputationState) : Prop :=
  ∀ id ∈ candidate.validatorIds,
    (id ∉ previous.validatorIds ∨ (previous.account id).excluded = false) →
      (candidate.account id).excluded = false

instance (previous candidate : PenaltyReputationState) :
    Decidable (ValidPenaltyCandidate previous candidate) := by
  unfold ValidPenaltyCandidate
  infer_instance

/-- Blend may grow the identity domain, but cannot remove old keys or invent
an exclusion before the penalty stage. Penalty offenders must still be known
in the previous snapshot, as required by Rust's prior-entry lookup. -/
def ValidReferencePenaltyEvents (previous candidate : PenaltyReputationState)
    (events : PenaltyTransitionEvents context) : Prop :=
  previous.validatorIds ⊆ candidate.validatorIds ∧
    ValidPenaltyEvents previous (penaltyValidationCandidate previous candidate) events ∧
    ValidPenaltyCandidate previous candidate

instance (previous candidate : PenaltyReputationState)
    (events : PenaltyTransitionEvents context) :
    Decidable (ValidReferencePenaltyEvents previous candidate events) := by
  unfold ValidReferencePenaltyEvents
  infer_instance

/-- One missed round consumes previous reputation, not the blended candidate.
New candidate identities retain their candidate account and do not contribute
to previous-round correlation. Prior registered exclusions take priority. -/
def inactivityPenaltySuccessor (config : SlashingConfig) (gamma : Nat)
    (previous candidate : PenaltyReputationState)
    (events : PenaltyTransitionEvents context) : PenaltyReputationState :=
  let slashed := penaltySuccessor config previous candidate events.round
    events.equivocations.toFinset
  { slashed with
    validatorIds := candidate.validatorIds
    account := fun id =>
      if id ∈ candidate.validatorIds ∧ id ∉ previous.validatorIds then candidate.account id
      else if (previous.account id).excluded then previous.account id
      else if id ∈ events.inactivity.map (·.offender) then
        { slashed.account id with
          activeReputation := retainedReputation config (previous.account id).activeReputation gamma }
      else slashed.account id }

/-- Complete admission contract. `ratingParticipants` is the explicit input
for Rust's rule forbidding an inactive offender from rating or being rated.
Finalized inactivity decisions remain a host input, as in #283. -/
def ReferencePenaltyReady (config : SlashingConfig) (gamma : Nat)
    (previous candidate : PenaltyReputationState)
    (events : PenaltyTransitionEvents context) (ratingParticipants : List NodeId) : Prop :=
  (previous.WellFormed ∧ candidate.WellFormed) ∧
    ValidReferencePenaltyEvents previous candidate events ∧
    BoundedPenaltyEvidence events ∧
    BoundedPenaltyInputs previous candidate events ∧
    gamma ≤ config.scale ∧
    (∀ event ∈ events.inactivity,
      event.offender ∉ ratingParticipants ∧ (candidate.account event.offender).excluded = false) ∧
    PenaltyArithmeticReady config previous events.equivocations.toFinset

instance (config : SlashingConfig) (gamma : Nat)
    (previous candidate : PenaltyReputationState)
    (events : PenaltyTransitionEvents context) (ratingParticipants : List NodeId) :
    Decidable (ReferencePenaltyReady config gamma previous candidate events ratingParticipants) := by
  unfold ReferencePenaltyReady
  infer_instance

/-- Relational successor: admission plus complete state equality, expressed
fieldwise so a caller need not assume equality of account functions. This
contract mentions no executable reference checker. -/
structure ReferencePenaltyTransition (config : SlashingConfig) (gamma : Nat)
    (previous candidate : PenaltyReputationState)
    (events : PenaltyTransitionEvents context) (ratingParticipants : List NodeId)
    (successor : PenaltyReputationState) : Prop where
  ready : ReferencePenaltyReady config gamma previous candidate events ratingParticipants
  round : successor.round = events.round
  identities : successor.validatorIds = candidate.validatorIds
  accounts : ∀ id, successor.account id =
    (inactivityPenaltySuccessor config gamma previous candidate events).account id

theorem referencePenaltyTransition_iff (config : SlashingConfig) (gamma : Nat)
    (previous candidate successor : PenaltyReputationState)
    (events : PenaltyTransitionEvents context) (ratingParticipants : List NodeId) :
    ReferencePenaltyTransition config gamma previous candidate events ratingParticipants successor ↔
      ReferencePenaltyReady config gamma previous candidate events ratingParticipants ∧
      inactivityPenaltySuccessor config gamma previous candidate events = successor := by
  constructor
  · intro h
    refine ⟨h.ready, ?_⟩
    cases successor
    simp only [inactivityPenaltySuccessor, penaltySuccessor, PenaltyReputationState.mk.injEq]
    exact ⟨h.round.symm, h.identities.symm, (funext h.accounts).symm⟩
  · rintro ⟨hReady, rfl⟩
    exact ⟨hReady, rfl, rfl, fun _ => rfl⟩

theorem inactivityPenaltySuccessor_noInactivity (config : SlashingConfig) (gamma : Nat)
    (previous candidate : PenaltyReputationState)
    (events : PenaltyTransitionEvents context) (hEmpty : events.inactivity = [])
    (hIds : candidate.validatorIds = previous.validatorIds) :
    inactivityPenaltySuccessor config gamma previous candidate events =
      penaltySuccessor config previous candidate events.round events.equivocations.toFinset := by
  simp only [inactivityPenaltySuccessor, hEmpty, List.map_nil, List.not_mem_nil,
    hIds, and_not_self, ↓reduceIte]
  congr 1
  funext id
  by_cases hExcluded : (previous.account id).excluded = true <;>
    simp [penaltySuccessor, hExcluded]

end CordialMiners
