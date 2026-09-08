/-
Executable Cordial Miners reference predicates.

These functions are finite computations over `observeSet` and `Finset`.
The theorems in this file connect each computation to the Prop-level KR1,
KR2, and KR4 definitions used by the safety proofs.
-/

import LeanVerification.Finality
import LeanVerification.WCert

namespace CordialMiners.CMRef

open CordialMiners
open scoped BigOperators

/-! ### Executable KR3 certificate accumulator -/

/-- Fold the proved KR3 `WCert.accept` operation over reported validators.
Repeated validators are deduplicated by `WCert.accept`, so the resulting
running weight cannot be inflated by duplicate certificate members. -/
def buildWCert (bonds : NodeId → ℕ) (members : List NodeId) : WCert NodeId :=
  members.foldl (WCert.accept bonds) WCert.empty

/-- The executable certificate accumulator's cached weight is exactly the
weight of its accepted validator set. -/
theorem buildWCert_invariant (bonds : NodeId → ℕ) (members : List NodeId) :
    (buildWCert bonds members).Invariant bonds := by
  unfold buildWCert
  have preserve : ∀ (certificate : WCert NodeId), certificate.Invariant bonds →
      (members.foldl (WCert.accept bonds) certificate).Invariant bonds := by
    intro certificate hCertificate
    induction members generalizing certificate with
    | nil => simpa
    | cons member tail ih =>
        simp only [List.foldl_cons]
        exact ih (WCert.accept bonds certificate member)
          (WCert.accept_invariant bonds certificate member hCertificate)
  exact preserve WCert.empty (WCert.empty_invariant bonds)

/-- Executable observation membership. -/
def checkObserves (B : Blocklace) (hV : ValidBlocklace B) (a b : BlockId) : Bool :=
  decide (b ∈ observeSet B hV a)

theorem checkObserves_iff (B : Blocklace) (hV : ValidBlocklace B)
    (a b : BlockId) (hb : b ∈ B.keys) :
    checkObserves B hV a b = true ↔ Observes B a b := by
  simp [checkObserves, observeSet_equiv B hV a b hb]

/-- Executable KR2 same-round equivocation predicate. -/
def checkEquivocation (B : Blocklace) (hV : ValidBlocklace B)
    (left right : BlockId) : Bool :=
  decide (left ∈ B.keys) && decide (right ∈ B.keys) &&
  decide (creatorOf B left = creatorOf B right) && decide (left ≠ right) &&
  !checkObserves B hV left right && !checkObserves B hV right left &&
  decide (blockDepth B hV left = blockDepth B hV right)

theorem checkEquivocation_iff (B : Blocklace) (hV : ValidBlocklace B)
    (left right : BlockId) :
    checkEquivocation B hV left right = true ↔ Equivocation B hV left right := by
  unfold checkEquivocation Equivocation Fork
  by_cases hl : left ∈ B.keys <;> by_cases hr : right ∈ B.keys
  · (simp [hl, hr, checkObserves, observeSet_equiv B hV left right hr,
      observeSet_equiv B hV right left hl]; tauto)
  · simp [hl, hr]
  · simp [hl]
  · simp [hl]

/-- Finite form of `Approves`: all quantified competitors are drawn from the
actual finite blocklace domain and every observation is decided by
`observeSet`. -/
def ApprovesFinite (B : Blocklace) (hV : ValidBlocklace B)
    (approver target : BlockId) : Prop :=
  target ∈ observeSet B hV approver ∧
  ∀ other ∈ B.keys,
    creatorOf B other = creatorOf B target → other ≠ target →
    other ∈ observeSet B hV approver →
      ¬ (¬ target ∈ observeSet B hV other ∧
         ¬ other ∈ observeSet B hV target)

instance (B : Blocklace) (hV : ValidBlocklace B) (approver target : BlockId) :
    Decidable (ApprovesFinite B hV approver target) := by
  unfold ApprovesFinite
  infer_instance

def checkApproves (B : Blocklace) (hV : ValidBlocklace B)
    (approver target : BlockId) : Bool :=
  decide (ApprovesFinite B hV approver target)

private theorem creator_some_of_mem (B : Blocklace) {b : BlockId} (hb : b ∈ B.keys) :
    ∃ v, creatorOf B b = some v := by
  cases hlookup : B.lookup b with
  | none =>
      exact absurd hb (by
        intro hmem
        have := Finmap.mem_iff.mp (Finmap.mem_keys.mp hmem)
        simp [hlookup] at this)
  | some blk => exact ⟨blk.creator, by simp [creatorOf, hlookup]⟩

private theorem mem_of_creator_eq_some (B : Blocklace) {b : BlockId} {v : NodeId}
    (h : creatorOf B b = some v) : b ∈ B.keys := by
  cases hb : B.lookup b with
  | none => simp [creatorOf, hb] at h
  | some blk => exact Finmap.mem_keys.mpr (Finmap.mem_iff.mpr ⟨blk, hb⟩)

theorem checkApproves_iff (B : Blocklace) (hV : ValidBlocklace B)
    (approver target : BlockId) (_ha : approver ∈ B.keys) (ht : target ∈ B.keys) :
    checkApproves B hV approver target = true ↔ Approves B approver target := by
  simp only [checkApproves, decide_eq_true_eq]
  unfold Approves VouchesFor ApprovesFinite
  constructor
  · rintro ⟨hobs, hclean⟩
    refine ⟨(observeSet_equiv B hV approver target ht).mp hobs, ?_⟩
    intro other hcreator hne haOther hfork
    obtain ⟨v, htargetCreator⟩ := creator_some_of_mem B ht
    have hotherCreator : creatorOf B other = some v := hcreator.trans htargetCreator
    have ho : other ∈ B.keys := mem_of_creator_eq_some B hotherCreator
    apply hclean other ho hcreator hne
      ((observeSet_equiv B hV approver other ho).mpr haOther)
    exact ⟨
      fun h => hfork.2 ((observeSet_equiv B hV other target ht).mp h),
      fun h => hfork.1 ((observeSet_equiv B hV target other ho).mp h)⟩
  · rintro ⟨hobs, hclean⟩
    refine ⟨(observeSet_equiv B hV approver target ht).mpr hobs, ?_⟩
    intro other ho hcreator hne haOther hfork
    apply hclean other hcreator hne
      ((observeSet_equiv B hV approver other ho).mp haOther)
    exact ⟨
      fun h => hfork.2 ((observeSet_equiv B hV target other ho).mpr h),
      fun h => hfork.1 ((observeSet_equiv B hV other target ht).mpr h)⟩

/-- Validators for which `r` observes an approving block for `target`. -/
def approvingCreators (validators : Finset NodeId) (B : Blocklace)
    (hV : ValidBlocklace B) (r target : BlockId) : Finset NodeId :=
  validators.filter fun v =>
    ∃ a ∈ B.keys,
      a ∈ observeSet B hV r ∧ creatorOf B a = some v ∧
      ApprovesFinite B hV a target

def checkStrictTwoThirds (bonds : NodeId → ℕ) (validators S : Finset NodeId) : Bool :=
  decide (3 * bondOf bonds S > 2 * bondOf bonds validators)

theorem checkStrictTwoThirds_iff (bonds : NodeId → ℕ)
    (validators S : Finset NodeId) :
    checkStrictTwoThirds bonds validators S = true ↔
      StrictTwoThirdsMaj bonds validators S := by
  simp [checkStrictTwoThirds, StrictTwoThirdsMaj]

def checkRatifies (bonds : NodeId → ℕ) (validators : Finset NodeId)
    (B : Blocklace) (hV : ValidBlocklace B) (r target : BlockId) : Bool :=
  checkStrictTwoThirds bonds validators (approvingCreators validators B hV r target)

theorem checkRatifies_iff (bonds : NodeId → ℕ) (validators : Finset NodeId)
    (B : Blocklace) (hV : ValidBlocklace B) (r target : BlockId)
    (_hr : r ∈ B.keys) (ht : target ∈ B.keys) :
    checkRatifies bonds validators B hV r target = true ↔
      Ratifies bonds validators B r target := by
  unfold checkRatifies
  rw [checkStrictTwoThirds_iff]
  constructor
  · intro hmaj
    refine ⟨approvingCreators validators B hV r target, Finset.filter_subset _ _, ?_, hmaj⟩
    intro v hv
    rw [approvingCreators, Finset.mem_filter] at hv
    obtain ⟨_, ⟨a, ha, har, hacreator, haapprove⟩⟩ := hv
    exact ⟨a, (observeSet_equiv B hV r a ha).mp har, hacreator,
      (checkApproves_iff B hV a target ha ht).mp (by simpa [checkApproves] using haapprove)⟩
  · rintro ⟨S, hSsub, hSwit, hSmaj⟩
    have hsubset : S ⊆ approvingCreators validators B hV r target := by
      intro v hv
      have hvValidator := hSsub hv
      obtain ⟨a, har, hacreator, haapprove⟩ := hSwit v hv
      have ha : a ∈ B.keys := mem_of_creator_eq_some B hacreator
      rw [approvingCreators, Finset.mem_filter]
      exact ⟨hvValidator, a, ha, (observeSet_equiv B hV r a ha).mpr har,
        hacreator, by
          simpa [checkApproves] using
            (checkApproves_iff B hV a target ha ht).mpr haapprove⟩
    unfold StrictTwoThirdsMaj at *
    have hweight := bondOf_mono bonds hsubset
    omega

/-- Creators represented in `witness` by a block that ratifies `target`. -/
def ratifyingCreators (bonds : NodeId → ℕ) (validators : Finset NodeId)
    (B : Blocklace) (hV : ValidBlocklace B) (witness : Finset BlockId)
    (target : BlockId) : Finset NodeId :=
  validators.filter fun v =>
    ∃ r ∈ witness, r ∈ B.keys ∧ creatorOf B r = some v ∧
      checkRatifies bonds validators B hV r target = true

def checkSuperRatifies (bonds : NodeId → ℕ) (validators : Finset NodeId)
    (B : Blocklace) (hV : ValidBlocklace B) (witness : Finset BlockId)
    (target : BlockId) : Bool :=
  checkStrictTwoThirds bonds validators
    (ratifyingCreators bonds validators B hV witness target)

theorem checkSuperRatifies_iff (bonds : NodeId → ℕ) (validators : Finset NodeId)
    (B : Blocklace) (hV : ValidBlocklace B) (witness : Finset BlockId)
    (target : BlockId) (hW : witness ⊆ B.keys) (ht : target ∈ B.keys) :
    checkSuperRatifies bonds validators B hV witness target = true ↔
      SuperRatifies bonds validators B witness target := by
  unfold checkSuperRatifies
  rw [checkStrictTwoThirds_iff]
  constructor
  · intro hmaj
    refine ⟨ratifyingCreators bonds validators B hV witness target,
      Finset.filter_subset _ _, ?_, hmaj⟩
    intro v hv
    rw [ratifyingCreators, Finset.mem_filter] at hv
    obtain ⟨_, r, hrW, hrB, hrcreator, hrat⟩ := hv
    exact ⟨r, hrW, hrcreator,
      (checkRatifies_iff bonds validators B hV r target hrB ht).mp hrat⟩
  · rintro ⟨R, hRsub, hRwit, hRmaj⟩
    have hsubset : R ⊆ ratifyingCreators bonds validators B hV witness target := by
      intro v hv
      obtain ⟨r, hrW, hrcreator, hrat⟩ := hRwit v hv
      have hrB := hW hrW
      rw [ratifyingCreators, Finset.mem_filter]
      exact ⟨hRsub hv, r, hrW, hrB, hrcreator,
        (checkRatifies_iff bonds validators B hV r target hrB ht).mpr hrat⟩
    unfold StrictTwoThirdsMaj at *
    have hweight := bondOf_mono bonds hsubset
    omega

def waveWitness (B : Blocklace) (hV : ValidBlocklace B)
    (wave wavelength : Nat) : Finset BlockId :=
  B.keys.filter fun block =>
    leaderRoundOfWave wave wavelength ≤ blockDepth B hV block ∧
    blockDepth B hV block ≤ lastRoundOfWave wave wavelength

/-- Independently decide the KR4 `FinalLeader` predicate. -/
def checkFinal (bonds : NodeId → ℕ) (validators : Finset NodeId)
    (B : Blocklace) (hV : ValidBlocklace B) (wave wavelength : Nat)
    (sel : Nat → Option NodeId) (candidate : BlockId) : Bool :=
  decide (candidate ∈ B.keys) &&
  decide (creatorOf B candidate = sel wave) &&
  decide (blockDepth B hV candidate = leaderRoundOfWave wave wavelength) &&
  checkSuperRatifies bonds validators B hV (waveWitness B hV wave wavelength) candidate

/-- CMRef finality soundness and completeness. This is the independent oracle
used by trace replay; it does not mention the Rust decision field. -/
theorem checkFinal_iff (bonds : NodeId → ℕ) (validators : Finset NodeId)
    (B : Blocklace) (hV : ValidBlocklace B) (wave wavelength : Nat)
    (sel : Nat → Option NodeId) (candidate : BlockId) :
    checkFinal bonds validators B hV wave wavelength sel candidate = true ↔
      FinalLeader bonds validators B hV wave wavelength sel candidate := by
  unfold checkFinal FinalLeader leaderBlocksOfWave
  simp only [Bool.and_eq_true, decide_eq_true_eq]
  constructor
  · rintro ⟨⟨⟨hmem, hcreator⟩, hdepth⟩, hsuper⟩
    have hW : waveWitness B hV wave wavelength ⊆ B.keys := Finset.filter_subset _ _
    refine ⟨⟨hmem, hcreator, hdepth⟩, waveWitness B hV wave wavelength, ?_, ?_⟩
    · intro block hb
      rw [waveWitness, Finset.mem_filter] at hb
      exact ⟨hb.1, hb.2.1, hb.2.2⟩
    · exact (checkSuperRatifies_iff bonds validators B hV _ candidate hW hmem).mp hsuper
  · rintro ⟨⟨hmem, hcreator, hdepth⟩, witness, hW, hsuper⟩
    have hsubset : witness ⊆ waveWitness B hV wave wavelength := by
      intro block hb
      have hbounds := hW block hb
      rw [waveWitness, Finset.mem_filter]
      exact ⟨hbounds.1, hbounds.2.1, hbounds.2.2⟩
    have hcanonical : SuperRatifies bonds validators B
        (waveWitness B hV wave wavelength) candidate :=
      superRatifies_mono bonds validators B witness
        (waveWitness B hV wave wavelength) candidate hsubset hsuper
    have hDomain : waveWitness B hV wave wavelength ⊆ B.keys := Finset.filter_subset _ _
    exact ⟨⟨⟨hmem, hcreator⟩, hdepth⟩,
      (checkSuperRatifies_iff bonds validators B hV _ candidate hDomain hmem).mpr hcanonical⟩

/-! ### Executable τ reference model

This independently implements the algorithm described informally in
`Ordering.lean`: select the latest finalized leader, recursively include the
latest earlier final leader ratified by it, then deterministically
topologically sort the newly approved blocks. `key` is the canonical Rust
block-hash encoding used only to break ties; every consensus predicate is
decided over the formal `Blocklace` above. The component predicates have
soundness lemmas; the composite ordering algorithm does NOT yet have a
refinement theorem to KR4's opaque `tau`. This remains an acceptance gap. -/

private def insertByKey (key : BlockId → String) (value : BlockId) :
    List BlockId → List BlockId
  | [] => [value]
  | head :: tail =>
      if key value < key head then value :: head :: tail
      else head :: insertByKey key value tail

private def sortByKey (key : BlockId → String) (values : List BlockId) : List BlockId :=
  values.foldl (fun sorted value => insertByKey key value sorted) []

private def topoOrder (B : Blocklace) (key : BlockId → String)
    (subset : List BlockId) : Except String (List BlockId) :=
  let subset := subset.eraseDups
  let rec loop (remaining ordered : List BlockId) (fuel : Nat) : Except String (List BlockId) :=
    match fuel with
    | 0 => if remaining.isEmpty then pure ordered else throw "tau subset contains a cycle"
    | fuel + 1 =>
        if remaining.isEmpty then pure ordered
        else
          let ready := remaining.filter fun block =>
            match B.lookup block with
            | none => false
            | some value => decide
                (value.content.predecessors ∩ subset.toFinset ⊆ ordered.toFinset)
          match (sortByKey key ready).head? with
          | none => throw "tau subset contains a cycle or unknown block"
          | some next => loop (remaining.erase next) (ordered ++ [next]) fuel
  loop subset [] (subset.length + 1)

private def finalLeaderAt? (bonds : NodeId → ℕ) (validators : Finset NodeId)
    (B : Blocklace) (hV : ValidBlocklace B) (wavelength : Nat)
    (sel : Nat → Option NodeId) (domain : List BlockId) (wave : Nat) : Option BlockId := do
  let candidates := domain.filter fun block =>
    decide (creatorOf B block = sel wave) &&
    decide (blockDepth B hV block = leaderRoundOfWave wave wavelength)
  let candidate ← match candidates with
    | [block] => some block
    | _ => none
  if checkFinal bonds validators B hV wave wavelength sel candidate then
    some candidate
  else none

private def tauFromLeader (bonds : NodeId → ℕ) (validators : Finset NodeId)
    (B : Blocklace) (hV : ValidBlocklace B) (wavelength : Nat)
    (sel : Nat → Option NodeId) (domain : List BlockId)
    (key : BlockId → String) :
    Nat → Nat → BlockId → Except String (List BlockId)
  | 0, _, _ => throw "tau recursion fuel exhausted"
  | fuel + 1, wave, leader => do
      let mut orderPrefix : List BlockId := []
      if wave > 0 then
        for priorWave in (List.range wave).reverse do
          if let some previous :=
              finalLeaderAt? bonds validators B hV wavelength sel domain priorWave then
            if checkRatifies bonds validators B hV leader previous then
              orderPrefix ← tauFromLeader bonds validators B hV wavelength sel domain key
                fuel priorWave previous
              break
      let approved := domain.filter fun block =>
        checkApproves B hV leader block && !orderPrefix.contains block
      let suffix ← topoOrder B key approved
      pure (orderPrefix ++ suffix)

/-- Independently compute a canonical τ reference result through the same
KR2/KR4 executable predicates used by finality replay. Equivalence to the
abstract KR4 `tau` has not been proved. -/
def computeTau (bonds : NodeId → ℕ) (validators : Finset NodeId)
    (B : Blocklace) (hV : ValidBlocklace B) (wavelength : Nat)
    (sel : Nat → Option NodeId) (domain : List BlockId)
    (key : BlockId → String)
    (throughWave : Nat) : Except String (BlockId × List BlockId) := do
  let waves := (List.range (throughWave + 1)).reverse
  let latest ← match waves.findSome?
      (finalLeaderAt? bonds validators B hV wavelength sel domain) with
    | some block => pure block
    | none => throw "Lean found no finalized leader for tau"
  let latestWave := waveOfRound (blockDepth B hV latest) wavelength
  let order ← tauFromLeader bonds validators B hV wavelength sel domain key
    (throughWave + 2) latestWave latest
  pure (latest, order)

end CordialMiners.CMRef
