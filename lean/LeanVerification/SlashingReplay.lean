/-
Versioned structural Rust-to-Lean slashing replay (#285).
The candidate is an explicit Rust blend/clamp output. Opaque proof bytes and
cryptographic commitments are outside this projection; synthetic valid forks
provide the typed witness required by SlashingRef, with explicit trusted crypto
and policy assumptions. No axiom or byte-to-authentication inference is added.
-/
import Lean.Data.Json
import LeanVerification.SlashingRef
import LeanVerification.SlashingTransitionExamples

namespace CordialMiners.SlashingReplay
open Lean SlashingRef SlashingTransitionExamples

private def fields (j : Json) (keys : List String) : Except String Unit := do
  let obj ← j.getObj?
  if obj.size != keys.length then throw "missing or unknown object fields"
  for (key, _) in obj.toList do
    if key ∉ keys then throw s!"unknown field: {key}"

private def get [FromJson α] (j : Json) (key : String) : Except String α :=
  j.getObjValAs? α key

private def number (j : Json) (key : String) : Except String Nat := do
  let n : Nat ← get j key
  if n > u64Max then throw s!"{key} exceeds u64"
  pure n

structure Snapshot where
  round : Nat
  rows : List (NodeId × ValidatorReputation)
  deriving DecidableEq, Repr

private def parseSnapshot (j : Json) : Except String Snapshot := do
  fields j ["round", "entries"]
  let round ← number j "round"
  let rows ← (← get j "entries" : List Json).mapM fun e => do
    fields e ["id", "active", "retained", "excluded"]
    let id ← number e "id"
    let active ← number e "active"
    let retained ← number e "retained"
    let excluded : Bool ← get e "excluded"
    pure (id, (⟨active, retained, excluded⟩ : ValidatorReputation))
  if !(rows.map Prod.fst).Pairwise (· < ·) then throw "entries must be strictly sorted"
  pure ⟨round, rows⟩

def Snapshot.model (s : Snapshot) : PenaltyReputationState :=
  ⟨s.round, s.rows.map Prod.fst, fun id =>
    ((s.rows.find? (fun row => row.1 == id)).map Prod.snd).getD ⟨0, 0, false⟩⟩

private def snapshot (s : PenaltyReputationState) : Snapshot :=
  ⟨s.round, s.validatorIds.map fun id => (id, s.account id)⟩

/-- Explicitly trusted structural-only test context, not live evidence admission. -/
private def structuralContext : PenaltyAuthorizationContext :=
  ⟨⟨fun _ _ => True⟩, fun _ _ _ => True⟩

private def structuralEvent (id length applicationRound : Nat) :
    AuthorizedEquivocationEvent structuralContext :=
  .ofAuthorizedPenaltyEvent (forkView id) (forkView_valid id)
    { evidence := { sampleEvidence id with proofMaterial := List.replicate length 0 }
      validEvidence := by
        apply (checkEvidence_iff _ _ _).mp
        simpa only [checkEvidence] using
          (checkEvidence_iff _ _ _).mpr (sampleEvidence_valid id)
      authenticatedEvidence := True.intro
      offender := id
      offenderBound := rfl
      offenseRound := 0
      offenseRoundBound := rfl
      applicationRound := applicationRound
      authorizationDecision := .attested id
      authorized := True.intro }

private def parseEvents (j : Json) : Except String (PenaltyTransitionEvents structuralContext) := do
  fields j ["round", "equivocations", "inactivity"]
  let round ← number j "round"
  let equivocations ← (← get j "equivocations" : List Json).mapM fun e => do
    fields e ["id", "evidence_length", "offense_round", "application_round"]
    let id ← number e "id"
    let length ← number e "evidence_length"
    -- Bound allocation while allowing the explicit oversized negative fixture.
    if length > maxPenaltyEvidenceLength + 1 then throw "evidence projection too large"
    if (← number e "offense_round") != 0 then throw "v1 only projects depth-zero forks"
    pure (structuralEvent id length (← number e "application_round"))
  let inactivity ← (← get j "inactivity" : List Json).mapM fun e => do
    fields e ["id", "missed_rounds"]
    pure (⟨← number e "id", ← number e "missed_rounds"⟩ : TransitionInactivityEvent)
  pure ⟨round, equivocations, inactivity⟩

private def parsePairs (j : Json) (key valueKey : String) :
    Except String (List (Nat × Nat)) := do
  let pairs ← (← get j key : List Json).mapM fun e => do
    fields e ["id", valueKey]
    pure (← number e "id", ← number e valueKey)
  if !(pairs.map Prod.fst).Pairwise (· < ·) then throw s!"{key} must be strictly sorted"
  pure pairs

private def category : SlashingRef.Error → String
  | .invalidConfiguration => "invalid_configuration"
  | .arithmeticOverflow => "arithmetic_overflow"
  | .invalidState | .invalidValidatorSet => "invalid_state"
  | _ => "invalid_penalty_events"

private def require (condition : Bool) (message : String) : Except String Unit :=
  if condition then .ok () else .error message

/-- Replay every observable field and preserve the full modeled snapshot on errors. -/
def replayFixture (j : Json) : Except String String := do
  fields j ["name", "config", "previous", "candidate", "ratings", "events", "expected"]
  let name : String ← get j "name"
  let cfg : Json ← get j "config"
  fields cfg ["scale", "initial", "alpha", "minimum_rating", "maximum_rating",
    "missing_entry_policy", "threshold", "base_penalty", "gamma"]
  let scale ← number cfg "scale"
  let initial ← number cfg "initial"
  let alpha ← number cfg "alpha"
  let minimum ← number cfg "minimum_rating"
  let maximum ← number cfg "maximum_rating"
  let policy : String ← get cfg "missing_entry_policy"
  require (policy == "carry_forward") "unsupported missing-entry policy"
  let config : SlashingConfig := ⟨scale, ← number cfg "threshold", ← number cfg "base_penalty", scale⟩
  let gamma ← number cfg "gamma"
  let previous ← parseSnapshot (← get j "previous")
  let candidate ← parseSnapshot (← get j "candidate")
  let events ← parseEvents (← get j "events")
  let participants ← (← get j "ratings" : List Json).mapM fun r => do
    fields r ["round", "rater", "recipient", "score", "signature"]
    require ((← number r "round") == candidate.round) "rating round mismatch"
    let rater ← number r "rater"
    let recipient ← number r "recipient"
    require (rater != recipient) "self rating outside fixture boundary"
    let score ← number r "score"
    require (minimum ≤ score && score ≤ maximum) "rating score outside fixture boundary"
    let signature : List Nat ← get r "signature"
    require (!signature.isEmpty && signature.all (· ≤ 255)) "invalid rating signature projection"
    pure [rater, recipient]
  let expected : Json ← get j "expected"
  fields expected ["status", "state", "weights", "tiers"]
  let expectedStatus : String ← get expected "status"
  let expectedState ← parseSnapshot (← get expected "state")
  let expectedWeights ← parsePairs expected "weights" "weight"
  let expectedTiers ← parsePairs expected "tiers" "penalty"
  -- PorConfig has additional preconditions outside SlashingConfig. Check the
  -- exact serialized subset before calling the shared executable reference.
  let result := if initial > scale || alpha > scale || minimum > maximum then
      Except.error SlashingRef.Error.invalidConfiguration
    else transition config gamma previous.model candidate.model events participants.flatten
  let (status, successor, tiers) ← match result with
    | .error error => pure (category error, previous.model, [])
    | .ok successor => do
      let tiers ← events.equivocations.mapM fun event => do
        match selectTier config
            (previousCorrelatedWeightFor previous.model events.equivocations.toFinset event.offender)
            (previousActiveTotal previous.model) with
        | .ok penalty => pure (event.offender, penalty)
        | .error _ => throw "accepted transition has invalid tier arithmetic"
      pure ("ok", successor, tiers.mergeSort (fun a b => a.1 ≤ b.1))
  require (status == expectedStatus) s!"{name}: status {status}, expected {expectedStatus}"
  require (decide (snapshot successor = expectedState)) s!"{name}: successor/atomicity mismatch"
  let actualWeights := successor.validatorIds.map fun id => (id, exportedActiveWeight successor id)
  require (actualWeights == expectedWeights) s!"{name}: exported weight mismatch"
  require (tiers == expectedTiers) s!"{name}: penalty tier mismatch"
  pure name

def replaySuite (j : Json) : Except String (List String) := do
  fields j ["schema_version", "boundary", "fixtures"]
  require ((← number j "schema_version") == 1) "unsupported schema version"
  let boundary : String ← get j "boundary"
  require (boundary == "cordial-por/slashing-structural-v1") "unsupported boundary"
  let fixtures : List Json ← get j "fixtures"
  require (!fixtures.isEmpty) "empty fixture suite"
  let names ← fixtures.mapM replayFixture
  require (decide (names.Pairwise (· < ·))) "fixture names must be unique and sorted"
  pure names

end CordialMiners.SlashingReplay
