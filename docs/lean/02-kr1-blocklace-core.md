# KR1 — Blocklace Core: Lean Formalization

**Lean files:** `Block.lean`, `Blocklace.lean`, `Observe.lean`
**Rust reference:** `crates/cordial-miners-core/src/blocklace.rs`, `crates/cordial-miners-core/src/block.rs`
**Issue:** #185 (KR1 — Blocklace Core)

## 1. Core Definitions

### `BlockId` and `NodeId`

```lean
abbrev NodeId  := Nat
abbrev BlockId := Nat
```

Both are opaque `Nat` surrogates. In Rust, `BlockIdentity` carries a creator `NodeId` plus a
cryptographic content hash; `NodeId` is the creator identity. Signatures and hash internals
are hidden behind `hashContent` and `hashInj`.

---

### `BlockContent`

```lean
structure BlockContent where
  payload      : List UInt8
  predecessors : Finset BlockId
```

`payload` is the arbitrary application value `v` from Definition 2.1 of the paper.
`predecessors` is the set `P` of block IDs that this block directly points back at.

**Rust:** `BlockContent` in `types/content_id.rs`; accessed via `block.content.predecessors`
(`blocklace.rs:159–165`).

---

### `Block`

```lean
structure Block where
  id      : BlockId
  creator : NodeId
  content : BlockContent
  id_eq   : id = hashContent content
```

`id_eq` is a proof-carrying invariant: every `Block` is self-certifying, its identity is
definitionally the hash of its content. You cannot fabricate a `Block` value with a mismatched ID.

**Rust:** `Block { identity, content }` (`block.rs`).

---

### `hashContent` and `hashInj`

```lean
opaque hashContent : BlockContent → BlockId

axiom hashInj {c1 c2 : BlockContent}
    (h : hashContent c1 = hashContent c2) : c1 = c2
```

`hashContent` is `opaque`; Lean treats it as an abstract function. `hashInj` is the **only
non-standard axiom** in the formalization, it asserts collision-resistance. All acyclicity
and well-foundedness reasoning depends on it.

---

### `Blocklace`

```lean
def Blocklace := Finmap (fun _ : BlockId => Block)
```

A finite map from block IDs to blocks. Corresponds to the `HashMap<BlockIdentity, BlockContent>`
in Rust (`blocklace.rs:14`), but with the `id_eq` invariant tracked per block.

---

### `Closed`

```lean
def Closed (B : Blocklace) : Prop :=
  ∀ id blk,
    B.lookup id = some blk →
      ∀ p ∈ blk.content.predecessors,
        p ∈ B.keys
```

`Closed B` holds iff every predecessor referenced by any block in `B` is itself present in
`B`. This is the **CLOSED** invariant from Definition 2.3: no dangling pointers.

**Rust:** `Blocklace::is_closed` (`blocklace.rs:183–191`); enforced at insert time
(`blocklace.rs:159–165`).

---

### `Observes`

```lean
def DirectPred (B : Blocklace) (b p : BlockId) : Prop :=
  ∃ blk : Block,
    B.lookup b = some blk ∧
    p ∈ blk.content.predecessors

def Observes (B : Blocklace) : BlockId → BlockId → Prop :=
  Relation.ReflTransGen (DirectPred B)
```

`DirectPred B b p`: `p` is a direct predecessor of `b`.
`Observes B a b`: `a` can reach `b` by following zero or more predecessor edges backward.

**Rust:**
- `DirectPred` ↔ `Blocklace::predecessors` (`blocklace.rs:198–203`)
- `Observes`    ↔ `Blocklace::observe` + `Blocklace::precedes_or_equals` (`blocklace.rs:230–257`, `284–286`)

---

### `Precedes` and `PrecedesOrEquals`

```lean
-- strict: at least one predecessor edge
def Precedes (B : Blocklace) (a b : BlockId) : Prop :=
  Relation.TransGen (DirectPred B) b a

-- reflexive: zero or more edges (= Observes with args swapped)
def PrecedesOrEquals (B : Blocklace) (a b : BlockId) : Prop :=
  Observes B b a
```

**Rust:**
- `Precedes`         ↔ `Blocklace::precedes`            (`blocklace.rs:277–281`)
- `PrecedesOrEquals` ↔ `Blocklace::preceedes_or_equals`  (`blocklace.rs:284–286`)

## 2. Trusted Boundary: Hash Injectivity

```lean
axiom hashInj {c1 c2 : BlockContent}
    (h : hashContent c1 = hashContent c2) : c1 = c2
```

This is stated explicitly as a **trusted boundary**, not proved. It represents the
collision-resistance of the cryptographic hash function used in production. All acyclicity
reasoning (`directPred_wf`, `directPred_acyclic`, `observes_antisymm`) traces back to this
assumption.

**Future work:** replace `directPred_wf` with a constructive proof once blocks carry a
rank/depth field that strictly decreases along predecessor edges.

## 3. Key Theorems

### 3.1 Insertion Preserves `Closed`

```lean
theorem insertPreservesClosed
    (B : Blocklace) (blk : Block)
    (hClosed : Closed B)
    (hInsertable : Insertable B blk) :
    Closed (blocklaceInsert B blk)
```

Any protocol layer that calls `insert` on an already-closed blocklace with an insertable block
can assume the resulting blocklace is still closed. This is what makes every downstream proof
about "any closed blocklace" applicable after every insertion.

**Rust:** correctness of `Blocklace::insert` (`blocklace.rs:148–170`).

---

### 3.2 Insertion Requires Predecessors (negative lemma)

```lean
theorem insertRequiresPredecessors
    (B : Blocklace) (blk : Block) (p : BlockId)
    (hpPred    : p ∈ blk.content.predecessors)
    (hpMissing : p ∉ B.keys)
    (hpNotSelf : p ≠ blk.id) :
    ¬ Closed (blocklaceInsert B blk)
```

Dual to the above: if a predecessor is missing, inserting the block violates closure.
Formalizes the error path of `Blocklace::insert` (`blocklace.rs:160–164`).

---

### 3.3 Acyclicity / Well-Foundedness

```lean
axiom directPred_wf
    (B : Blocklace) (hClosed : Closed B) :
    WellFounded (DirectPred B)

theorem directPred_acyclic
    (B : Blocklace) (hClosed : Closed B)
    {a : BlockId}
    (hcycle : Relation.TransGen (DirectPred B) a a) :
    False
```

`directPred_wf` is a trusted well-foundedness assumption encoding the DAG invariant.
`directPred_acyclic` is a proved consequence: no block can be its own ancestor.

**Why it matters:** every fixed-point argument (finality safety, ordering convergence) relies
on the observation relation being a DAG. A cycle would make "deepest block" ill-defined.

---

### 3.4 `Observes` is a Partial Order

```lean
theorem observes_refl      : Observes B a a
theorem observes_trans      : Observes B a b → Observes B b c → Observes B a c
theorem observes_antisymm   : Closed B → Observes B a b → Observes B b a → a = b
theorem observes_partialOrder :
    (∀ a, Observes B a a) ∧
    (∀ a b c, Observes B a b → Observes B b c → Observes B a c) ∧
    (∀ a b, Observes B a b → Observes B b a → a = b)
```

**Why antisymmetry matters:** if `a` observes `b` and `b` observes `a` with `a ≠ b`, a
directed cycle `a →* b →* a` would exist. `directPred_acyclic` rules this out. Antisymmetry
is the property that formally excludes a block "seeing" a block that sees it back.

---

### 3.5 Cone Monotonicity

```lean
theorem observes_mono
    (B B' : Blocklace)
    (hSub : ∀ id blk, B.lookup id = some blk → B'.lookup id = some blk)
    {a b : BlockId}
    (h : Observes B a b) :
    Observes B' a b
```

Blocklaces only grow (blocks are never removed in the consensus path). Cone monotonicity
formalizes: what you could see before, you can still see after new blocks are added. Used by
any argument of the form "once approved, always approved."

---
### 3.6 `observeSet` Soundness, Completeness, Equivalence

```lean
noncomputable def observeSet (B : Blocklace) (a : BlockId) : Finset BlockId :=
  letI : DecidablePred (Observes B a) := fun _ => Classical.propDecidable _
  B.keys.filter (Observes B a)

theorem observeSet_sound    : p ∈ observeSet B a → Observes B a p
theorem observeSet_complete : p ∈ B.keys → Observes B a p → p ∈ observeSet B a
theorem observeSet_equiv    : p ∈ B.keys → (p ∈ observeSet B a ↔ Observes B a p)
```

`observeSet` is `noncomputable` because `Observes` is `Prop`-valued and `Finset.filter`
needs classical decidability. The sound/complete pair provides the bidirectional bridge
between the finite-set representation (`Finset BlockId`) and the logical `Observes` relation.

**Rust:** `Blocklace::observe` (`blocklace.rs:230–257`),
`Blocklace::ancestors_inclusive` (`blocklace.rs:261–267`).

---

### 3.7 Strict Predecessor Implies Observation

```lean
theorem precedes_implies_observes
    (B : Blocklace) {a b : BlockId}
    (h : Precedes B a b) :
    Observes B b a
```

Every strict predecessor is observed (at least one edge ⊆ zero-or-more edges).

---

## 4. Rust Mapping Table

| Lean definition / theorem | Rust function | File | Lines |
|---|---|---|---|
| `BlockId` | `BlockIdentity` | `types/identity_id.rs` | 15–25 |
| `BlockContent` | `BlockContent` | `types/content_id.rs` | 10–22 |
| `Block` | `Block` | `block.rs` | 15–40 |
| `Blocklace` | `Blocklace.blocks` | `blocklace.rs` | 13–21 |
| `Closed B` | `Blocklace::is_closed` | `blocklace.rs` | 183–191 |
| `Insertable B blk` | predecessor check in `insert` | `blocklace.rs` | 159–165 |
| `blocklaceInsert` | `commit_validated` | `blocklace.rs` | 58–61 |
| `insertPreservesClosed` | correctness of `insert` gate | `blocklace.rs` | 148–170 |
| `insertRequiresPredecessors` | error path of `insert` | `blocklace.rs` | 160–164 |
| `hashContent` | content hash in `BlockIdentity` | `types/content_id.rs` | — |
| `hashInj` | collision-resistance (trusted) | *(axiom)* | — |
| `DirectPred B b p` | `Blocklace::predecessors` | `blocklace.rs` | 198–203 |
| `Observes B a b` | `Blocklace::precedes_or_equals` | `blocklace.rs` | 284–286 |
| `observeSet B a` | `Blocklace::observe` | `blocklace.rs` | 230–257 |
| `Precedes B a b` | `Blocklace::precedes` | `blocklace.rs` | 277–281 |
| `PrecedesOrEquals B a b` | `Blocklace::preceedes_or_equals` | `blocklace.rs` | 284–286 |
| `directPred_wf` | DAG invariant (no cycles, trusted) | *(axiom)* | — |
| `directPred_acyclic` | DAG acyclicity consequence | — | — |
| `observes_antisymm` | partial order / no mutual observation | Definition 2.2 | — |
| `observes_mono` | monotonicity of observation cone | `blocklace.rs` | 230–257 |