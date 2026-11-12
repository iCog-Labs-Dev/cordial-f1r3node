/- Executable witnesses for the limits of complete-output exclusion. -/
import LeanVerification.OrderingEquivocation

namespace CordialMiners.TauEquivocationExample

open CMRef

/-- The two roots `e1` and `e2` from KR2 are a genuine same-depth fork.
This honest leader observes only the first branch. -/
def leaderContent : BlockContent := { payload := [], predecessors := {e1.id} }
def leader : Block :=
  { id := hashContent observerId leaderContent, creator := observerId,
    content := leaderContent, id_eq := rfl }
def B : Blocklace := blocklaceInsert demoB2 leader

theorem valid : ValidBlocklace B := by
  apply ValidBlocklace.insert demoB2 leader demoB2_valid
  · intro predecessor hMem
    have hEq : predecessor = e1.id := by simpa [leader, leaderContent] using hMem
    rw [hEq]
    exact key_mem_insert_of_mem demoB1 e2 (key_mem_insert_self demoB0 e1)
  · apply key_not_mem_insert demoB1 e2
    · native_decide
    · apply key_not_mem_insert demoB0 e1
      · native_decide
      · exact no_mem_demoB0 leader.id

def bonds (node : NodeId) : ℕ := if node = observerId then 1 else 0
def validators : Finset NodeId := {observerId}
def selectLeader (_wave : ℕ) : Option NodeId := some observerId
def domain : List BlockId := [e1.id, e2.id, leader.id]

theorem domain_complete : ∀ block, block ∈ domain ↔ block ∈ B.keys := by
  intro block
  constructor
  · intro hBlock
    simp only [domain, List.mem_cons, List.not_mem_nil, or_false] at hBlock
    rcases hBlock with rfl | rfl | rfl <;> native_decide
  · intro hBlock
    rcases Finmap.mem_insert.mp (Finmap.mem_keys.mp hBlock) with hLeader | hOld
    · simp [domain, hLeader]
    rcases Finmap.mem_insert.mp hOld with hE2 | hOld
    · simp [domain, hE2]
    rcases Finmap.mem_insert.mp hOld with hE1 | hEmpty
    · simp [domain, hE1]
    exact False.elim (no_mem_demoB0 block (Finmap.mem_keys.mpr hEmpty))

theorem equivocation : Equivocation B valid e1.id e2.id := by
  exact (checkEquivocation_iff B valid e1.id e2.id).mp (by native_decide)

/-- The complete executable τ output contains a branch of a known global
equivocation. The domain is the entire blocklace, and the leader has all stake;
neither a truncated input nor absent finality explains the counterexample. -/
theorem tau_emits_one_branch :
    tauRef bonds validators B valid 1 selectLeader domain toString 1 =
      .ok [e1.id, leader.id] := by
  native_decide

/-- A concrete refutation of unconditional exclusion of every fork branch. -/
theorem blanket_exclusion_false :
    ¬ (∀ output, tauRef bonds validators B valid 1 selectLeader domain toString 1 =
        .ok output →
      ∀ left right, Equivocation B valid left right →
        left ∉ output ∧ right ∉ output) := by
  intro h
  exact (h _ tau_emits_one_branch _ _ equivocation).1 (by simp)

/-- Apply the general theorem to this nonempty counterexample: a single
equivocation branch is allowed, while the conflicting pair is excluded. -/
theorem emitted_output_forkFree : ForkFree B [e1.id, leader.id] :=
  tauRef_forkFree bonds validators B valid 1 selectLeader domain toString 1
    (fun block hBlock => (domain_complete block).mp hBlock) _ tau_emits_one_branch

namespace Acknowledged

def domain : List BlockId := [e1.id, e2.id, g.id, c.id]

theorem domain_present : ∀ block ∈ domain, block ∈ demoB.keys := by
  intro block hBlock
  simp only [domain, List.mem_cons, List.not_mem_nil, or_false] at hBlock
  rcases hBlock with rfl | rfl | rfl | rfl <;> native_decide

/-- When the first emitting leader sees both branches, neither is emitted. -/
theorem tau_excludes_both :
    tauRef bonds validators demoB demoB_valid 1 selectLeader domain toString 1 =
      .ok [c.id] := by
  native_decide

theorem plan :
    computeTauPlan bonds validators demoB demoB_valid 1 selectLeader domain toString 1 =
      .ok (some c.id, [{ leader := c.id, blocks := [c.id] }]) := by
  native_decide

/-- The visibility premise is satisfiable with a nonempty τ output and a
real equivocation. This exercises the complete-output exclusion theorem. -/
theorem both_excluded_by_theorem : e1.id ∉ [c.id] ∧ e2.id ∉ [c.id] := by
  apply tauRef_excludes_acknowledged_equivocation bonds validators demoB demoB_valid
    1 selectLeader domain toString 1 domain_present _ tau_excludes_both
    e1.id e2.id e1_e2_equivocate
  intro latest epochs hPlan epoch hEpoch _
  rw [plan] at hPlan
  obtain ⟨_, hEpochs⟩ := Prod.mk.inj (Except.ok.inj hPlan)
  rw [← hEpochs] at hEpoch
  have hEpochEq : epoch = { leader := c.id, blocks := [c.id] } := by
    simpa using hEpoch
  rw [hEpochEq]
  exact c_acknowledges

end Acknowledged

namespace DelayedAcknowledgement

/-- This next leader sees the earlier leader and the formerly hidden branch. -/
def nextContent : BlockContent :=
  { payload := [], predecessors := {leader.id, e2.id} }
def next : Block :=
  { id := hashContent observerId nextContent, creator := observerId,
    content := nextContent, id_eq := rfl }
def extended : Blocklace := blocklaceInsert B next

theorem valid : ValidBlocklace extended := by
  apply ValidBlocklace.insert B next TauEquivocationExample.valid
  · intro predecessor hMem
    have hCases : predecessor = leader.id ∨ predecessor = e2.id := by
      simpa [next, nextContent] using hMem
    rcases hCases with rfl | rfl <;> native_decide
  · apply key_not_mem_insert demoB2 leader
    · native_decide
    · apply key_not_mem_insert demoB1 e2
      · native_decide
      · apply key_not_mem_insert demoB0 e1
        · native_decide
        · exact no_mem_demoB0 next.id

def domain : List BlockId := [e1.id, e2.id, leader.id, next.id]

theorem domain_present : ∀ block ∈ domain, block ∈ extended.keys := by
  intro block hBlock
  simp only [domain, List.mem_cons, List.not_mem_nil, or_false] at hBlock
  rcases hBlock with rfl | rfl | rfl | rfl <;> native_decide

theorem equivocation : Equivocation extended valid e1.id e2.id :=
  (checkEquivocation_iff extended valid e1.id e2.id).mp (by native_decide)

theorem latest_acknowledges : Acknowledges extended next.id e1.id e2.id := by
  constructor
  · exact (checkObserves_iff extended valid next.id e1.id
      (domain_present _ (by simp [domain]))).mp (by native_decide)
  · exact (checkObserves_iff extended valid next.id e2.id
      (domain_present _ (by simp [domain]))).mp (by native_decide)

/-- Later acknowledgement does not retract a branch emitted by an earlier
epoch: τ's approved fragments are accumulated into a complete prefix. -/
theorem tau_preserves_earlier_branch :
    tauRef bonds validators extended valid 1 selectLeader domain toString 2 =
      .ok [e1.id, leader.id, next.id] := by
  native_decide

/-- The general conflict-freedom proof applies across both emitting epochs. -/
theorem complete_output_forkFree : ForkFree extended [e1.id, leader.id, next.id] :=
  tauRef_forkFree bonds validators extended valid 1 selectLeader domain toString 2
    domain_present _ tau_preserves_earlier_branch

end DelayedAcknowledgement

end CordialMiners.TauEquivocationExample
