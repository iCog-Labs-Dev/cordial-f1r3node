/-
Correctness of the executable τ ordering model (Issue #244).

The executable path is:

  final leaders → previous-final recursion → fresh approved fragments
                → deterministic Kahn sort → τ output

This module proves the observable contracts of that path. Prefix preservation
is deliberately stated for `ValidAppend`, not arbitrary `SubBlocklace`: a DAG
extension may introduce an earlier competing leader or change an old approved
fragment. `ValidAppend` records the protocol facts that rule those cases out.
-/

import LeanVerification.Ordering

namespace CordialMiners

open CMRef

/-! ## Certified topological sort -/

/-- The Boolean topological validator is exactly the semantic contract. -/
theorem checkTopological_iff (parents : ParentMap) (subset order : List BlockId) :
    checkTopological parents subset order = true ↔
      IsTopologicalOrder parents subset order := by
  simp only [checkTopological, IsTopologicalOrder, Bool.and_eq_true,
    decide_eq_true_eq]
  tauto

/-- Every successful `xsortRef` result satisfies the topological contract. -/
theorem xsortRef_correct (parents : ParentMap) (key : BlockId → String)
    (subset order : List BlockId)
    (h : xsortRef parents key subset = .ok order) :
    IsTopologicalOrder parents subset order := by
  unfold xsortRef at h
  cases hCandidate : topoOrder parents key subset with
  | error message =>
      simp only [hCandidate] at h
      change (Except.error message : Except String (List BlockId)) =
        Except.ok order at h
      exact nomatch h
  | ok candidate =>
      simp only [hCandidate] at h
      change (if checkTopological parents subset candidate = true then
        Except.ok candidate else
        Except.error "internal xsort result failed its topological contract") =
          Except.ok order at h
      split at h
      · rename_i hChecked
        have hEq : candidate = order := Except.ok.inj h
        subst order
        exact (checkTopological_iff parents subset candidate).mp hChecked
      · simp at h

/-- A cyclic selected graph is one admitting no topological order. -/
def CyclicOn (parents : ParentMap) (subset : List BlockId) : Prop :=
  ∀ order, ¬ IsTopologicalOrder parents subset order

/-- `xsortRef` rejects every cyclic selected graph. -/
theorem xsortRef_rejects_cycle (parents : ParentMap) (key : BlockId → String)
    (subset : List BlockId) (hCycle : CyclicOn parents subset) :
    ∃ message, xsortRef parents key subset = .error message := by
  cases hRun : xsortRef parents key subset with
  | error message => exact ⟨message, rfl⟩
  | ok order => exact False.elim (hCycle order (xsortRef_correct _ _ _ _ hRun))

/-- Extract the edge-order fact checked by `predecessorsBefore`. -/
theorem predecessorsBefore_sound (parents : ParentMap) (subset order : List BlockId)
    (child predecessor : BlockId) (predecessors : List BlockId)
    (hChecked : predecessorsBefore parents subset order = true)
    (hChild : child ∈ order) (hParents : parents child = some predecessors)
    (hPredecessor : predecessor ∈ predecessors)
    (hSelected : predecessor ∈ subset) :
    order.idxOf predecessor < order.idxOf child := by
  unfold predecessorsBefore at hChecked
  have hChildChecked := List.all_eq_true.mp hChecked child hChild
  rw [hParents] at hChildChecked
  have hPredecessorChecked :=
    List.all_eq_true.mp hChildChecked predecessor hPredecessor
  have hContains : subset.contains predecessor = true := by simpa using hSelected
  rw [hContains] at hPredecessorChecked
  simp at hPredecessorChecked
  exact hPredecessorChecked

/-! ## τ safety properties -/

/-- A successful τ computation satisfies the whole-output topological
contract. This is independent of the Rust-reported ordering. -/
theorem tauRef_correct
    (bonds : NodeId → ℕ) (validators : Finset NodeId)
    (B : Blocklace) (hV : ValidBlocklace B)
    (wavelength : ℕ) (sel : ℕ → Option NodeId)
    (domain : List BlockId) (key : BlockId → String) (throughWave : ℕ)
    (order : List BlockId)
    (h : tauRef bonds validators B hV wavelength sel domain key throughWave =
      .ok order) :
    IsTopologicalOrder (blockParents B) order order := by
  unfold tauRef at h
  cases hPlan : computeTauPlan bonds validators B hV wavelength sel domain key
      throughWave with
  | error message =>
      simp only [hPlan] at h
      change (Except.error message : Except String (List BlockId)) =
        Except.ok order at h
      exact nomatch h
  | ok result =>
      rcases result with ⟨latest, epochs⟩
      simp only [hPlan] at h
      change (if checkTopological (blockParents B) (flattenEpochs epochs)
          (flattenEpochs epochs) = true then
        Except.ok (flattenEpochs epochs) else
        Except.error "tau output failed duplicate/predecessor validation") =
          Except.ok order at h
      split at h
      · rename_i hChecked
        have hEq : flattenEpochs epochs = order := Except.ok.inj h
        subst order
        exact (checkTopological_iff _ _ _).mp hChecked
      · simp at h

/-- τ is deterministic: successful calls with identical inputs cannot return
different sequences. -/
theorem tauRef_deterministic
    (bonds : NodeId → ℕ) (validators : Finset NodeId)
    (B : Blocklace) (hV : ValidBlocklace B)
    (wavelength : ℕ) (sel : ℕ → Option NodeId)
    (domain : List BlockId) (key : BlockId → String) (throughWave : ℕ)
    {left right : List BlockId}
    (hLeft : tauRef bonds validators B hV wavelength sel domain key throughWave =
      .ok left)
    (hRight : tauRef bonds validators B hV wavelength sel domain key throughWave =
      .ok right) :
    left = right := by
  rw [hLeft] at hRight
  exact Except.ok.inj hRight

/-- τ never returns a duplicate block identity. -/
theorem tauRef_nodup
    (bonds : NodeId → ℕ) (validators : Finset NodeId)
    (B : Blocklace) (hV : ValidBlocklace B)
    (wavelength : ℕ) (sel : ℕ → Option NodeId)
    (domain : List BlockId) (key : BlockId → String) (throughWave : ℕ)
    (order : List BlockId)
    (h : tauRef bonds validators B hV wavelength sel domain key throughWave =
      .ok order) :
    order.Nodup :=
  (tauRef_correct bonds validators B hV wavelength sel domain key throughWave order h).1

/-- If a block and one of its direct predecessors are both emitted, the
predecessor has a strictly smaller output position. -/
theorem tauRef_predecessor_ordered
    (bonds : NodeId → ℕ) (validators : Finset NodeId)
    (B : Blocklace) (hV : ValidBlocklace B)
    (wavelength : ℕ) (sel : ℕ → Option NodeId)
    (domain : List BlockId) (key : BlockId → String) (throughWave : ℕ)
    (order : List BlockId) (child predecessor : BlockId) (block : Block)
    (hRun : tauRef bonds validators B hV wavelength sel domain key throughWave =
      .ok order)
    (hChild : child ∈ order) (hPredecessorOut : predecessor ∈ order)
    (hLookup : B.lookup child = some block)
    (hPredecessor : predecessor ∈ block.content.predecessors) :
    order.idxOf predecessor < order.idxOf child := by
  have hCorrect := tauRef_correct bonds validators B hV wavelength sel domain key
    throughWave order hRun
  have hParents : blockParents B child =
      some (block.content.predecessors.sort (· ≤ ·)) := by
    simp [blockParents, hLookup]
  apply predecessorsBefore_sound (blockParents B) order order child predecessor
      (block.content.predecessors.sort (· ≤ ·)) hCorrect.2.2 hChild hParents
  · simpa using hPredecessor
  · exact hPredecessorOut

/-! ## Prefix preservation under a valid append -/

/-- Earlier finality decisions are unchanged through the old replay horizon. -/
def StableEarlierFinality
    (bonds : NodeId → ℕ) (validators : Finset NodeId)
    (B B' : Blocklace) (hV : ValidBlocklace B) (hV' : ValidBlocklace B')
    (wavelength : ℕ) (sel : ℕ → Option NodeId)
    (domain domain' : List BlockId) (throughWave : ℕ) : Prop :=
  ∀ wave, wave ≤ throughWave →
    finalLeaderAt? bonds validators B hV wavelength sel domain wave =
      finalLeaderAt? bonds validators B' hV' wavelength sel domain' wave

/-- The exact conditions under which a blocklace extension is safe for an
already emitted τ prefix.

The same `bonds`, validator universe, wavelength, leader selector, and key are
indices of this proposition, so they cannot change between the two runs.
Besides append-only DAG growth, the old finality decisions must be stable and
the new epoch plan must extend the old one. An epoch contains both its final
leader and its freshly approved, topologically sorted fragment, so
`epochsPrefix` explicitly rules out retroactive leader/fragment changes; it
does not merely assume the flattened output conclusion. -/
structure ValidAppendEvidence
    (bonds : NodeId → ℕ) (validators : Finset NodeId)
    (B B' : Blocklace) (hV : ValidBlocklace B) (hV' : ValidBlocklace B')
    (wavelength : ℕ) (sel : ℕ → Option NodeId)
    (domain domain' : List BlockId) (key : BlockId → String)
    (throughWave throughWave' : ℕ) where
  subBlocklace : SubBlocklace B B'
  domainExtension : ∀ block, block ∈ domain → block ∈ domain'
  horizonExtension : throughWave ≤ throughWave'
  stableFinality : StableEarlierFinality bonds validators B B' hV hV'
    wavelength sel domain domain' throughWave
  oldLatest : Option BlockId
  newLatest : Option BlockId
  oldEpochs : List TauEpoch
  newEpochs : List TauEpoch
  oldPlan : computeTauPlan bonds validators B hV wavelength sel domain key
    throughWave = .ok (oldLatest, oldEpochs)
  newPlan : computeTauPlan bonds validators B' hV' wavelength sel domain' key
    throughWave' = .ok (newLatest, newEpochs)
  epochsPrefix : List.IsPrefix oldEpochs newEpochs
  newOutputChecked : checkTopological (blockParents B')
    (flattenEpochs newEpochs) (flattenEpochs newEpochs) = true

/-- Proposition-level valid-append predicate. The witness is hidden so callers
state a protocol condition, not a particular internal plan representation. -/
def ValidAppend
    (bonds : NodeId → ℕ) (validators : Finset NodeId)
    (B B' : Blocklace) (hV : ValidBlocklace B) (hV' : ValidBlocklace B')
    (wavelength : ℕ) (sel : ℕ → Option NodeId)
    (domain domain' : List BlockId) (key : BlockId → String)
    (throughWave throughWave' : ℕ) : Prop :=
  Nonempty (ValidAppendEvidence bonds validators B B' hV hV' wavelength sel
    domain domain' key throughWave throughWave')

theorem flattenEpochs_append (left right : List TauEpoch) :
    flattenEpochs (left ++ right) = flattenEpochs left ++ flattenEpochs right := by
  simp [flattenEpochs, List.flatMap_append]

/-- Flattening an epoch-prefix produces an output-prefix. -/
theorem flattenEpochs_prefix {left right : List TauEpoch}
    (h : List.IsPrefix left right) :
    List.IsPrefix (flattenEpochs left) (flattenEpochs right) := by
  rcases h with ⟨suffix, rfl⟩
  exact ⟨flattenEpochs suffix, (flattenEpochs_append left suffix).symm⟩

/-- A checked plan is exactly the successful `tauRef` output. -/
theorem tauRef_eq_of_plan
    (bonds : NodeId → ℕ) (validators : Finset NodeId)
    (B : Blocklace) (hV : ValidBlocklace B)
    (wavelength : ℕ) (sel : ℕ → Option NodeId)
    (domain : List BlockId) (key : BlockId → String) (throughWave : ℕ)
    (latest : Option BlockId) (epochs : List TauEpoch)
    (hPlan : computeTauPlan bonds validators B hV wavelength sel domain key
      throughWave = .ok (latest, epochs))
    (hChecked : checkTopological (blockParents B)
      (flattenEpochs epochs) (flattenEpochs epochs) = true) :
    tauRef bonds validators B hV wavelength sel domain key throughWave =
      .ok (flattenEpochs epochs) := by
  unfold tauRef
  rw [hPlan]
  change (if checkTopological (blockParents B) (flattenEpochs epochs)
      (flattenEpochs epochs) = true then
    Except.ok (flattenEpochs epochs) else
    Except.error "tau output failed duplicate/predecessor validation") = _
  rw [hChecked]
  rfl

/-- **τ prefix theorem.** A valid append preserves the old output and only
adds a suffix. The hypotheses explicitly include append-only blocklace
growth, unchanged earlier finality, unchanged ordering parameters, and a
stable old leader/approved-fragment epoch plan. -/
theorem tauRef_prefix_of_validAppend
    (bonds : NodeId → ℕ) (validators : Finset NodeId)
    (B B' : Blocklace) (hV : ValidBlocklace B) (hV' : ValidBlocklace B')
    (wavelength : ℕ) (sel : ℕ → Option NodeId)
    (domain domain' : List BlockId) (key : BlockId → String)
    (throughWave throughWave' : ℕ)
    (hAppend : ValidAppend bonds validators B B' hV hV' wavelength sel
      domain domain' key throughWave throughWave')
    (oldOutput : List BlockId)
    (hOld : tauRef bonds validators B hV wavelength sel domain key throughWave =
      .ok oldOutput) :
    ∃ suffix,
      tauRef bonds validators B' hV' wavelength sel domain' key throughWave' =
        .ok (oldOutput ++ suffix) := by
  rcases hAppend with ⟨append⟩
  have hOldShape : oldOutput = flattenEpochs append.oldEpochs := by
    unfold tauRef at hOld
    rw [append.oldPlan] at hOld
    change (if checkTopological (blockParents B) (flattenEpochs append.oldEpochs)
        (flattenEpochs append.oldEpochs) = true then
      Except.ok (flattenEpochs append.oldEpochs) else
      Except.error "tau output failed duplicate/predecessor validation") =
        Except.ok oldOutput at hOld
    split at hOld
    · exact (Except.ok.inj hOld).symm
    · simp at hOld
  rcases append.epochsPrefix with ⟨laterEpochs, hEpochs⟩
  subst oldOutput
  refine ⟨flattenEpochs laterEpochs, ?_⟩
  have hNew := tauRef_eq_of_plan bonds validators B' hV' wavelength sel domain'
    key throughWave' append.newLatest append.newEpochs append.newPlan
    append.newOutputChecked
  rw [← hEpochs, flattenEpochs_append] at hNew
  exact hNew

/-! ## Small executable regression examples -/

private def noParents : ParentMap := fun _ => some []

private def chainParents : ParentMap
  | 0 => some []
  | 1 => some [0]
  | _ => none

private def cyclicParents : ParentMap
  | 0 => some [1]
  | 1 => some [0]
  | _ => none

/-- Canonical-key tie breaking is deterministic for simultaneously ready
vertices. -/
example : xsortRef noParents toString [2, 1] = .ok [1, 2] := by
  native_decide

/-- A predecessor is emitted before its child even when the input is reversed. -/
example : xsortRef chainParents toString [1, 0] = .ok [0, 1] := by
  native_decide

/-- Kahn's algorithm rejects a two-vertex cycle. -/
example : xsortRef cyclicParents toString [0, 1] =
    .error "tau subset contains a cycle or unknown block" := by
  native_decide

/-! The remaining examples construct an actual valid two-wave blocklace. With
one validator of weight one, each chain block is the selected final leader for
its wave; the second leader ratifies the first. -/

namespace TwoWaveExample

def validator : NodeId := 10

def content0 : BlockContent := { payload := [0], predecessors := ∅ }
def block0 : Block :=
  { id := hashContent validator content0, creator := validator,
    content := content0, id_eq := rfl }

def content1 : BlockContent := { payload := [1], predecessors := {block0.id} }
def block1 : Block :=
  { id := hashContent validator content1, creator := validator,
    content := content1, id_eq := rfl }

def B0 : Blocklace := emptyBlocklace
def B1 : Blocklace := blocklaceInsert B0 block0
def B2 : Blocklace := blocklaceInsert B1 block1

theorem block0_ne_block1 : block0.id ≠ block1.id := by
  intro h
  have hContent := (hashInj h).2
  simp [block0, content0, content1] at hContent

theorem B0_valid : ValidBlocklace B0 := ValidBlocklace.empty

theorem B1_valid : ValidBlocklace B1 := by
  apply ValidBlocklace.insert B0 block0 B0_valid
  · intro predecessor hPredecessor
    simp [block0, content0] at hPredecessor
  · intro hMember
    have h := Finmap.mem_keys.mp hMember
    simp [B0, emptyBlocklace, Finmap.mem_def] at h

theorem block0_mem_B1 : block0.id ∈ B1.keys := by
  exact Finmap.mem_keys.mpr (Finmap.mem_insert.mpr (Or.inl rfl))

theorem B2_valid : ValidBlocklace B2 := by
  apply ValidBlocklace.insert B1 block1 B1_valid
  · intro predecessor hPredecessor
    have hEq : predecessor = block0.id := by
      simpa [block1, content1] using hPredecessor
    simpa [hEq] using block0_mem_B1
  · intro hMember
    rcases Finmap.mem_insert.mp (Finmap.mem_keys.mp hMember) with hEq | hOld
    · exact block0_ne_block1 hEq.symm
    · have hEmpty : block1.id ∈ B0.keys := Finmap.mem_keys.mpr hOld
      have h := Finmap.mem_keys.mp hEmpty
      simp [B0, emptyBlocklace, Finmap.mem_def] at h

def bonds (node : NodeId) : ℕ := if node = validator then 1 else 0
def validators : Finset NodeId := {validator}
def selectLeader (_wave : ℕ) : Option NodeId := some validator
def key (block : BlockId) : String := toString block

/-- The executable model follows both final leaders and returns the expected
two-wave τ order. -/
example : tauRef bonds validators B2 B2_valid 1 selectLeader
    [block0.id, block1.id] key 1 = .ok [block0.id, block1.id] := by
  native_decide

/-- Extending the replay horizon from the first stable epoch to the second
preserves the first output as a prefix. -/
example : List.IsPrefix [block0.id]
    (match tauRef bonds validators B2 B2_valid 1 selectLeader
      [block0.id, block1.id] key 1 with
    | .ok output => output
    | .error _ => []) := by
  native_decide

end TwoWaveExample

end CordialMiners
