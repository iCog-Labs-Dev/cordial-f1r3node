/-
Equivocation safety of the complete executable τ output.

Approval is causal: seeing one branch does not mean seeing its competitor.
Consequently absence of *both* branches requires visibility assumptions; it
must not be confused with absence of a conflicting pair in the output.
-/
import LeanVerification.OrderingProofs

namespace CordialMiners

open CMRef

/-- Conflict-freedom of a whole output, including pairs in different epochs. -/
def ForkFree (B : Blocklace) (output : List BlockId) : Prop :=
  ∀ left ∈ output, ∀ right ∈ output, ¬ Fork B left right

/-- An approver cannot approve a branch while observing its competitor. -/
theorem not_fork_of_approved_of_observed (B : Blocklace) (leader left right : BlockId)
    (hApproved : Approves B leader right) (hObserved : Observes B leader left) :
    ¬ Fork B left right := by
  rintro ⟨_, _, hCreator, hNe, hLR, hRL⟩
  exact hApproved.2 left hCreator hNe hObserved ⟨hRL, hLR⟩

/-- A strict weighted ratification has an approving witness, hence observes
its target. This follows even without a positive-total-stake assumption. -/
theorem observes_of_ratifies (bonds : NodeId → ℕ) (validators : Finset NodeId)
    (B : Blocklace) (leader target : BlockId)
    (hRatifies : Ratifies bonds validators B leader target) :
    Observes B leader target := by
  obtain ⟨S, _, hWitness, hMajority⟩ := hRatifies
  have hPositive : 0 < bondOf bonds S := by
    unfold StrictTwoThirdsMaj at hMajority
    omega
  obtain ⟨v, hv⟩ := bondOf_pos_nonempty bonds S hPositive
  obtain ⟨approver, hObserves, _, hApproves⟩ := hWitness v hv
  exact observes_trans B hObserves hApproves.1

/-- A recursion edge really ratifies the previous leader. -/
theorem previousFinalLeader_checkRatifies
    (bonds : NodeId → ℕ) (validators : Finset NodeId)
    (B : Blocklace) (hV : ValidBlocklace B)
    (wavelength : ℕ) (sel : ℕ → Option NodeId)
    (domain : List BlockId) (wave leader priorWave priorLeader : ℕ)
    (hPrevious : previousFinalLeader? bonds validators B hV wavelength sel
      domain wave leader = some (priorWave, priorLeader)) :
    checkRatifies bonds validators B hV leader priorLeader = true := by
  unfold previousFinalLeader? at hPrevious
  rw [List.findSome?_eq_some_iff] at hPrevious
  obtain ⟨before, foundWave, after, _, hFound, _⟩ := hPrevious
  cases hFinal : finalLeaderAt? bonds validators B hV wavelength sel domain foundWave with
  | none => simp [hFinal] at hFound
  | some foundLeader =>
      by_cases hCheck : checkRatifies bonds validators B hV leader foundLeader = true
      · simp [hFinal, hCheck] at hFound
        simpa [hFound.2] using hCheck
      · simp [hFinal, hCheck] at hFound

/-- A successful certified sort emits only members of its selected fragment. -/
theorem xsortRef_output_subset (parents : ParentMap) (key : BlockId → String)
    (selected output : List BlockId) (hRun : xsortRef parents key selected = .ok output) :
    ∀ block, block ∈ output → block ∈ selected := by
  have hSame := (xsortRef_correct parents key selected output hRun).2.1
  simp only [sameDomain, Bool.and_eq_true] at hSame
  have hAll := hSame.2
  intro block hBlock
  simpa using List.all_eq_true.mp hAll block hBlock

/-- Every emitted epoch block really is approved by that epoch's leader.
The replay domain need only contain present blocks; completeness is unnecessary. -/
def EpochsApproved (B : Blocklace) (epochs : List TauEpoch) : Prop :=
  ∀ epoch ∈ epochs, ∀ block ∈ epoch.blocks, Approves B epoch.leader block

/-- Approval provenance survives the entire previous-final-leader recursion. -/
theorem tauPlanFromLeader_epochsApproved
    (bonds : NodeId → ℕ) (validators : Finset NodeId)
    (B : Blocklace) (hV : ValidBlocklace B)
    (wavelength : ℕ) (sel : ℕ → Option NodeId)
    (domain : List BlockId) (key : BlockId → String)
    (hDomain : ∀ block ∈ domain, block ∈ B.keys) :
    ∀ fuel wave leader epochs, leader ∈ B.keys →
      tauPlanFromLeader bonds validators B hV wavelength sel domain key
        fuel wave leader = .ok epochs → EpochsApproved B epochs := by
  intro fuel
  induction fuel with
  | zero => intro wave leader epochs hLeader hRun; simp [tauPlanFromLeader] at hRun
  | succ fuel ih =>
      intro wave leader epochs hLeader hRun
      have finish (prior : List TauEpoch) (hPrior : EpochsApproved B prior)
          (hSuffix : (do
            let suffix ← xsortRef (blockParents B) key (domain.filter fun block =>
              checkApproves B hV leader block && !(flattenEpochs prior).contains block)
            pure (prior ++ [({ leader := leader, blocks := suffix } : TauEpoch)])) =
              .ok epochs) :
          EpochsApproved B epochs := by
        cases hSort : xsortRef (blockParents B) key (domain.filter fun block =>
            checkApproves B hV leader block && !(flattenEpochs prior).contains block) with
        | error message =>
            rw [hSort] at hSuffix
            contradiction
        | ok suffix =>
            rw [hSort] at hSuffix
            change Except.ok (prior ++ [{ leader := leader, blocks := suffix }]) =
              Except.ok epochs at hSuffix
            cases Except.ok.inj hSuffix
            intro epoch hEpoch block hBlock
            rcases List.mem_append.mp hEpoch with hOld | hNew
            · exact hPrior epoch hOld block hBlock
            · have hEq : epoch = { leader := leader, blocks := suffix } := by
                simpa using hNew
              subst epoch
              have hSelected := xsortRef_output_subset _ _ _ _ hSort block hBlock
              obtain ⟨hMem, hCheck⟩ := List.mem_filter.mp hSelected
              simp only [Bool.and_eq_true] at hCheck
              exact (checkApproves_iff B hV leader block hLeader
                (hDomain block hMem)).mp hCheck.1
      simp only [tauPlanFromLeader] at hRun
      cases hPrevious : previousFinalLeader? bonds validators B hV wavelength sel
          domain wave leader with
      | none =>
          apply finish [] (by simp [EpochsApproved])
          simpa [hPrevious] using hRun
      | some previous =>
          rcases previous with ⟨priorWave, priorLeader⟩
          have hMem := (previousFinalLeader_lt_and_mem bonds validators B hV wavelength
            sel domain wave leader priorWave priorLeader hPrevious).2
          simp only [hPrevious] at hRun
          cases hPrior : tauPlanFromLeader bonds validators B hV wavelength sel domain
              key fuel priorWave priorLeader with
          | error message =>
              rw [hPrior] at hRun
              contradiction
          | ok prior =>
              apply finish prior (ih priorWave priorLeader prior (hDomain _ hMem) hPrior)
              rw [hPrior] at hRun
              exact hRun

/-- Appending an approved fragment preserves conflict-freedom if its leader
observes the entire prior output. This includes every cross-epoch pair. -/
theorem forkFree_append_approved (B : Blocklace) (leader : BlockId)
    (prior suffix : List BlockId)
    (hPrior : ForkFree B prior)
    (hObserved : ∀ block ∈ prior, Observes B leader block)
    (hApproved : ∀ block ∈ suffix, Approves B leader block) :
    ForkFree B (prior ++ suffix) := by
  intro left hLeft right hRight
  rcases List.mem_append.mp hLeft with hLeft | hLeft
  · rcases List.mem_append.mp hRight with hRight | hRight
    · exact hPrior left hLeft right hRight
    · exact not_fork_of_approved_of_observed B leader left right
        (hApproved right hRight) (hObserved left hLeft)
  · rcases List.mem_append.mp hRight with hRight | hRight
    · intro hFork
      obtain ⟨hL, hR, hCreator, hNe, hLR, hRL⟩ := hFork
      exact not_fork_of_approved_of_observed B leader right left
        (hApproved left hLeft) (hObserved right hRight)
        ⟨hR, hL, hCreator.symm, hNe.symm, hRL, hLR⟩
    · exact not_fork_of_approved_of_observed B leader left right
        (hApproved right hRight) (hApproved left hLeft).1

/-- The recursive τ output is observed by its root leader and contains no
structural fork, even when the two candidates come from different epochs. -/
theorem tauPlanFromLeader_forkFree
    (bonds : NodeId → ℕ) (validators : Finset NodeId)
    (B : Blocklace) (hV : ValidBlocklace B)
    (wavelength : ℕ) (sel : ℕ → Option NodeId)
    (domain : List BlockId) (key : BlockId → String)
    (hDomain : ∀ block ∈ domain, block ∈ B.keys) :
    ∀ fuel wave leader epochs, leader ∈ B.keys →
      tauPlanFromLeader bonds validators B hV wavelength sel domain key
        fuel wave leader = .ok epochs →
      (∀ block ∈ flattenEpochs epochs, Observes B leader block) ∧
        ForkFree B (flattenEpochs epochs) := by
  intro fuel
  induction fuel with
  | zero => intro wave leader epochs hLeader hRun; simp [tauPlanFromLeader] at hRun
  | succ fuel ih =>
      intro wave leader epochs hLeader hRun
      have finish (prior : List TauEpoch)
          (hObserved : ∀ block ∈ flattenEpochs prior, Observes B leader block)
          (hFree : ForkFree B (flattenEpochs prior))
          (hSuffix : (do
            let suffix ← xsortRef (blockParents B) key (domain.filter fun block =>
              checkApproves B hV leader block && !(flattenEpochs prior).contains block)
            pure (prior ++ [({ leader := leader, blocks := suffix } : TauEpoch)])) =
              .ok epochs) :
          (∀ block ∈ flattenEpochs epochs, Observes B leader block) ∧
            ForkFree B (flattenEpochs epochs) := by
        cases hSort : xsortRef (blockParents B) key (domain.filter fun block =>
            checkApproves B hV leader block && !(flattenEpochs prior).contains block) with
        | error message => rw [hSort] at hSuffix; contradiction
        | ok suffix =>
            rw [hSort] at hSuffix
            change Except.ok (prior ++ [{ leader := leader, blocks := suffix }]) =
              Except.ok epochs at hSuffix
            cases Except.ok.inj hSuffix
            have hApproved : ∀ block ∈ suffix, Approves B leader block := by
              intro block hBlock
              have hSelected := xsortRef_output_subset _ _ _ _ hSort block hBlock
              obtain ⟨hMem, hCheck⟩ := List.mem_filter.mp hSelected
              simp only [Bool.and_eq_true] at hCheck
              exact (checkApproves_iff B hV leader block hLeader
                (hDomain block hMem)).mp hCheck.1
            rw [flattenEpochs_append]
            simp only [flattenEpochs, List.flatMap_cons, List.flatMap_nil, List.append_nil]
            change (∀ block ∈ flattenEpochs prior ++ suffix, Observes B leader block) ∧
              ForkFree B (flattenEpochs prior ++ suffix)
            refine ⟨?_, forkFree_append_approved B leader _ _ hFree hObserved hApproved⟩
            intro block hBlock
            rcases List.mem_append.mp hBlock with hOld | hNew
            · exact hObserved block hOld
            · exact (hApproved block hNew).1
      simp only [tauPlanFromLeader] at hRun
      cases hPrevious : previousFinalLeader? bonds validators B hV wavelength sel
          domain wave leader with
      | none =>
          apply finish [] (by simp [flattenEpochs]) (by simp [ForkFree, flattenEpochs])
          simpa [hPrevious] using hRun
      | some previous =>
          rcases previous with ⟨priorWave, priorLeader⟩
          have hMem := (previousFinalLeader_lt_and_mem bonds validators B hV wavelength
            sel domain wave leader priorWave priorLeader hPrevious).2
          have hObserves := observes_of_ratifies bonds validators B leader priorLeader
            ((checkRatifies_iff bonds validators B hV leader priorLeader hLeader
              (hDomain _ hMem)).mp (previousFinalLeader_checkRatifies bonds validators
                B hV wavelength sel domain wave leader priorWave priorLeader hPrevious))
          simp only [hPrevious] at hRun
          cases hPrior : tauPlanFromLeader bonds validators B hV wavelength sel domain
              key fuel priorWave priorLeader with
          | error message => rw [hPrior] at hRun; contradiction
          | ok prior =>
              obtain ⟨hSeen, hFree⟩ := ih priorWave priorLeader prior (hDomain _ hMem) hPrior
              apply finish prior (fun block hBlock =>
                observes_trans B hObserves (hSeen block hBlock)) hFree
              rw [hPrior] at hRun
              exact hRun

/-- The latest leader of a successful plan belongs to the replay domain. -/
theorem computeTauPlan_latest_mem_domain
    (bonds : NodeId → ℕ) (validators : Finset NodeId)
    (B : Blocklace) (hV : ValidBlocklace B)
    (wavelength : ℕ) (sel : ℕ → Option NodeId)
    (domain : List BlockId) (key : BlockId → String) (throughWave : ℕ)
    (leader : BlockId) (epochs : List TauEpoch)
    (hPlan : computeTauPlan bonds validators B hV wavelength sel domain key
      throughWave = .ok (some leader, epochs)) : leader ∈ domain := by
  unfold computeTauPlan at hPlan
  dsimp only at hPlan
  cases hLatest : (List.range (throughWave + 1)).reverse.findSome?
      (finalLeaderAt? bonds validators B hV wavelength sel domain) with
  | none =>
      rw [hLatest] at hPlan
      change Except.ok (none, []) = Except.ok (some leader, epochs) at hPlan
      have hImpossible := (Prod.mk.inj (Except.ok.inj hPlan)).1
      contradiction
  | some found =>
      have hFound := hLatest
      rw [List.findSome?_eq_some_iff] at hFound
      obtain ⟨_, wave, _, _, hFinal, _⟩ := hFound
      have hMem := finalLeaderAt_mem_domain bonds validators B hV wavelength sel
        domain wave found hFinal
      simp only [hLatest] at hPlan
      cases hRecursive : tauPlanFromLeader bonds validators B hV wavelength sel domain
          key (throughWave + 2) (waveOfRound (blockDepth B hV found) wavelength) found with
      | error message => rw [hRecursive] at hPlan; contradiction
      | ok foundEpochs =>
          rw [hRecursive] at hPlan
          change Except.ok (some found, foundEpochs) = Except.ok (some leader, epochs) at hPlan
          have hEq := Option.some.inj (Prod.mk.inj (Except.ok.inj hPlan)).1
          simpa [hEq] using hMem

/-- A successful complete τ output comes from an explicitly approved epoch plan. -/
theorem tauRef_approval_provenance
    (bonds : NodeId → ℕ) (validators : Finset NodeId)
    (B : Blocklace) (hV : ValidBlocklace B)
    (wavelength : ℕ) (sel : ℕ → Option NodeId)
    (domain : List BlockId) (key : BlockId → String) (throughWave : ℕ)
    (hDomain : ∀ block ∈ domain, block ∈ B.keys)
    (output : List BlockId)
    (hRun : tauRef bonds validators B hV wavelength sel domain key throughWave =
      .ok output) :
    ∃ latest epochs,
      computeTauPlan bonds validators B hV wavelength sel domain key throughWave =
        .ok (latest, epochs) ∧
      output = flattenEpochs epochs ∧ EpochsApproved B epochs := by
  unfold tauRef at hRun
  cases hPlan : computeTauPlan bonds validators B hV wavelength sel domain key
      throughWave with
  | error message =>
      rw [hPlan] at hRun
      contradiction
  | ok result =>
      rcases result with ⟨latest, epochs⟩
      have hOutput : output = flattenEpochs epochs := by
        simp only [hPlan] at hRun
        change (if checkTopological (blockParents B) (flattenEpochs epochs)
            (flattenEpochs epochs) then Except.ok (flattenEpochs epochs)
          else Except.error "tau output failed duplicate/predecessor validation") =
            Except.ok output at hRun
        split at hRun
        · exact (Except.ok.inj hRun).symm
        · simp at hRun
      refine ⟨latest, epochs, rfl, hOutput, ?_⟩
      cases latest with
      | none =>
          have hEmpty := computeTauPlan_none_epochs bonds validators B hV wavelength
            sel domain key throughWave epochs hPlan
          simp [hEmpty, EpochsApproved]
      | some leader =>
          have hLeader := computeTauPlan_latest_mem_domain bonds validators B hV
            wavelength sel domain key throughWave leader epochs hPlan
          exact tauPlanFromLeader_epochsApproved bonds validators B hV wavelength sel
            domain key hDomain _ _ _ _ (hDomain _ hLeader)
            (computeTauPlan_some_run bonds validators B hV wavelength sel domain key
              throughWave leader epochs hPlan)

/-- The complete τ output is conflict-free across all its epochs. The only
input condition beyond a valid blocklace is that replay-domain entries exist.
No acknowledgement or assumed output safety is supplied as a premise. -/
theorem tauRef_forkFree
    (bonds : NodeId → ℕ) (validators : Finset NodeId)
    (B : Blocklace) (hV : ValidBlocklace B)
    (wavelength : ℕ) (sel : ℕ → Option NodeId)
    (domain : List BlockId) (key : BlockId → String) (throughWave : ℕ)
    (hDomain : ∀ block ∈ domain, block ∈ B.keys)
    (output : List BlockId)
    (hRun : tauRef bonds validators B hV wavelength sel domain key throughWave =
      .ok output) : ForkFree B output := by
  obtain ⟨latest, epochs, hPlan, hOutput, _⟩ :=
    tauRef_approval_provenance bonds validators B hV wavelength sel domain key
      throughWave hDomain output hRun
  rw [hOutput]
  cases latest with
  | none =>
      have hEmpty := computeTauPlan_none_epochs bonds validators B hV wavelength
        sel domain key throughWave epochs hPlan
      simp [hEmpty, ForkFree, flattenEpochs]
  | some leader =>
      have hLeader := computeTauPlan_latest_mem_domain bonds validators B hV
        wavelength sel domain key throughWave leader epochs hPlan
      exact (tauPlanFromLeader_forkFree bonds validators B hV wavelength sel domain
        key hDomain _ _ _ _ (hDomain _ hLeader)
        (computeTauPlan_some_run bonds validators B hV wavelength sel domain key
          throughWave leader epochs hPlan)).2

/-- No two equivocating blocks can both occur anywhere in the complete τ
output. Structural fork-freedom above is stronger: equal depth is unnecessary. -/
theorem tauRef_no_equivocating_pair
    (bonds : NodeId → ℕ) (validators : Finset NodeId)
    (B : Blocklace) (hV : ValidBlocklace B)
    (wavelength : ℕ) (sel : ℕ → Option NodeId)
    (domain : List BlockId) (key : BlockId → String) (throughWave : ℕ)
    (hDomain : ∀ block ∈ domain, block ∈ B.keys)
    (output : List BlockId)
    (hRun : tauRef bonds validators B hV wavelength sel domain key throughWave =
      .ok output) :
    ∀ left right, Equivocation B hV left right → ¬ (left ∈ output ∧ right ∈ output) := by
  intro left right hEq hBoth
  exact tauRef_forkFree bonds validators B hV wavelength sel domain key throughWave
    hDomain output hRun left hBoth.1 right hBoth.2 hEq.1

/-- Complete-output exclusion, with the essential visibility premise exposed.
Every emitting epoch leader must acknowledge the pair, including earlier leaders;
acknowledgement by the latest leader alone cannot remove an emitted prefix. -/
theorem tauRef_excludes_acknowledged_equivocation
    (bonds : NodeId → ℕ) (validators : Finset NodeId)
    (B : Blocklace) (hV : ValidBlocklace B)
    (wavelength : ℕ) (sel : ℕ → Option NodeId)
    (domain : List BlockId) (key : BlockId → String) (throughWave : ℕ)
    (hDomain : ∀ block ∈ domain, block ∈ B.keys)
    (output : List BlockId)
    (hRun : tauRef bonds validators B hV wavelength sel domain key throughWave =
      .ok output)
    (left right : BlockId) (hEq : Equivocation B hV left right)
    (hVisible : ∀ latest epochs,
      computeTauPlan bonds validators B hV wavelength sel domain key throughWave =
        .ok (latest, epochs) →
      ∀ epoch ∈ epochs, epoch.blocks ≠ [] →
        Acknowledges B epoch.leader left right) :
    left ∉ output ∧ right ∉ output := by
  obtain ⟨latest, epochs, hPlan, hOutput, hApproved⟩ :=
    tauRef_approval_provenance bonds validators B hV wavelength sel domain key
      throughWave hDomain output hRun
  have excluded (block : BlockId) (hSide : block = left ∨ block = right)
      (hBlock : block ∈ output) : False := by
    rw [hOutput, flattenEpochs, List.mem_flatMap] at hBlock
    obtain ⟨epoch, hEpoch, hMem⟩ := hBlock
    have hNonempty : epoch.blocks ≠ [] := by intro h; simp [h] at hMem
    have hNo := approves_exclusion B hV left right epoch.leader hEq
      (hVisible latest epochs hPlan epoch hEpoch hNonempty)
    rcases hSide with hLeft | hRight
    · exact hNo.1 (hLeft ▸ hApproved epoch hEpoch block hMem)
    · exact hNo.2 (hRight ▸ hApproved epoch hEpoch block hMem)
  exact ⟨excluded left (Or.inl rfl), excluded right (Or.inr rfl)⟩

/-- Every branch of every equivocation is excluded from the whole output
provided every emitting leader acknowledges every such pair. This is an extra
protocol condition, not a property of arbitrary valid blocklaces. -/
theorem tauRef_excludes_all_equivocations
    (bonds : NodeId → ℕ) (validators : Finset NodeId)
    (B : Blocklace) (hV : ValidBlocklace B)
    (wavelength : ℕ) (sel : ℕ → Option NodeId)
    (domain : List BlockId) (key : BlockId → String) (throughWave : ℕ)
    (hDomain : ∀ block ∈ domain, block ∈ B.keys)
    (output : List BlockId)
    (hRun : tauRef bonds validators B hV wavelength sel domain key throughWave =
      .ok output)
    (hVisible : ∀ latest epochs,
      computeTauPlan bonds validators B hV wavelength sel domain key throughWave =
        .ok (latest, epochs) →
      ∀ epoch ∈ epochs, epoch.blocks ≠ [] →
      ∀ left right, Equivocation B hV left right →
        Acknowledges B epoch.leader left right) :
    ∀ left right, Equivocation B hV left right → left ∉ output ∧ right ∉ output := by
  intro left right hEq
  exact tauRef_excludes_acknowledged_equivocation bonds validators B hV wavelength
    sel domain key throughWave hDomain output hRun left right hEq
    (fun latest epochs hPlan epoch hEpoch hNonempty =>
      hVisible latest epochs hPlan epoch hEpoch hNonempty left right hEq)

end CordialMiners
