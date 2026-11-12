/-
Executable equivocation checks at the deployed wavelength of three rounds.
Both waves have a leader, an approval round, and a ratification round. The
second leader acknowledges a fork whose first branch was already emitted.
-/
import LeanVerification.OrderingEquivocationExamples

namespace CordialMiners.TauEquivocationExample.WavelengthThree

open CMRef

/-- Extend the sole bonded validator's chain, optionally learning a fork. -/
def chainBlock (parents : Finset BlockId) : Block :=
  let content : BlockContent := { payload := [], predecessors := parents }
  { id := hashContent observerId content, creator := observerId,
    content := content, id_eq := rfl }

def round2 : Block := chainBlock {leader.id}
def leader3 : Block := chainBlock {round2.id}
def round4 : Block := chainBlock {leader3.id}
def round5 : Block := chainBlock {round4.id}
def leader6 : Block := chainBlock {round5.id, e2.id}
def round7 : Block := chainBlock {leader6.id}
def round8 : Block := chainBlock {round7.id}

def B2 : Blocklace := blocklaceInsert B round2
def B3 : Blocklace := blocklaceInsert B2 leader3
def B4 : Blocklace := blocklaceInsert B3 round4
def beforeAck : Blocklace := blocklaceInsert B4 round5
def B6 : Blocklace := blocklaceInsert beforeAck leader6
def B7 : Blocklace := blocklaceInsert B6 round7
def afterAck : Blocklace := blocklaceInsert B7 round8

private theorem checked_insert_valid (old : Blocklace) (block : Block)
    (hValid : ValidBlocklace old)
    (hParents : block.content.predecessors ⊆ old.keys)
    (hFresh : block.id ∉ old.keys) : ValidBlocklace (blocklaceInsert old block) :=
  ValidBlocklace.insert old block hValid hParents hFresh

theorem B2_valid : ValidBlocklace B2 :=
  checked_insert_valid B round2 TauEquivocationExample.valid
    (by native_decide) (by native_decide)

theorem B3_valid : ValidBlocklace B3 :=
  checked_insert_valid B2 leader3 B2_valid (by native_decide) (by native_decide)

theorem B4_valid : ValidBlocklace B4 :=
  checked_insert_valid B3 round4 B3_valid (by native_decide) (by native_decide)

theorem beforeAck_valid : ValidBlocklace beforeAck :=
  checked_insert_valid B4 round5 B4_valid (by native_decide) (by native_decide)

theorem B6_valid : ValidBlocklace B6 :=
  checked_insert_valid beforeAck leader6 beforeAck_valid
    (by native_decide) (by native_decide)

theorem B7_valid : ValidBlocklace B7 :=
  checked_insert_valid B6 round7 B6_valid (by native_decide) (by native_decide)

theorem afterAck_valid : ValidBlocklace afterAck :=
  checked_insert_valid B7 round8 B7_valid (by native_decide) (by native_decide)

/-- Use every block in each snapshot, including the unapproved fork branch
and the two rounds following its final leader. -/
def firstDomain : List BlockId := beforeAck.keys.sort (· ≤ ·)
def secondDomain : List BlockId := afterAck.keys.sort (· ≤ ·)

theorem firstDomain_complete : ∀ block, block ∈ firstDomain ↔ block ∈ beforeAck.keys := by
  intro block
  simp [firstDomain]

theorem secondDomain_complete : ∀ block, block ∈ secondDomain ↔ block ∈ afterAck.keys := by
  intro block
  simp [secondDomain]

def firstOutput : List BlockId := [e1.id, leader.id, round2.id, leader3.id]
def secondOutput : List BlockId := firstOutput ++ [round4.id, round5.id, leader6.id]

/-- Leaders and witnesses lie in rounds 3--5 and 6--8, respectively. -/
theorem first_wave_depths :
    blockDepth beforeAck beforeAck_valid leader3.id = 3 ∧
    blockDepth beforeAck beforeAck_valid round4.id = 4 ∧
    blockDepth beforeAck beforeAck_valid round5.id = 5 := by
  native_decide

theorem second_wave_depths :
    blockDepth afterAck afterAck_valid leader6.id = 6 ∧
    blockDepth afterAck afterAck_valid round7.id = 7 ∧
    blockDepth afterAck afterAck_valid round8.id = 8 := by
  native_decide

/-- Finality has a witness in the last round of each length-three wave. -/
theorem first_wave_witness :
    checkSuperRatifies bonds validators beforeAck beforeAck_valid
      {round5.id} leader3.id = true := by
  native_decide

theorem second_wave_witness :
    checkSuperRatifies bonds validators afterAck afterAck_valid
      {round8.id} leader6.id = true := by
  native_decide

theorem first_leader_final :
    FinalLeader bonds validators beforeAck beforeAck_valid 1 3 selectLeader leader3.id :=
  (checkFinal_iff bonds validators beforeAck beforeAck_valid 1 3 selectLeader
    leader3.id).mp (by native_decide)

theorem second_leader_final :
    FinalLeader bonds validators afterAck afterAck_valid 2 3 selectLeader leader6.id :=
  (checkFinal_iff bonds validators afterAck afterAck_valid 2 3 selectLeader
    leader6.id).mp (by native_decide)

theorem first_equivocation : Equivocation beforeAck beforeAck_valid e1.id e2.id :=
  (checkEquivocation_iff beforeAck beforeAck_valid e1.id e2.id).mp (by native_decide)

theorem second_equivocation : Equivocation afterAck afterAck_valid e1.id e2.id :=
  (checkEquivocation_iff afterAck afterAck_valid e1.id e2.id).mp (by native_decide)

/-- The first leader sees only one branch, despite both being in the domain. -/
theorem first_leader_visibility :
    checkObserves beforeAck beforeAck_valid leader3.id e1.id = true ∧
    checkObserves beforeAck beforeAck_valid leader3.id e2.id = false := by
  native_decide

/-- Execute τ at wavelength three and check its exact nonempty output. -/
theorem first_tau_output :
    tauRef bonds validators beforeAck beforeAck_valid 3 selectLeader
      firstDomain toString 1 = .ok firstOutput := by
  native_decide

theorem first_output_nonempty : firstOutput ≠ [] := by native_decide

theorem first_pair_excluded : ¬ (e1.id ∈ firstOutput ∧ e2.id ∈ firstOutput) := by
  native_decide

/-- Also instantiate the general theorem on this concrete wavelength-three run. -/
theorem first_output_forkFree : ForkFree beforeAck firstOutput :=
  tauRef_forkFree bonds validators beforeAck beforeAck_valid 3 selectLeader
    firstDomain toString 1 (fun block hBlock => (firstDomain_complete block).mp hBlock)
    firstOutput first_tau_output

/-- The next wave follows the previous final leader rather than starting an
unrelated τ plan. -/
theorem previous_final_leader :
    previousFinalLeader? bonds validators afterAck afterAck_valid 3 selectLeader
      secondDomain 2 leader6.id = some (1, leader3.id) := by
  native_decide

theorem later_acknowledgement : Acknowledges afterAck leader6.id e1.id e2.id := by
  constructor
  · exact (checkObserves_iff afterAck afterAck_valid leader6.id e1.id
      (by native_decide)).mp (by native_decide)
  · exact (checkObserves_iff afterAck afterAck_valid leader6.id e2.id
      (by native_decide)).mp (by native_decide)

/-- Later acknowledgement excludes both branches from the new fragment. -/
theorem later_leader_rejects_both :
    checkApproves afterAck afterAck_valid leader6.id e1.id = false ∧
    checkApproves afterAck afterAck_valid leader6.id e2.id = false := by
  native_decide

/-- Both epochs emit blocks; the older branch survives in the prefix, and
the newly acknowledged competitor is absent from the complete output. -/
theorem second_tau_plan :
    computeTauPlan bonds validators afterAck afterAck_valid 3 selectLeader
      secondDomain toString 2 = .ok (some leader6.id,
        [{ leader := leader3.id, blocks := firstOutput },
         { leader := leader6.id, blocks := [round4.id, round5.id, leader6.id] }]) := by
  native_decide

theorem second_tau_output :
    tauRef bonds validators afterAck afterAck_valid 3 selectLeader
      secondDomain toString 2 = .ok secondOutput := by
  native_decide

theorem second_output_nonempty : secondOutput ≠ [] := by native_decide

theorem earlier_prefix_preserved : List.IsPrefix firstOutput secondOutput :=
  ⟨[round4.id, round5.id, leader6.id], rfl⟩

theorem earlier_branch_retained : e1.id ∈ secondOutput ∧ e2.id ∉ secondOutput := by
  native_decide

theorem second_pair_excluded : ¬ (e1.id ∈ secondOutput ∧ e2.id ∈ secondOutput) := by
  native_decide

theorem second_output_forkFree : ForkFree afterAck secondOutput :=
  tauRef_forkFree bonds validators afterAck afterAck_valid 3 selectLeader
    secondDomain toString 2 (fun block hBlock => (secondDomain_complete block).mp hBlock)
    secondOutput second_tau_output

end CordialMiners.TauEquivocationExample.WavelengthThree
