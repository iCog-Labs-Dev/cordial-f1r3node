/-
Canonical Rust trace schema and strict NDJSON parser.

Every Rust `TraceEvent` variant has a distinct Lean payload type. Parsing is
performed by Lean's JSON parser; malformed JSON, unknown events, missing
fields, wrong field types, and unknown fields are errors. Nullable Rust fields
remain `Option` values, so `null` is never confused with zero or an empty
string.
-/

import Lean.Data.Json

namespace CordialMiners

open Lean

structure CreateBlockEvent where
  nodeId : String
  wave : Option Nat
  round : Option Nat
  blockHash : String
  parentHashes : List String
  missingParentHashes : List String
  creator : String
  weightTableHash : Option String
  deriving Repr, DecidableEq

structure ValidateBlockEvent where
  nodeId : String
  wave : Option Nat
  round : Option Nat
  blockHash : String
  parentHashes : List String
  creator : String
  weightTableHash : Option String
  outcome : String
  errors : List String
  deriving Repr, DecidableEq

structure InsertBlockEvent where
  nodeId : String
  wave : Option Nat
  round : Option Nat
  blockHash : String
  parentHashes : List String
  missingParentHashes : List String
  creator : String
  weightTableHash : Option String
  deriving Repr, DecidableEq

structure BufferBlockEvent where
  nodeId : String
  wave : Option Nat
  round : Option Nat
  blockHash : String
  parentHashes : List String
  missingParentHashes : List String
  creator : String
  weightTableHash : Option String
  deriving Repr, DecidableEq

structure ResolveMissingParentEvent where
  nodeId : String
  blockHash : String
  resolvedParentHash : String
  deriving Repr, DecidableEq

structure DetectEquivocationEvent where
  nodeId : String
  equivocator : String
  round : Nat
  conflictingBlockHashes : List String
  deriving Repr, DecidableEq

structure AcceptApprovalEvent where
  nodeId : String
  wave : Option Nat
  round : Nat
  approver : String
  approverHash : String
  targetHash : String
  deriving Repr, DecidableEq

structure BuildThresholdCertificateEvent where
  nodeId : String
  wave : Option Nat
  kind : String
  leaderHash : String
  ratifierHash : Option String
  certificateId : String
  approverHashes : List String
  approvers : List String
  approverCount : Nat
  approverWeight : Nat
  totalWeight : Nat
  weightTableHash : String
  deriving Repr, DecidableEq

structure ComputeFinalityEvent where
  nodeId : String
  wave : Nat
  wavelength : Nat
  blockHash : String
  decision : String
  certificateId : Option String
  outputPrefixHash : Option String
  weightTableHash : Option String
  deriving Repr, DecidableEq

structure RunTauOrderEvent where
  nodeId : String
  wave : Nat
  wavelength : Nat
  latestLeaderHash : String
  orderedBlockHashes : List String
  outputLen : Nat
  deriving Repr, DecidableEq

structure EmitOutputEvent where
  nodeId : String
  wave : Nat
  blockHash : String
  outputIndex : Nat
  outputPrefixHash : String
  deriving Repr, DecidableEq

structure SendPackageEvent where
  nodeId : String
  peerId : String
  blockHashes : List String
  deriving Repr, DecidableEq

structure DeliverPackageEvent where
  nodeId : String
  peerId : String
  blockHashes : List String
  deriving Repr, DecidableEq

structure SchedulerTickEvent where
  nodeId : String
  tick : Nat
  wave : Option Nat
  deriving Repr, DecidableEq

structure RunWaveTaskEvent where
  nodeId : String
  wave : Nat
  task : String
  deriving Repr, DecidableEq

inductive TraceEvent where
  | createBlock : CreateBlockEvent → TraceEvent
  | validateBlock : ValidateBlockEvent → TraceEvent
  | insertBlock : InsertBlockEvent → TraceEvent
  | bufferBlock : BufferBlockEvent → TraceEvent
  | resolveMissingParent : ResolveMissingParentEvent → TraceEvent
  | detectEquivocation : DetectEquivocationEvent → TraceEvent
  | acceptApproval : AcceptApprovalEvent → TraceEvent
  | buildThresholdCertificate : BuildThresholdCertificateEvent → TraceEvent
  | computeFinality : ComputeFinalityEvent → TraceEvent
  | runTauOrder : RunTauOrderEvent → TraceEvent
  | emitOutput : EmitOutputEvent → TraceEvent
  | sendPackage : SendPackageEvent → TraceEvent
  | deliverPackage : DeliverPackageEvent → TraceEvent
  | schedulerTick : SchedulerTickEvent → TraceEvent
  | runWaveTask : RunWaveTaskEvent → TraceEvent
  deriving Repr, DecidableEq

/-- Stable canonical event name used in first-mismatch diagnostics. -/
def TraceEvent.kind : TraceEvent → String
  | .createBlock _ => "create_block"
  | .validateBlock _ => "validate_block"
  | .insertBlock _ => "insert_block"
  | .bufferBlock _ => "buffer_block"
  | .resolveMissingParent _ => "resolve_missing_parent"
  | .detectEquivocation _ => "detect_equivocation"
  | .acceptApproval _ => "accept_approval"
  | .buildThresholdCertificate _ => "build_threshold_certificate"
  | .computeFinality _ => "compute_finality"
  | .runTauOrder _ => "run_tau_order"
  | .emitOutput _ => "emit_output"
  | .sendPackage _ => "send_package"
  | .deliverPackage _ => "deliver_package"
  | .schedulerTick _ => "scheduler_tick"
  | .runWaveTask _ => "run_wave_task"

private def requireOnlyKeys (json : Json) (allowed : List String) : Except String Unit := do
  let object ← json.getObj?
  match object.toList.find? (fun field => !allowed.contains field.1) with
  | some (key, _) => throw s!"unknown field '{key}'"
  | none => pure ()

private def field (json : Json) (key : String) [FromJson α] : Except String α :=
  json.getObjValAs? α key

/-- `FromJson (Option α)` treats a missing member like JSON `null`.  That is
useful for ordinary configuration files, but not for a canonical interchange
schema: omission and an explicitly encoded `null` are different wire values.
Check membership before decoding every required nullable field. -/
private def optionField (json : Json) (key : String) [FromJson α] : Except String (Option α) := do
  let object ← json.getObj?
  if object.toList.any (fun entry => entry.1 == key) then
    field json key
  else
    throw s!"missing field '{key}'"

private structure LifecycleFields where
  nodeId : String
  wave : Option Nat
  round : Option Nat
  blockHash : String
  parentHashes : List String
  missingParentHashes : List String
  creator : String
  weightTableHash : Option String

private def parseLifecycle (json : Json) : Except String LifecycleFields := do
  let nodeId ← field json "node_id"
  let wave ← optionField json "wave"
  let round ← optionField json "round"
  let blockHash ← field json "block_hash"
  let parentHashes ← field json "parent_hashes"
  let missingParentHashes ← field json "missing_parent_hashes"
  let creator ← field json "creator"
  let weightTableHash ← optionField json "weight_table_hash"
  pure (LifecycleFields.mk nodeId wave round blockHash parentHashes
    missingParentHashes creator weightTableHash)

private def lifecycleKeys := ["event", "node_id", "wave", "round", "block_hash",
  "parent_hashes", "missing_parent_hashes", "creator", "weight_table_hash"]

private def parseEventJson (json : Json) : Except String TraceEvent := do
  let event ← field json "event"
  match event with
  | "create_block" =>
      requireOnlyKeys json lifecycleKeys
      let x ← parseLifecycle json
      pure (.createBlock ⟨x.nodeId, x.wave, x.round, x.blockHash, x.parentHashes,
        x.missingParentHashes, x.creator, x.weightTableHash⟩)
  | "validate_block" =>
      requireOnlyKeys json ["event", "node_id", "wave", "round", "block_hash",
        "parent_hashes", "creator", "weight_table_hash", "outcome", "errors"]
      pure (.validateBlock ⟨← field json "node_id", ← optionField json "wave",
        ← optionField json "round", ← field json "block_hash", ← field json "parent_hashes",
        ← field json "creator", ← optionField json "weight_table_hash", ← field json "outcome",
        ← field json "errors"⟩)
  | "insert_block" =>
      requireOnlyKeys json lifecycleKeys
      let x ← parseLifecycle json
      pure (.insertBlock ⟨x.nodeId, x.wave, x.round, x.blockHash, x.parentHashes,
        x.missingParentHashes, x.creator, x.weightTableHash⟩)
  | "buffer_block" =>
      requireOnlyKeys json lifecycleKeys
      let x ← parseLifecycle json
      pure (.bufferBlock ⟨x.nodeId, x.wave, x.round, x.blockHash, x.parentHashes,
        x.missingParentHashes, x.creator, x.weightTableHash⟩)
  | "resolve_missing_parent" =>
      requireOnlyKeys json ["event", "node_id", "block_hash", "resolved_parent_hash"]
      pure (.resolveMissingParent ⟨← field json "node_id", ← field json "block_hash",
        ← field json "resolved_parent_hash"⟩)
  | "detect_equivocation" =>
      requireOnlyKeys json ["event", "node_id", "equivocator", "round",
        "conflicting_block_hashes"]
      pure (.detectEquivocation ⟨← field json "node_id", ← field json "equivocator",
        ← field json "round", ← field json "conflicting_block_hashes"⟩)
  | "accept_approval" =>
      requireOnlyKeys json ["event", "node_id", "wave", "round", "approver",
        "approver_hash", "target_hash"]
      pure (.acceptApproval ⟨← field json "node_id", ← optionField json "wave",
        ← field json "round", ← field json "approver", ← field json "approver_hash",
        ← field json "target_hash"⟩)
  | "build_threshold_certificate" =>
      requireOnlyKeys json ["event", "node_id", "wave", "kind", "leader_hash",
        "ratifier_hash", "certificate_id", "approver_hashes", "approvers",
        "approver_count", "approver_weight", "total_weight", "weight_table_hash"]
      pure (.buildThresholdCertificate ⟨← field json "node_id", ← optionField json "wave",
        ← field json "kind", ← field json "leader_hash", ← optionField json "ratifier_hash",
        ← field json "certificate_id", ← field json "approver_hashes",
        ← field json "approvers", ← field json "approver_count",
        ← field json "approver_weight", ← field json "total_weight",
        ← field json "weight_table_hash"⟩)
  | "compute_finality" =>
      requireOnlyKeys json ["event", "node_id", "wave", "wavelength", "block_hash",
        "decision", "certificate_id", "output_prefix_hash", "weight_table_hash"]
      pure (.computeFinality ⟨← field json "node_id", ← field json "wave",
        ← field json "wavelength", ← field json "block_hash", ← field json "decision",
        ← optionField json "certificate_id", ← optionField json "output_prefix_hash",
        ← optionField json "weight_table_hash"⟩)
  | "run_tau_order" =>
      requireOnlyKeys json ["event", "node_id", "wave", "wavelength",
        "latest_leader_hash", "ordered_block_hashes", "output_len"]
      pure (.runTauOrder ⟨← field json "node_id", ← field json "wave",
        ← field json "wavelength", ← field json "latest_leader_hash",
        ← field json "ordered_block_hashes", ← field json "output_len"⟩)
  | "emit_output" =>
      requireOnlyKeys json ["event", "node_id", "wave", "block_hash", "output_index",
        "output_prefix_hash"]
      pure (.emitOutput ⟨← field json "node_id", ← field json "wave",
        ← field json "block_hash", ← field json "output_index",
        ← field json "output_prefix_hash"⟩)
  | "send_package" =>
      requireOnlyKeys json ["event", "node_id", "peer_id", "block_hashes"]
      pure (.sendPackage ⟨← field json "node_id", ← field json "peer_id",
        ← field json "block_hashes"⟩)
  | "deliver_package" =>
      requireOnlyKeys json ["event", "node_id", "peer_id", "block_hashes"]
      pure (.deliverPackage ⟨← field json "node_id", ← field json "peer_id",
        ← field json "block_hashes"⟩)
  | "scheduler_tick" =>
      requireOnlyKeys json ["event", "node_id", "tick", "wave"]
      pure (.schedulerTick ⟨← field json "node_id", ← field json "tick",
        ← optionField json "wave"⟩)
  | "run_wave_task" =>
      requireOnlyKeys json ["event", "node_id", "wave", "task"]
      pure (.runWaveTask ⟨← field json "node_id", ← field json "wave",
        ← field json "task"⟩)
  | other => throw s!"unknown event '{other}'"

/-- Parse one canonical JSON event. -/
def parseLine (line : String) : Except String TraceEvent := do
  let json ← Json.parse line
  parseEventJson json

/-- Read canonical newline-delimited JSON. A final line terminator is allowed,
but an empty record in the middle of a trace is an error rather than a silently
discarded event. Every record must be one valid, known event, and the first
error includes its one-based line number. -/
def readTraceFile (path : String) : IO (Except String (List TraceEvent)) := do
  let content ← IO.FS.readFile path
  let rec go (lineNo : Nat) (lines : List String) (events : List TraceEvent) :=
    match lines with
    | [] => .ok events.reverse
    | [line] =>
        if line.isEmpty then .ok events.reverse
        else match parseLine line with
          | .ok event => .ok (event :: events).reverse
          | .error reason => .error s!"line {lineNo}: {reason}"
    | line :: rest =>
        if line.isEmpty then .error s!"line {lineNo}: empty trace record"
        else
          match parseLine line with
          | .ok event => go (lineNo + 1) rest (event :: events)
          | .error reason => .error s!"line {lineNo}: {reason}"
  pure (go 1 (content.splitOn "\n") [])

end CordialMiners
