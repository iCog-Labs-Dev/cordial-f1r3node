/-
The block model: a single blocklace block and its well-formedness
conditions (author, parents, payload).

Mirrors the Rust types:
  - `BlockIdentity` → `BlockId`       (types/identity_id.rs)
  - `BlockContent`  → `BlockContent`  (types/content_id.rs)
  - `Block`         → `Block`         (block.rs)

Hash injectivity is stated as an explicit axiom (`hashInj`), a trusted
boundary, not something proved here. All acyclicity reasoning downstream
depends on it.

Owned by Issue 02 (KR1 — Blocklace Core).
-/

import Mathlib.Data.Finset.Basic

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

/-! ### Block Identity / Hash -/

/-- Abstract hash function mapping creator identity and block content to a block identifier.

Corresponds to paper §2.2: i = signedhash((v, P), k_p).
Including creator NodeId ensures two distinct nodes creating identical content yield
distinct block identifiers (matching Rust BlockIdentity).
-/
opaque hashContent : NodeId → BlockContent → BlockId

/--
Hash injectivity assumption.

This is a trusted mathematical boundary: if two blocks have the same block ID,
then their creator node IDs and block contents are equal.

All downstream acyclicity reasoning relies on this assumption.
-/
axiom hashInj {n1 n2 : NodeId} {c1 c2 : BlockContent}
    (h : hashContent n1 c1 = hashContent n2 c2) : n1 = n2 ∧ c1 = c2

/-! ### Block -/

/--
A single blocklace block.

A block consists of:
* an identifier
* the node that created it
* its content (payload and predecessors)
* a well-formedness proof that its identifier is the signed hash of its creator and content
-/
structure Block where
  id        : BlockId
  creator   : NodeId
  content   : BlockContent
  id_eq     : id = hashContent creator content
  deriving DecidableEq

end CordialMiners
