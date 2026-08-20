/-
The blocklace: the finite DAG of blocks.

Mirrors:
  crates/cordial-miners-core/src/blocklace.rs

The key invariant is CLOSED:
every predecessor referenced by a block must already exist
in the blocklace.

Owned by Issue 02 (KR1 — Blocklace Core).
-/

import LeanVerification.Block
import Mathlib.Data.Finmap

namespace CordialMiners

open Finmap

/-- A blocklace is a finite map from block IDs to blocks. -/
def Blocklace := Finmap (fun _ : BlockId => Block)

/--
Every predecessor referenced by a block in the blocklace
must itself be present in the blocklace.

Corresponds to Rust `Blocklace::is_closed`.
-/
def Closed (B : Blocklace) : Prop :=
  ∀ id blk,
    B.lookup id = some blk →
      ∀ p ∈ blk.content.predecessors,
        p ∈ B.keys

/--
A block is insertable iff all of its predecessors are
already present in the blocklace.
-/
def Insertable (B : Blocklace) (blk : Block) : Prop :=
  ∀ p ∈ blk.content.predecessors,
    p ∈ B.keys

/--
Raw insertion into the blocklace.
-/
def blocklaceInsert (B : Blocklace) (blk : Block) : Blocklace :=
  B.insert blk.id blk

/--
If `B` is closed and all predecessors of `blk` are already
present, inserting `blk` preserves closure.
-/
theorem insertPreservesClosed
    (B : Blocklace) (blk : Block)
    (hClosed : Closed B)
    (hInsertable : Insertable B blk) :
    Closed (blocklaceInsert B blk) := by
  have step : ∀ {q}, q ∈ B.keys → q ∈ (blocklaceInsert B blk).keys :=
    fun hq => mem_keys.mpr (mem_insert.mpr (Or.inr (mem_keys.mp hq)))
  intro id b hlookup p hp
  rcases eq_or_ne id blk.id with rfl | hid
  · have h : lookup blk.id (B.insert blk.id blk) = some b := hlookup
    rw [lookup_insert B] at h; cases h
    exact step (hInsertable p hp)
  · have h : lookup id (B.insert blk.id blk) = some b := hlookup
    rw [lookup_insert_of_ne B hid] at h
    exact step (hClosed id b h p hp)

/--
If a block has a predecessor that is missing from `B.keys` (and not equal to the block itself),
then inserting it violates the closure invariant.
-/
theorem insertRequiresPredecessors
    (B : Blocklace) (blk : Block) (p : BlockId)
    (hpPred : p ∈ blk.content.predecessors)
    (hpMissing : p ∉ B.keys)
    (hpNotSelf : p ≠ blk.id) :
    ¬ Closed (blocklaceInsert B blk) := by
  intro hClosed'
  have hpPresent := hClosed' blk.id blk (lookup_insert B) p hpPred
  exact hpMissing (mem_keys.mpr ((mem_insert.mp (mem_keys.mp hpPresent)).resolve_left hpNotSelf))

end CordialMiners
