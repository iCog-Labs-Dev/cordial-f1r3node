/-
The block model: a single blocklace block and its well-formedness
conditions (author, parents, payload).

Mirrors the Rust types:
  - `BlockIdentity` → `BlockId`       (types/identity_id.rs)
  - `BlockContent`  → `BlockContent`  (types/content_id.rs)
  - `Block`         → `Block`         (block.rs)

The cryptographic hash is abstracted by an executable injective encoding.
Trace replay constructs formal content from the checked creator/predecessor
graph and a compact opaque payload tag, then records a separate one-to-one
association between the resulting `BlockId` and the concrete Rust digest. It
does not claim this natural-number encoding is the fixed-width Rust hash.

Owned by Issue 02 (KR1 — Blocklace Core).
-/

import Mathlib.Data.Finset.Basic
import Mathlib.Logic.Equiv.Finset

namespace CordialMiners

/-! ### Primitive identifiers -/

/-- Opaque node identifier — corresponds to `NodeId` in node_id.rs. -/
abbrev NodeId := Nat

/-- Opaque block identifier.
In Rust this is represented by `BlockIdentity`, which contains the
content hash, creator, and signature. At the Lean abstraction level,
we treat it as an opaque natural number.
-/
abbrev BlockId := Nat

/-! ### Block Content -/

/-- Block content `C = (v, P)`.

`payload` corresponds to the arbitrary value `v`, while `predecessors`
is the set `P` of predecessor block identities.
-/
structure BlockContent where
  payload      : List UInt8
  predecessors : Finset BlockId
  deriving DecidableEq

private def uint8Equiv : UInt8 ≃ Fin 256 where
  toFun value := ⟨value.toNat, value.toNat_lt⟩
  invFun value := UInt8.ofNat value.val
  left_inv value := UInt8.toNat_inj.mp (by simp)
  right_inv value := Fin.ext (by simp)

private instance : Encodable UInt8 :=
  Encodable.ofEquiv (Fin 256) uint8Equiv

private def blockContentEquiv : BlockContent ≃ List UInt8 × Finset BlockId where
  toFun content := (content.payload, content.predecessors)
  invFun data := { payload := data.1, predecessors := data.2 }
  left_inv _ := rfl
  right_inv _ := rfl

instance : Encodable BlockContent :=
  Encodable.ofEquiv (List UInt8 × Finset BlockId) blockContentEquiv

/-! ### Block Identity / Hash -/

/-- Executable injective abstraction of the signed content hash.

Corresponds to paper §2.2: i = signedhash((v, P), k_p).
The concrete Rust digest remains an external identifier in the trace adapter;
this encoding supplies the collision-free identifier used by the formal DAG.
-/
def hashContent (creator : NodeId) (content : BlockContent) : BlockId :=
  Encodable.encode (creator, content)

/--
Hash injectivity for the executable abstraction. Unlike a cryptographic
collision-resistance assumption, this follows from `Encodable.encode`.
-/
theorem hashInj {n1 n2 : NodeId} {c1 c2 : BlockContent}
    (h : hashContent n1 c1 = hashContent n2 c2) : n1 = n2 ∧ c1 = c2 := by
  have hp : (n1, c1) = (n2, c2) := Encodable.encode_injective h
  exact ⟨congrArg Prod.fst hp, congrArg Prod.snd hp⟩

/-! ### Block -/

/--
A single blocklace block.

Every formal block identifier is the executable injective encoding of its
creator and content. The trace adapter separately retains the concrete Rust
digest and rejects duplicate digest declarations; the trace intentionally does
not expose enough bytes to reproduce Rust's cryptographic digest itself.
-/
structure Block where
  id        : BlockId
  creator   : NodeId
  content   : BlockContent
  id_eq     : id = hashContent creator content
  deriving DecidableEq

end CordialMiners
