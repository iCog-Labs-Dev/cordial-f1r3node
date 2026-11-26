/-
Executable regression examples for the penalty transition. The authorization
fixtures construct actual same-creator, same-depth forks in valid blocklaces.
Only the external crypto check is stipulated successful in this test context;
the protocol policy separately binds the decision and application round.
-/
import LeanVerification.SlashingTransition

namespace CordialMiners.SlashingTransitionExamples

def sampleContext : PenaltyAuthorizationContext where
  crypto := ⟨fun _ _ => True⟩
  policy := fun decision evidence round =>
    decision = .attested evidence.claimedOffender ∧ round = 1

def sampleBlock (id : NodeId) (tag : UInt8) : Block :=
  let content : BlockContent := ⟨[tag], ∅⟩
  ⟨hashContent id content, id, content, rfl⟩

def firstView (id : NodeId) : Blocklace :=
  blocklaceInsert emptyBlocklace (sampleBlock id 0)

def forkView (id : NodeId) : Blocklace :=
  blocklaceInsert (firstView id) (sampleBlock id 1)

theorem sampleBlocks_distinct (id : NodeId) :
    (sampleBlock id 0).id ≠ (sampleBlock id 1).id := by
  intro h
  have hContent := (hashInj h).2
  have hPayload := congrArg BlockContent.payload hContent
  have hTag : (0 : UInt8) = 1 := (List.cons.inj hPayload).1
  exact (by decide : (0 : UInt8) ≠ 1) hTag

theorem firstView_valid (id : NodeId) : ValidBlocklace (firstView id) := by
  apply ValidBlocklace.insert _ _ ValidBlocklace.empty
  · intro p hp
    simp [sampleBlock] at hp
  · intro h
    have hMem := Finmap.mem_keys.mp h
    simp [emptyBlocklace, Finmap.mem_def] at hMem

theorem forkView_valid (id : NodeId) : ValidBlocklace (forkView id) := by
  apply ValidBlocklace.insert _ _ (firstView_valid id)
  · intro p hp
    simp [sampleBlock] at hp
  · intro h
    rcases Finmap.mem_insert.mp (Finmap.mem_keys.mp h) with hEq | hEmpty
    · exact sampleBlocks_distinct id hEq.symm
    · simp [emptyBlocklace, Finmap.mem_def] at hEmpty

theorem first_lookup (id : NodeId) :
    (forkView id).lookup (sampleBlock id 0).id = some (sampleBlock id 0) := by
  calc
    _ = (firstView id).lookup (sampleBlock id 0).id :=
      Finmap.lookup_insert_of_ne (firstView id) (sampleBlocks_distinct id)
    _ = _ := Finmap.lookup_insert emptyBlocklace

theorem second_lookup (id : NodeId) :
    (forkView id).lookup (sampleBlock id 1).id = some (sampleBlock id 1) := by
  exact Finmap.lookup_insert _

theorem first_depth (id : NodeId) :
    blockDepth (forkView id) (forkView_valid id) (sampleBlock id 0).id = 0 := by
  unfold blockDepth
  rw [blockDepthWF_eq, first_lookup]
  simp [sampleBlock]

theorem second_depth (id : NodeId) :
    blockDepth (forkView id) (forkView_valid id) (sampleBlock id 1).id = 0 := by
  unfold blockDepth
  rw [blockDepthWF_eq, second_lookup]
  simp [sampleBlock]

def sampleEvidence (id : NodeId) : RawEquivocationEvidence :=
  ⟨id, (sampleBlock id 0).id, (sampleBlock id 1).id, 0, [1]⟩

theorem sampleEvidence_valid (id : NodeId) :
    ValidEquivocationEvidence (forkView id) (forkView_valid id) (sampleEvidence id) := by
  have hFirst : (sampleBlock id 0).id ∈ (forkView id).keys :=
    Finmap.mem_keys.mpr (Finmap.mem_insert.mpr (Or.inr
      (Finmap.mem_insert.mpr (Or.inl rfl))))
  have hSecond : (sampleBlock id 1).id ∈ (forkView id).keys :=
    Finmap.mem_keys.mpr (Finmap.mem_insert.mpr (Or.inl rfl))
  have hCreatorFirst : creatorOf (forkView id) (sampleBlock id 0).id = some id := by
    simp only [creatorOf, first_lookup]
    rfl
  have hCreatorSecond : creatorOf (forkView id) (sampleBlock id 1).id = some id := by
    simp only [creatorOf, second_lookup]
    rfl
  have hDepth := (first_depth id).trans (second_depth id).symm
  have hIncomparable := same_depth_incomparable _ _ _ _ (sampleBlocks_distinct id) hDepth
  exact
    { firstPresent := hFirst
      secondPresent := hSecond
      distinct := sampleBlocks_distinct id
      sameCreator := hCreatorFirst.trans hCreatorSecond.symm
      firstCreatorIsOffender := hCreatorFirst
      secondCreatorIsOffender := hCreatorSecond
      firstDoesNotObserveSecond := hIncomparable.1
      secondDoesNotObserveFirst := hIncomparable.2
      sameDepth := hDepth
      offenseRoundIsFirstDepth := (first_depth id).symm
      equivocation := ⟨⟨hFirst, hSecond, hCreatorFirst.trans hCreatorSecond.symm,
        sampleBlocks_distinct id, hIncomparable.1, hIncomparable.2⟩, hDepth⟩ }

def sampleAuthorizedPenalty (id : NodeId) :
    AuthorizedPenaltyEvent (forkView id) (forkView_valid id)
      sampleContext.crypto sampleContext.policy where
  evidence := sampleEvidence id
  validEvidence := sampleEvidence_valid id
  authenticatedEvidence := True.intro
  offender := id
  offenderBound := rfl
  offenseRound := 0
  offenseRoundBound := rfl
  applicationRound := 1
  authorizationDecision := .attested id
  authorized := ⟨rfl, rfl⟩

def sampleEquivocation (id : NodeId) : AuthorizedEquivocationEvent sampleContext :=
  .ofAuthorizedPenaltyEvent _ _ (sampleAuthorizedPenalty id)

/-! ## Small executable scenarios -/

private def sampleAccount (id : NodeId) : ValidatorReputation :=
  if id < 4 then ⟨100, 0, false⟩ else ⟨0, 0, false⟩

private def samplePrevious : PenaltyReputationState :=
  ⟨0, [0, 1, 2, 3], sampleAccount⟩

private def sampleCandidate : PenaltyReputationState :=
  ⟨1, [0, 1, 2, 3], sampleAccount⟩

private def sampleInactivity (id : Nat) : TransitionInactivityEvent :=
  ⟨id, 1⟩

/-- One isolated offender among four equal validators retains 75%. -/
example :
    (penaltySuccessor defaultSlashingConfig samplePrevious sampleCandidate 1
      [sampleEquivocation 0].toFinset).account 0 = ⟨0, 75, true⟩ := by
  native_decide

/-- Two same-round offenders exceed the 30% threshold and retain zero. -/
example :
    (penaltySuccessor defaultSlashingConfig samplePrevious sampleCandidate 1
      [sampleEquivocation 0, sampleEquivocation 1].toFinset).account 0 =
        ⟨0, 0, true⟩ := by
  native_decide

/-- An innocent validator is unchanged alongside several offenders. -/
example :
    (penaltySuccessor defaultSlashingConfig samplePrevious sampleCandidate 1
      [sampleEquivocation 0, sampleEquivocation 1].toFinset).account 3 =
        sampleCandidate.account 3 := by
  native_decide

/-- Duplicate equivocation offenders are rejected. -/
example : ¬ ValidPenaltyEvents samplePrevious sampleCandidate
    ⟨1, [sampleEquivocation 0, sampleEquivocation 0], []⟩ := by
  native_decide

/-- Equivocation/inactivity overlap is rejected. -/
example : ¬ ValidPenaltyEvents samplePrevious sampleCandidate
    ⟨1, [sampleEquivocation 0], [sampleInactivity 0]⟩ := by
  native_decide

/-- An unknown offender is rejected. -/
example : ¬ ValidPenaltyEvents samplePrevious sampleCandidate
    ⟨1, [sampleEquivocation 9], []⟩ := by
  native_decide

private def excludedPrevious : PenaltyReputationState :=
  { samplePrevious with
    account := fun id => if id = 0 then ⟨0, 75, true⟩ else sampleAccount id }

/-- An already-ejected offender is rejected. -/
example : ¬ ValidPenaltyEvents excludedPrevious sampleCandidate
    ⟨1, [sampleEquivocation 0], []⟩ := by
  native_decide

/-- Reversed event order is a permutation, hence has the same successor. -/
example :
    [sampleEquivocation 0, sampleEquivocation 1].Perm
      [sampleEquivocation 1, sampleEquivocation 0] := by
  exact List.Perm.swap _ _ []

/-- A later candidate cannot restore an ejected key's active weight. -/
example :
    ((penaltySuccessor defaultSlashingConfig excludedPrevious sampleCandidate 1 (∅ : Finset (AuthorizedEquivocationEvent sampleContext)))
      ).account 0 = ⟨0, 75, true⟩ := by
  native_decide

/-! ## Regression cases from PR #297 -/

/-- Nonempty bytes cannot bypass a context that denies protocol authorization. -/
example : IsEmpty (AuthorizedEquivocationEvent
    { sampleContext with policy := fun _ _ _ => False }) := by
  constructor
  intro event
  exact deniedPolicy_noTransitionEvent (by intros; exact id) event

/-- Nor can the transition replace the context's cryptographic verifier. -/
example : IsEmpty (AuthorizedEquivocationEvent
    { sampleContext with crypto := ⟨fun _ _ => False⟩ }) := by
  constructor
  intro event
  exact deniedAuthentication_noTransitionEvent (by intros; exact id) event

private def zeroPrevious : PenaltyReputationState :=
  ⟨0, [0], fun _ => ⟨0, 0, false⟩⟩

private def zeroCandidate : PenaltyReputationState :=
  { zeroPrevious with round := 1 }

/-- An empty penalty round over zero weight advances normally. -/
example :
    (commitPenaltyTransition defaultSlashingConfig zeroPrevious zeroCandidate
      (⟨1, [], []⟩ : PenaltyTransitionEvents sampleContext)).round = 1 := by
  native_decide

/-- Inactivity-only input does not trigger slash-only total-weight checks. -/
example :
    (commitPenaltyTransition defaultSlashingConfig zeroPrevious zeroCandidate
      (⟨1, [], [sampleInactivity 0]⟩ : PenaltyTransitionEvents sampleContext)).round = 1 := by
  native_decide

/-- Zero total weight must still reject a round with an actual slash. -/
example :
    (match calculatePenaltyTransition defaultSlashingConfig zeroPrevious zeroCandidate
      ⟨1, [sampleEquivocation 0], []⟩ with
    | .error .arithmeticFailure => true
    | _ => false) = true := by
  native_decide

private def wideConfig : SlashingConfig := ⟨u64Max, 0, 0, u64Max⟩

private def widePrevious : PenaltyReputationState :=
  ⟨0, [0, 1], fun _ => ⟨u64Max, 0, false⟩⟩

private def wideCandidate : PenaltyReputationState :=
  { widePrevious with round := 1 }

/-- The old total × scale condition would overflow for this valid input. -/
example : u128Max < previousActiveTotal widePrevious * wideConfig.scale := by
  native_decide

/-- One maximum-u64 offender among two maximum-u64 validators is accepted:
the correlated product fits and the zero-threshold product is zero. -/
example : PenaltyArithmeticReady wideConfig widePrevious
    [sampleEquivocation 0].toFinset := by
  native_decide

example : checkedSelectSlashPenalty wideConfig u64Max (2 * u64Max) = .ok u64Max := by
  native_decide

example :
    (commitPenaltyTransition wideConfig widePrevious wideCandidate
      ⟨1, [sampleEquivocation 0], []⟩).account 0 = ⟨0, 0, true⟩ := by
  native_decide

/-- In contrast, two correlated maximum-u64 offenders really overflow the
correlated product, so the complete state must remain uncommitted. -/
example :
    (match calculatePenaltyTransition wideConfig widePrevious wideCandidate
      ⟨1, [sampleEquivocation 0, sampleEquivocation 1], []⟩ with
    | .error .arithmeticFailure => true
    | _ => false) = true := by
  native_decide

example :
    (commitPenaltyTransition wideConfig widePrevious wideCandidate
      ⟨1, [sampleEquivocation 0, sampleEquivocation 1], []⟩).round = 0 := by
  native_decide

/-- Configuration validation still runs when the equivocation set is empty. -/
example : ¬ PenaltyArithmeticReady
    { defaultSlashingConfig with scale := 0 } zeroPrevious
    (∅ : Finset (AuthorizedEquivocationEvent sampleContext)) := by
  native_decide

example : samplePrevious.WellFormed := by
  norm_num [PenaltyReputationState.WellFormed, samplePrevious, sampleAccount]

/-- Canonical state validation rejects unsorted and duplicate identities. -/
example : ¬ ({ samplePrevious with validatorIds := [1, 0] }).WellFormed := by
  norm_num [PenaltyReputationState.WellFormed]

example : ¬ ({ samplePrevious with validatorIds := [0, 0] }).WellFormed := by
  norm_num [PenaltyReputationState.WellFormed]

/-- Active keys may not carry retained capital. -/
example : ¬ (⟨0, [0], fun _ => ⟨100, 75, false⟩⟩ : PenaltyReputationState).WellFormed := by
  simp [PenaltyReputationState.WellFormed]

/-- Excluded keys may not carry active reputation. -/
example : ¬ (⟨0, [0], fun _ => ⟨100, 75, true⟩⟩ : PenaltyReputationState).WellFormed := by
  simp [PenaltyReputationState.WellFormed]

/-! ## State validation at the executable transition boundary -/

private def emptyEvents : PenaltyTransitionEvents sampleContext := ⟨1, [], []⟩

private def malformedStates : List PenaltyReputationState :=
  [{ samplePrevious with validatorIds := [1, 0] },
   { samplePrevious with validatorIds := [0, 0] },
   { samplePrevious with account := fun _ => ⟨100, 75, false⟩ },
   { samplePrevious with account := fun _ => ⟨100, 75, true⟩ }]

private def rejectsInvalidState
    (previous candidate : PenaltyReputationState) : Bool :=
  match calculatePenaltyTransition defaultSlashingConfig previous candidate emptyEvents with
  | .error .invalidState => true
  | _ => false

/-- Malformed previous snapshots are rejected even when both snapshots share
the same identities and all empty-event checks would otherwise pass. -/
example : malformedStates.all (fun previous =>
    rejectsInvalidState previous { previous with round := 1 }) = true := by
  native_decide

/-- Validate previous snapshots independently of a well-formed candidate. -/
example : malformedStates.all (fun previous =>
    rejectsInvalidState previous sampleCandidate) = true := by
  native_decide

/-- Validate candidate snapshots independently of a well-formed previous state. -/
example : malformedStates.all (fun candidate =>
    rejectsInvalidState samplePrevious { candidate with round := 1 }) = true := by
  native_decide

private def retainedActiveCandidate : PenaltyReputationState :=
  { sampleCandidate with account := fun _ => ⟨100, 75, false⟩ }

/-- This is the review's counterexample: event validation and arithmetic alone
accept it, but state validation must reject its active retained balance. -/
example : ValidPenaltyEvents samplePrevious retainedActiveCandidate emptyEvents := by
  native_decide

example : PenaltyArithmeticReady defaultSlashingConfig samplePrevious
    (∅ : Finset (AuthorizedEquivocationEvent sampleContext)) := by
  native_decide

example : rejectsInvalidState samplePrevious retainedActiveCandidate = true := by
  native_decide

/-- The entire valid current state survives rejection of that candidate. -/
example : commitPenaltyTransition defaultSlashingConfig samplePrevious
    retainedActiveCandidate emptyEvents = samplePrevious := by
  apply invalidState_preserve_entireState
  native_decide

/-- A malformed current state is also returned intact on rejection. -/
example : commitPenaltyTransition defaultSlashingConfig
    { retainedActiveCandidate with round := 0 } sampleCandidate emptyEvents =
      { retainedActiveCandidate with round := 0 } := by
  apply invalidState_preserve_entireState
  native_decide

end CordialMiners.SlashingTransitionExamples
