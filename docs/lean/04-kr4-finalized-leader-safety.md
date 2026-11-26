# KR4 — Finalized Leader Safety

Lean modules: `LeanVerification/Approval.lean`, `LeanVerification/Finality.lean`,
`LeanVerification/Ordering.lean`.
Depends on: KR1 (`Observe.lean`), KR2 (`Equivocation.lean`), KR3 (`Weights.lean`, `WCert.lean`).

## Why this exists

`no_conflicting_finals` is the theorem everything else in this initiative
exists to support: **two distinct blocks cannot both be finalized for the
same wave.** Until this is proved, the protocol has no formal safety
guarantee — only Rust tests covering the scenarios that were thought of.
This module provides the general proof, over all possible blocklace
histories regardless of adversarial choice.

The supporting `Ordering.lean` and `OrderingProofs.lean` modules add two guarantees:
- **Append-only ledger under a valid append**: when earlier finality decisions
  and emitted leader fragments remain stable, `tauRef` only extends its output.
- **Finality monotonicity**: once a block is finalized in `B`, it stays
  finalized in any superset `B'`.

---

## Approval (`Approval.lean`)

### `Approves`

```lean
def Approves (B : Blocklace) (approver target : BlockId) : Prop :=
  VouchesFor B approver target
```

Definitionally equal to `VouchesFor` from `Equivocation.lean`. The Rust
`approves` function (`consensus/approval.rs:29–104`, Definition 18 of
arXiv:2205.09174) checks exactly the same thing: the approver observes
the target and has no observed incomparable same-creator competitor.
Naming it separately lets `Finality.lean` and `Ordering.lean` refer to
the approval concept without depending on KR2's internals.

### Bond-weighted supermajority

```lean
abbrev bondOf (bonds : NodeId → ℕ) (S : Finset NodeId) : ℕ :=
  ∑ v ∈ S, bonds v

def StrictTwoThirdsMaj (bonds : NodeId → ℕ) (validators : Finset NodeId)
    (S : Finset NodeId) : Prop :=
  3 * bondOf bonds S > 2 * bondOf bonds validators
```

`NodeId = Nat` is infinite, so `Fintype NodeId` does not hold. We carry
an explicit finite `validators : Finset NodeId` universe and express the
2/3 threshold via cross-multiplication over `Finset.sum`. This is
arithmetically identical to `OpenTheta w 2 3` from `Weights.lean` on a
`Fintype` type.

Rust: `cordiality.rs:strict_two_thirds`, `is_weighted_supermajority`.

### `Ratifies` and `SuperRatifies`

```lean
def Ratifies (bonds : NodeId → ℕ) (validators : Finset NodeId)
    (B : Blocklace) (r b : BlockId) : Prop :=
  ∃ S : Finset NodeId,
    S ⊆ validators ∧
    (∀ v ∈ S, ∃ a, Observes B r a ∧ creatorOf B a = some v ∧ Approves B a b) ∧
    StrictTwoThirdsMaj bonds validators S

def SuperRatifies (bonds : NodeId → ℕ) (validators : Finset NodeId)
    (B : Blocklace) (witness : Finset BlockId) (b : BlockId) : Prop :=
  ∃ R : Finset NodeId,
    R ⊆ validators ∧
    (∀ v ∈ R, ∃ r ∈ witness, creatorOf B r = some v ∧ Ratifies bonds validators B r b) ∧
    StrictTwoThirdsMaj bonds validators R
```

Both are stated propositionally (using `∃`) to avoid requiring a
decidable `Observes` check, which would need a finite graph traversal
over the opaque `Blocklace`.

Rust:
- `Ratifies` → `cordiality.rs:weighted_ratifies` (252–303)
- `SuperRatifies` → `cordiality.rs:weighted_super_ratifies` (311–340)

### Bridge lemmas

```lean
theorem approves_implies_vouchesFor : Approves B approver target → VouchesFor B approver target
theorem approves_exclusion          : Equivocation B hV b₁ b₂ → Acknowledges B approver b₁ b₂ →
                                      ¬ Approves B approver b₁ ∧ ¬ Approves B approver b₂
theorem ratifies_mono               : Observes B d r → Ratifies ... B r b → Ratifies ... B d b
theorem superRatifies_mono          : witness ⊆ witness' → SuperRatifies ... witness b →
                                      SuperRatifies ... witness' b
```

`approves_exclusion` is the key bridge: an acknowledging block cannot
approve either branch of an equivocation. This is a direct application of
`Equivocation.lean:equivocation_not_approved` to the concrete `Approves`.

---

## Finalized Leader Safety (`Finality.lean`)

### Wave arithmetic

```lean
def waveOfRound      (round wavelength : ℕ) : ℕ := round / wavelength
def leaderRoundOfWave (wave wavelength : ℕ) : ℕ := wave * wavelength
def lastRoundOfWave   (wave wavelength : ℕ) : ℕ := wave * wavelength + wavelength - 1
```

Direct translation of `wave.rs`. Uses pure `ℕ`-division; call sites
hypothesize `0 < wavelength` where needed.

Rust: `wave.rs:wave_of_round`, `first_round_of_wave`, `last_round_of_wave`.

### Leader blocks

```lean
def leaderBlocksOfWave (B : Blocklace) (hV : ValidBlocklace B)
    (wave wavelength : ℕ) (sel : ℕ → Option NodeId) : Set BlockId :=
  {b | b ∈ B.keys ∧
       creatorOf B b = sel wave ∧
       blockDepth B hV b = leaderRoundOfWave wave wavelength}
```

Uses `Set BlockId` (propositional) rather than `Finset BlockId` to avoid
requiring a decidable depth comparison. `blockDepth B hV b` is the DAG
depth from `Equivocation.lean`, computed by well-founded recursion.

`sel : ℕ → Option NodeId` mirrors Rust's `Fn(u64) → Option<NodeId>`:
`sel wave = none` means no leader was elected for that wave (e.g. not
enough blocks have arrived yet), so the leader-block set is empty.

Rust: `wave.rs:leader_blocks_of_wave`.

### `FinalLeader`

```lean
def FinalLeader (bonds : NodeId → ℕ) (validators : Finset NodeId)
    (B : Blocklace) (hV : ValidBlocklace B)
    (wave wavelength : ℕ) (sel : ℕ → Option NodeId) (b : BlockId) : Prop :=
  b ∈ leaderBlocksOfWave B hV wave wavelength sel ∧
  ∃ witness : Finset BlockId,
    (∀ s ∈ witness,
      s ∈ B.keys ∧
      leaderRoundOfWave wave wavelength ≤ blockDepth B hV s ∧
      blockDepth B hV s ≤ lastRoundOfWave wave wavelength) ∧
    SuperRatifies bonds validators B witness b
```

Encodes two conditions:
1. **Leader-block membership** — `b` is present, has the correct creator
   (`creatorOf B b = sel wave`), and sits at the leader depth.
2. **Super-ratification** — a witness block set within the wave
   super-ratifies `b` under the given bond weights.

Uniqueness is **not** an axiom here — it is a theorem (`no_conflicting_finals`)
proved by the quorum-intersection argument from KR3.

Rust: `finality.rs:is_weighted_final_leader` + `final_leader_for_wave`.

### `leaderBlocks_equivocation`

```lean
lemma leaderBlocks_equivocation (B : Blocklace) (hV : ValidBlocklace B)
    (wave wavelength : ℕ) (sel : ℕ → Option NodeId)
    (b₁ b₂ : BlockId)
    (hb₁ : b₁ ∈ leaderBlocksOfWave B hV wave wavelength sel)
    (hb₂ : b₂ ∈ leaderBlocksOfWave B hV wave wavelength sel)
    (hne : b₁ ≠ b₂) : Equivocation B hV b₁ b₂
```

Any two distinct leader blocks for the same wave form a KR2-`Equivocation`:
they share creator (`sel wave`) and depth (`leaderRoundOfWave`), so
`same_depth_incomparable` gives incomparability, which is exactly the
equivocation condition. This lemma is the bridge from the leader-block
structure to the quorum-intersection argument.

### The safety theorem

```lean
theorem no_conflicting_finals
    (bonds : NodeId → ℕ) (validators : Finset NodeId)
    (B : Blocklace) (hV : ValidBlocklace B)
    (wave wavelength : ℕ) (_hwl : 0 < wavelength)
    (sel : ℕ → Option NodeId)
    (honestNodes : Finset NodeId)
    (hH_sub : honestNodes ⊆ validators)
    (hH_maj : StrictTwoThirdsMaj bonds validators honestNodes)
    (hHonest : ∀ v ∈ honestNodes, HonestIn B v)
    (b₁ b₂ : BlockId)
    (hb₁ : FinalLeader bonds validators B hV wave wavelength sel b₁)
    (hb₂ : FinalLeader bonds validators B hV wave wavelength sel b₂) :
    b₁ = b₂
```

**Proof sketch (by contradiction, assume `b₁ ≠ b₂`)**:

1. `leaderBlocks_equivocation` gives `Equivocation B hV b₁ b₂`.
2. Extract ratifier sets `R₁` (from `hb₁`) and `R₂` (from `hb₂`).
3. `finset_honest_triple_intersection` yields an honest ratifier
   `v* ∈ R₁ ∩ R₂ ∩ honestNodes`.
4. `v*` ratified `b₁` via `r₁` and `b₂` via `r₂`.
5. Extract approver sets `S₁`, `S₂`; `finset_honest_triple_intersection`
   again gives an honest approver `u* ∈ S₁ ∩ S₂ ∩ honestNodes`.
6. `u*` approved both `b₁` and `b₂`; `honest_chain_linearity` gives
   comparability of `u*`'s blocks, so `u*` has an approver block `a`
   that observes both ratifier blocks and therefore both finalized blocks.
7. `approves_exclusion` contradicts `u*` approving either branch of the
   `b₁`/`b₂` equivocation.

**Why the encoding is sound.** The quorum-intersection argument at both
the ratifier and approver levels uses the honest-kernel lemmas from KR3
(`Weights.lean:honest_triple_intersection`). The equivocation exclusion
(`approves_exclusion` from KR2) provides the final contradiction.

---

## Tau Ordering (`Ordering.lean`)

### Sub-blocklace

```lean
def SubBlocklace (B B' : Blocklace) : Prop :=
  ∀ b blk, B.lookup b = some blk → B'.lookup b = some blk
```

`B'` is an append-only extension of `B`: every block in `B` is present
in `B'` with identical content (block content is immutable). From this,
`observes_of_subBlocklace` follows: observation can only grow, it never
shrinks.

### Finality monotonicity

```lean
theorem FinalLeader_of_subBlocklace
    (bonds : NodeId → ℕ) (validators : Finset NodeId)
    (B B' : Blocklace) (hV : ValidBlocklace B) (hV' : ValidBlocklace B')
    (wave wavelength : ℕ) (sel : ℕ → Option NodeId)
    (hsub : SubBlocklace B B') (b : BlockId)
    (hfin : FinalLeader bonds validators B hV wave wavelength sel b) :
    FinalLeader bonds validators B' hV' wave wavelength sel b
```

If `b` is a finalized leader in `B`, it remains finalized in any monotone
extension `B'`. The proof transfers each component of `FinalLeader`:

| Component | Lemma used |
|---|---|
| `b ∈ B'.keys` | `keys_mono` |
| `creatorOf B' b = sel wave` | `creatorOf_of_subBlocklace` |
| `blockDepth B' hV' b = leaderRoundOfWave ...` | `blockDepth_of_subBlocklace` |
| Witness depths preserved | `blockDepth_of_subBlocklace` |
| `SuperRatifies ... B' witness b` | `superRatifies_of_subBlocklace` |

**`blockDepth_of_subBlocklace`** is the most involved: well-founded
induction over `directPred_wf_of_valid B hV`, using `Finset.sup_congr`
at each step to equate the `blockDepthWF` recursive equation in `B` and
`B'` (same lookup for B-resident blocks, `Closed B` bounds all
predecessor lookups within `B.keys`).

**`observes_stays_in_B`** is the key containment lemma: starting from
`a ∈ B.keys`, any block reachable via `B'`'s predecessor relation stays
in `B.keys`. New blocks added to `B'` cannot be reached from B-resident
blocks because `Closed B` locks all predecessor lookups within `B.keys`,
and `SubBlocklace` preserves those lookups unchanged.

**`observes_iff_subBlocklace`** follows: for `a, b ∈ B.keys`,
`Observes B a b ↔ Observes B' a b`. This allows lifting the
`VouchesFor` fork-freedom condition (`Approves`) from `B` to `B'` and
back, making `approves_of_subBlocklace` provable.

Addresses reviewer concern 3 on PR #229: finality monotonicity was absent
from the original formalization and is false under the old
uniqueness-encoding definition.

### Executable `tauRef`

```lean
def tauRef (bonds : NodeId → ℕ) (validators : Finset NodeId)
    (B : Blocklace) (hV : ValidBlocklace B)
    (wavelength : ℕ) (sel : ℕ → Option NodeId)
    (domain : List BlockId) (key : BlockId → String)
    (throughWave : ℕ) : Except String (List BlockId)
```

`CMRef.computeTauPlan` mirrors the Rust flow:

1. find the latest finalized leader through the executable `FinalLeader` check;
2. recursively find the latest earlier finalized leader ratified by it;
3. select the leader's newly approved fragment;
4. run deterministic Kahn sorting with the canonical block key;
5. flatten the leader epochs into the τ output.

The previous-final recursion is structurally recursive on explicit fuel, so
Lean checks termination. `xsortRef` validates that every successful sort has
the requested domain, contains no duplicates, and places selected predecessors
before children. It rejects cyclic input.

Rust: `ordering.rs:tau`.

### Equivocation safety of the complete τ output

[`OrderingEquivocation.lean`](../../lean/LeanVerification/OrderingEquivocation.lean)
proves exclusion across the entire recursive output, including pairs drawn
from different leader epochs. There are two distinct contracts:

```lean
-- For a successful tauRef run over a domain of present blocks:
tauRef_forkFree : ForkFree B output
tauRef_no_equivocating_pair :
  ∀ left right, Equivocation B hV left right →
    ¬ (left ∈ output ∧ right ∈ output)
```

These are theorem signatures with the shared run parameters and hypotheses
omitted here. They require `ValidBlocklace B`, a successful executable
`tauRef` computation, and `∀ block ∈ domain, block ∈ B.keys`. They do not
assume acknowledgement, honest leaders, an output invariant, or an epoch
prefix. `ForkFree` excludes every structural fork, including unequal-depth
forks, so it is stronger than the same-depth `Equivocation` corollary.

The proof follows the actual recursion. Every block emitted by an epoch is
approved by its leader (`tauRef_approval_provenance`). A leader ratifies the
previous final leader, hence observes it and the complete earlier output.
Approving a new branch while observing its competitor is impossible by
`Approves`. Induction establishes conflict-freedom for earlier pairs, pairs
within a new fragment, and pairs crossing the fragment boundary. Topological
sorting cannot introduce a block outside the approved fragment.

The stronger statement that **neither branch of any equivocation occurs**
requires an additional visibility condition:

```lean
-- Also require every nonempty emitting epoch's leader to acknowledge
-- every equivocation pair in this blocklace:
tauRef_excludes_all_equivocations :
  ∀ left right, Equivocation B hV left right →
    left ∉ output ∧ right ∉ output
```

`tauRef_excludes_acknowledged_equivocation` gives the same guarantee for one
pair. The acknowledgement premise is quantified over leaders in the computed
epoch plan, including earlier emitting epochs. Acknowledgement by the latest
leader alone does not remove blocks from an earlier output prefix.

Unconditional exclusion of every branch is false for the existing algorithm.
[`OrderingEquivocationExamples.lean`](../../lean/LeanVerification/OrderingEquivocationExamples.lean)
contains a machine-checked counterexample with a complete replay domain: two
same-creator roots equivocate, while a finalized honest leader observes only
the first. The complete τ output contains that first branch. The equivocator
has zero stake and the honest leader has all positive stake, so the example
does not rely on Byzantine majority weight. `blanket_exclusion_false` formally
refutes the unconditional statement.

[`OrderingEquivocationWavelength3Examples.lean`](../../lean/LeanVerification/OrderingEquivocationWavelength3Examples.lean)
exercises the configured wavelength of three rounds with two consecutive
waves. Their leaders are at depths 3 and 6, with finality witnesses in the
last rounds at depths 5 and 8. Both snapshots use their complete blocklace
domains and return explicitly checked, nonempty τ outputs. The first leader
observes only one branch of a genuine equivocation. The second leader
acknowledges both and approves neither, but the first branch remains in the
earlier emitted prefix. Executable checks verify the previous-final-leader
link, the two emitting epochs, and exclusion of the conflicting pair before
and after acknowledgement. Both runs also instantiate `tauRef_forkFree`.
The fixture retains the single bonded validator used by the wavelength-one
examples; it checks wavelength-three ordering and acknowledgement rather than
simulating a multi-validator deployment.

These results concern the existing Lean `tauRef` model of `ordering.rs:tau`.
They do not establish source-level Rust refinement, agreement between
unrelated snapshots, or separate correctness of weighted epoch-boundary,
checkpoint, and cached Rust paths. No consensus algorithm is changed.

### Prefix-safety under valid blocklace growth

```lean
theorem tauRef_prefix_of_validBlocklaceGrowth
    (growth : ValidBlocklaceGrowth ...)
    (hOld : tauRef ... B ... = .ok oldOutput) :
    ∃ suffix, tauRef ... B' ... = .ok (oldOutput ++ suffix)
```

This is proved with no axiom and, unlike the earlier Issue #244 version, does
not accept `List.IsPrefix oldEpochs newEpochs` as evidence. `ValidAppend`
remains as a compatibility alias for `ValidBlocklaceGrowth`.

`ValidBlocklaceGrowthEvidence` records these conditions and
`ValidBlocklaceGrowth` hides the witness behind `Nonempty`:

- `B'` is a `SubBlocklace` extension of `B`;
- `AppendOnlyDomain` says the old replay domain is exactly `B.keys`, while the
  extended domain is `domain ++ added` and every added identity is a valid,
  genuinely new `B'` block;
- `throughWave ≤ throughWave'`, while executable `finalLeaderAt?` decisions
  are required to be stable only for `wave ≤ throughWave`;
- the `checkRatifies` decisions consulted for leaders in the old domain are
  stable;
- the old latest leader belongs to the old domain and its computed wave is
  bounded by the old horizon;
- if the old run has a latest leader, repeatedly applying
  `previousFinalLeader?` from the new latest leader reaches it, with enough
  added replay horizon to fund those recursive steps; and
- bonds, validators, wavelength, leader selector, and canonical key are shared
  by construction.

These are local protocol conditions rather than a reformulation of the desired
plan prefix. In particular, finality stability says nothing about newly scanned
waves above the old horizon. Thus ordinary growth such as `B1/[block0]` at
wave 0 to `B2/[block0, block1]` at wave 1 is not excluded: the new wave may
discover `block1`.

Approved-fragment or sorted-output equality is not a field of the growth
predicate. It is derived as follows. `observes_iff_subBlocklace` proves that
observations between old blocks are unchanged, while `observes_stays_in_B`
proves an old leader cannot observe a new block. Consequently
`checkApproves_eq_of_subBlocklace` preserves approval of old targets and
`checkApproves_new_eq_false` rejects appended targets. `approvedBlocks_stable`
then proves equality of the filtered approved lists. Parent lookup is immutable
for those old blocks, and `xsortRef_eq_of_parents_eq_on` proves the Kahn sorter
and its validator are insensitive to parent-map changes outside that selected
fragment. `earlierFragment_stable` combines those results.

The leader-chain condition remains necessary when the larger horizon discovers
newer final leaders: it connects their recursion to the old latest leader
without mentioning epoch or output lists.

The proof proceeds in four stages:

1. `approvedBlocks_stable` and `earlierFragment_stable` derive unchanged old
   approved/sorted fragments from append-only blocklace and domain growth.
2. `previousFinalLeader_stable` derives equality of earlier previous-leader
   searches from finality bounded by the old horizon and old-leader
   ratification stability.
3. `tauPlanFromLeader_stable` proves by fuel induction that an old recursive
   leader computation is unchanged, including each approved/sorted fragment.
4. `tauPlanFromLeader_prefix_of_chain` proves by induction over the semantic
   previous-leader chain that the old epoch plan prefixes the new plan.
   `tauPlanFromLeader_fuel_mono` handles the larger replay fuel.
5. `flattenEpochs_prefix` converts the derived epoch prefix to an output prefix;
   `tauRef_eq_of_plan` then yields the stated successful extended run.

`SubBlocklace` alone is intentionally insufficient: newly learned blocks may
change which earlier leader or approved fragment is visible. Append-only DAG,
domain, and horizon growth establish that this is a genuine extension; bounded
finality and old-leader ratification stability rule out retroactive decisions.
The former unconditional prefix premise hid this requirement and has been
removed.

`OrderingProofs.lean` also proves `tauRef_deterministic`, `tauRef_nodup`, and
`tauRef_predecessor_ordered`, and contains executable examples for tie-breaking,
cycle rejection, a real two-wave formal blocklace, and prefix extension.

---

## Rust Correspondence

| Lean | Rust location |
|---|---|
| `Approves` | `consensus/approval.rs:approves` (29–104) |
| `bondOf`, `StrictTwoThirdsMaj` | `cordiality.rs:strict_two_thirds`, `is_weighted_supermajority` |
| `Ratifies` | `cordiality.rs:weighted_ratifies` (252–303) |
| `SuperRatifies` | `cordiality.rs:weighted_super_ratifies` (311–340) |
| `waveOfRound` | `wave.rs:wave_of_round` (26–32) |
| `leaderRoundOfWave` | `wave.rs:first_round_of_wave` / `leader_round_of_wave` (35–69) |
| `lastRoundOfWave` | `wave.rs:last_round_of_wave` (44–47) |
| `leaderBlocksOfWave` | `wave.rs:leader_blocks_of_wave` (79–100) |
| `FinalLeader` | `finality.rs:is_weighted_final_leader` (196–245) + `final_leader_for_wave` (251–267) |
| `leaderBlocks_equivocation` | Structural precondition for KR2 exclusion |
| `no_conflicting_finals` | Safety invariant — no single Rust function; proved by contradiction |
| `SubBlocklace` | Append-only blocklace growth model |
| `FinalLeader_of_subBlocklace` | Finality monotonicity — once final, always final |
| `CMRef.xsortRef` | `ordering.rs:xsort` |
| `CMRef.previousFinalLeader?` | `ordering.rs:previous_final_leader` |
| `CMRef.computeTauPlan`, `tauRef` | `ordering.rs:tau` |
| `tauRef_approval_provenance` | `approved_blocks_for_leader`, recursive `tau` fragments |
| `tauRef_forkFree`, `tauRef_no_equivocating_pair` | Whole `tau` output, including conflicts across recursive fragments |
| `tauRef_excludes_acknowledged_equivocation`, `tauRef_excludes_all_equivocations` | Complete-output exclusion under acknowledgement by every emitting leader |
| `AppendOnlyDomain`, `approvedBlocks_stable` | Old-domain preservation and proof that appended blocks cannot enter an old approved fragment |
| `xsortRef_eq_of_parents_eq_on`, `earlierFragment_stable` | Deterministic `xsort` stability derived for earlier fragments |
| `StableEarlierFinality` | Stability of repeated `final_leader_for_wave` results only through the old replay horizon |
| `StableEarlierRatification`, `previousFinalLeader_stable` | Stable `previous_final_leader` scan and `weighted_ratifies` decisions |
| `tauPlanFromLeader_stable` | Recursive old-plan stability using the derived fragment theorem |
| `PreviousFinalLeaderChain` | Recursive `previous_final_leader` path from a newer latest leader to the old latest leader |
| `tauRef_prefix_of_validBlocklaceGrowth` | Derived ledger append-only invariant under valid growth |
