/-
Canonical trace replay against the executable formal reference model.

The replay state contains a proof-valid KR1 `Blocklace`. Finality,
equivocation, approval, ratification, and super-ratification are recomputed by
`CMRef`; Rust's result strings and certificate totals are only observations to
compare with the independently calculated values.
-/

import LeanVerification.Trace
import LeanVerification.CMRef
import LeanVerification.Ordering

namespace CordialMiners

open Lean

structure WeightEntry where
  nodeId : String
  weight : Nat
  deriving Repr, DecidableEq

structure LeaderEntry where
  wave : Nat
  nodeId : String
  deriving Repr, DecidableEq

structure ReplayConfig where
  hashAlgorithm : String
  weightTableHash : String
  wavelength : Nat
  validators : List WeightEntry
  leaders : List LeaderEntry
  deriving Repr, DecidableEq

private def jsonField (json : Json) (key : String) [FromJson α] : Except String α :=
  json.getObjValAs? α key

private def requireJsonKeys (json : Json) (allowed : List String) : Except String Unit := do
  let object ← json.getObj?
  match object.toList.find? (fun field => !allowed.contains field.1) with
  | some (key, _) => throw s!"unknown configuration field '{key}'"
  | none => pure ()

private def parseWeightEntry (json : Json) : Except String WeightEntry := do
  requireJsonKeys json ["node_id", "weight"]
  pure ⟨← jsonField json "node_id", ← jsonField json "weight"⟩

private def parseLeaderEntry (json : Json) : Except String LeaderEntry := do
  requireJsonKeys json ["wave", "node_id"]
  pure ⟨← jsonField json "wave", ← jsonField json "node_id"⟩

private def parseConfigJson (json : Json) : Except String ReplayConfig := do
  requireJsonKeys json ["hash_algorithm", "weight_table_hash", "wavelength",
    "validators", "leaders"]
  let rawWeights : Array Json ← jsonField json "validators"
  let rawLeaders : Array Json ← jsonField json "leaders"
  let validators : Array WeightEntry ← rawWeights.mapM parseWeightEntry
  let leaders : Array LeaderEntry ← rawLeaders.mapM parseLeaderEntry
  pure ⟨← jsonField json "hash_algorithm", ← jsonField json "weight_table_hash",
    ← jsonField json "wavelength", validators.toList, leaders.toList⟩

private def fnvStep (hash : UInt64) (byte : Nat) : UInt64 :=
  (hash ^^^ UInt64.ofNat byte) * (0x00000100000001b3 : UInt64)

private def fnvString (hash : UInt64) (value : String) : UInt64 :=
  value.toList.foldl (fun acc char => fnvStep acc char.toNat) hash

private def hexDigit (n : Nat) : Char :=
  if n < 10 then Char.ofNat (48 + n) else Char.ofNat (87 + n)

private def uint64Hex (value : UInt64) : String :=
  String.ofList <| (List.range 16).reverse.map fun shift =>
    hexDigit ((value.toNat / (16 ^ shift)) % 16)

def computeWeightTableHash (config : ReplayConfig) : String :=
  let hash := config.validators.foldl (fun acc entry =>
    fnvString acc s!"{entry.nodeId}:{entry.weight}\n")
    (0xcbf29ce484222325 : UInt64)
  uint64Hex hash

def computeOutputPrefixHash (hashes : List String) : String :=
  let hash := hashes.foldl (fun acc blockHash =>
    fnvStep (fnvString acc blockHash) 255) (0xcbf29ce484222325 : UInt64)
  uint64Hex hash

def computeCertificateId (kind leaderHash : String) (ratifierHash : Option String) : String :=
  let h0 := fnvString (0xcbf29ce484222325 : UInt64) kind
  let h1 := fnvStep h0 255
  let h2 := fnvString h1 leaderHash
  let h3 := fnvStep h2 255
  uint64Hex (fnvString h3 (ratifierHash.getD ""))

private def strictlySortedUnique (values : List String) : Bool :=
  values.Pairwise (fun left right => left < right) |> decide

def validateConfig (config : ReplayConfig) : Except String ReplayConfig := do
  if config.hashAlgorithm != "fnv1a64-v1" then
    throw s!"unsupported weight hash algorithm '{config.hashAlgorithm}'"
  if config.wavelength == 0 then throw "wavelength must be positive"
  let nodes := config.validators.map WeightEntry.nodeId
  if !strictlySortedUnique nodes then
    throw "validator table must be strictly sorted with unique node_id values"
  if config.validators.any (fun entry => entry.weight == 0) then
    throw "validator weights must be positive"
  let leaderWaves := config.leaders.map LeaderEntry.wave
  if !(decide (leaderWaves.Pairwise (fun left right => left < right))) then
    throw "leader waves must be strictly sorted and unique"
  if config.leaders.any (fun entry => !nodes.contains entry.nodeId) then
    throw "leader table names a node absent from the validator table"
  let actual := computeWeightTableHash config
  if actual != config.weightTableHash then
    throw s!"weight table hash mismatch: file={config.weightTableHash}, Lean={actual}"
  pure config

def readReplayConfig (path : String) : IO (Except String ReplayConfig) := do
  let content ← IO.FS.readFile path
  pure <| do
    let json ← Json.parse content
    validateConfig (← parseConfigJson json)

private def nodeIndexAux (node : String) : List WeightEntry → Nat → Option NodeId
  | [], _ => none
  | entry :: rest, index =>
      if entry.nodeId == node then some index else nodeIndexAux node rest (index + 1)

def ReplayConfig.nodeIndex? (config : ReplayConfig) (node : String) : Option NodeId :=
  nodeIndexAux node config.validators 0

def ReplayConfig.validatorSet (config : ReplayConfig) : Finset NodeId :=
  Finset.range config.validators.length

def ReplayConfig.bonds (config : ReplayConfig) (node : NodeId) : Nat :=
  (config.validators[node]?).map WeightEntry.weight |>.getD 0

def ReplayConfig.leader (config : ReplayConfig) (wave : Nat) : Option NodeId := do
  let entry ← config.leaders.find? (fun item => item.wave == wave)
  config.nodeIndex? entry.nodeId

structure ReplayedBlock where
  rustHash : String
  formalId : BlockId
  creator : String
  round : Nat
  parentHashes : List String

structure ReplayDag where
  blocklace : Blocklace
  valid : ValidBlocklace blocklace
  blocks : List ReplayedBlock

def ReplayDag.empty : ReplayDag :=
  { blocklace := emptyBlocklace, valid := ValidBlocklace.empty, blocks := [] }

def ReplayDag.findHash? (dag : ReplayDag) (hash : String) : Option ReplayedBlock :=
  dag.blocks.find? (fun block => block.rustHash == hash)

def ReplayDag.formalId? (dag : ReplayDag) (hash : String) : Option BlockId :=
  (dag.findHash? hash).map ReplayedBlock.formalId

def ReplayDag.findFormal? (dag : ReplayDag) (id : BlockId) : Option ReplayedBlock :=
  dag.blocks.find? (fun block => block.formalId == id)

private def resolveHashes (dag : ReplayDag) (hashes : List String) : Except String (List BlockId) :=
  hashes.mapM fun hash =>
    match dag.formalId? hash with
    | some id => pure id
    | none => throw s!"unknown block hash '{hash}'"

private def insertBlock (config : ReplayConfig) (dag : ReplayDag)
    (event : InsertBlockEvent) : Except String ReplayDag := do
  if dag.findHash? event.blockHash |>.isSome then
    throw s!"duplicate insert for block {event.blockHash}"
  if !event.missingParentHashes.isEmpty then
    throw s!"insert declares unresolved parents {event.missingParentHashes}"
  if event.parentHashes.eraseDups.length != event.parentHashes.length then
    throw s!"block {event.blockHash} repeats a predecessor"
  let creator ← match config.nodeIndex? event.creator with
    | some creator => pure creator
    | none => throw s!"creator '{event.creator}' is absent from the weight table"
  let parents ← resolveHashes dag event.parentHashes
  let reportedRound ← match event.round with
    | some round => pure round
    | none => throw s!"inserted block {event.blockHash} has null round"
  let opaqueTag := toString dag.blocks.length
  let content : BlockContent :=
    -- The trace omits application payload bytes. Use an injective, compact
    -- insertion tag as the opaque payload abstraction. `ReplayedBlock` keeps
    -- the checked bijection to the real Rust digest separately.
    { payload := opaqueTag.toList.map (fun char => UInt8.ofNat char.toNat),
      predecessors := parents.toFinset }
  let block : Block :=
    { id := hashContent creator content, creator := creator, content := content, id_eq := rfl }
  if hParents : content.predecessors ⊆ dag.blocklace.keys then
    if hNew : block.id ∉ dag.blocklace.keys then
      let blocklace' := blocklaceInsert dag.blocklace block
      let valid' : ValidBlocklace blocklace' :=
        ValidBlocklace.insert dag.blocklace block dag.valid hParents hNew
      let actualRound := blockDepth blocklace' valid' block.id
      if actualRound != reportedRound then
        throw s!"round mismatch for {event.blockHash}: Rust={reportedRound}, Lean={actualRound}"
      match event.wave with
      | some wave =>
          let actualWave := waveOfRound actualRound config.wavelength
          if actualWave != wave then
            throw s!"wave mismatch for {event.blockHash}: Rust={wave}, Lean={actualWave}"
      | none => pure ()
      pure { blocklace := blocklace', valid := valid', blocks := dag.blocks ++
        [⟨event.blockHash, block.id, event.creator, reportedRound, event.parentHashes⟩] }
    else throw s!"formal block id collision while inserting {event.blockHash}"
  else throw s!"missing predecessor while inserting {event.blockHash}"

structure ReplayState where
  config : ReplayConfig
  dag : ReplayDag
  approvals : List AcceptApprovalEvent
  checkedApprovalPairs : List (String × String)
  certificates : List BuildThresholdCertificateEvent
  buffered : List (String × List String)
  equivocationChecks : Nat
  finalityChecks : Nat
  finalityDecisions : List Bool
  tauChecks : Nat
  outputChecks : Nat
  lastTau : Option (Nat × List String)
  lastSchedulerTick : Option Nat
  emitted : List String

def ReplayState.empty (config : ReplayConfig) : ReplayState :=
  { config, dag := ReplayDag.empty, approvals := [], checkedApprovalPairs := [],
    certificates := [], buffered := [], equivocationChecks := 0,
    finalityChecks := 0, finalityDecisions := [], tauChecks := 0,
    outputChecks := 0, lastTau := none, lastSchedulerTick := none, emitted := [] }

private def requireKnownBlock (state : ReplayState) (hash : String) : Except String ReplayedBlock :=
  match state.dag.findHash? hash with
  | some block => pure block
  | none => throw s!"unknown block hash '{hash}'"

private def checkWeightHash (state : ReplayState) (hash : String) : Except String Unit :=
  if hash == state.config.weightTableHash then pure ()
  else throw s!"weight table hash mismatch: trace={hash}, config={state.config.weightTableHash}"

private def checkOptionalWeightHash (state : ReplayState) : Option String → Except String Unit
  | some hash => checkWeightHash state hash
  | none => pure ()

private def candidateRound (state : ReplayState) (parents : List String) : Except String Nat := do
  let blocks ← parents.mapM (requireKnownBlock state)
  match blocks with
  | [] => pure 0
  | first :: rest => pure (rest.foldl (fun depth block => Nat.max depth block.round)
      first.round + 1)

private def checkKnownLifecycle (state : ReplayState) (blockHash : String)
    (parents : List String) (round wave : Option Nat) : Except String Unit := do
  if parents.eraseDups.length != parents.length then
    throw s!"block {blockHash} repeats a predecessor"
  let actualRound ← candidateRound state parents
  match round with
  | none => throw s!"block {blockHash} has null round despite known predecessors"
  | some reported =>
      if reported != actualRound then
        throw s!"round mismatch for {blockHash}: Rust={reported}, Lean={actualRound}"
  match wave with
  | none => pure ()
  | some reported =>
      let actual := waveOfRound actualRound state.config.wavelength
      if reported != actual then
        throw s!"wave mismatch for {blockHash}: Rust={reported}, Lean={actual}"

private def uniqueStrings (values : List String) : Bool :=
  values.eraseDups.length == values.length

private def sameStringSet (left right : List String) : Bool :=
  left.all (fun value => right.contains value) &&
  right.all (fun value => left.contains value) && uniqueStrings left && uniqueStrings right

private def checkApprovalEvent (state : ReplayState) (event : AcceptApprovalEvent) : Except String Unit := do
  let approver ← requireKnownBlock state event.approverHash
  let target ← requireKnownBlock state event.targetHash
  if approver.creator != event.approver then
    throw s!"approval creator mismatch for {event.approverHash}"
  if approver.round != event.round then
    throw s!"approval round mismatch for {event.approverHash}"
  match event.wave with
  | some wave =>
      let actual := waveOfRound approver.round state.config.wavelength
      if wave != actual then
        throw s!"approval wave mismatch: Rust={wave}, Lean={actual}"
  | none => pure ()
  if !CMRef.checkApproves state.dag.blocklace state.dag.valid approver.formalId target.formalId then
    throw s!"Lean KR2 approval predicate rejects {event.approverHash} → {event.targetHash}"

private def checkCertificateEvent (state : ReplayState)
    (event : BuildThresholdCertificateEvent) : Except String Unit := do
  checkWeightHash state event.weightTableHash
  if !uniqueStrings event.approvers then throw "certificate repeats an approver"
  if !uniqueStrings event.approverHashes then throw "certificate repeats an evidence block"
  if event.approverCount != event.approvers.length then
    throw s!"certificate approver_count={event.approverCount}, unique approvers={event.approvers.length}"
  let approverIds ← event.approvers.mapM fun node => match state.config.nodeIndex? node with
    | some id => pure id
    | none => throw s!"certificate names unknown validator '{node}'"
  let referenceCertificate := CMRef.buildWCert state.config.bonds approverIds
  let support := referenceCertificate.weight
  let total := bondOf state.config.bonds state.config.validatorSet
  if support != event.approverWeight then
    throw s!"certificate weight mismatch: Rust={event.approverWeight}, Lean={support}"
  if total != event.totalWeight then
    throw s!"certificate total mismatch: Rust={event.totalWeight}, Lean={total}"
  if !CMRef.checkStrictTwoThirds state.config.bonds state.config.validatorSet
      referenceCertificate.accepted then
    throw s!"insufficient quorum: support={support}, total={total}, required 3*support > 2*total"
  let expectedId := computeCertificateId event.kind event.leaderHash event.ratifierHash
  if expectedId != event.certificateId then
    throw s!"certificate id mismatch: Rust={event.certificateId}, Lean={expectedId}"
  let target ← requireKnownBlock state event.leaderHash
  match event.wave with
  | some wave =>
      let actual := waveOfRound target.round state.config.wavelength
      if wave != actual then
        throw s!"certificate wave mismatch: Rust={wave}, Lean={actual}"
  | none => pure ()
  let evidence ← event.approverHashes.mapM (requireKnownBlock state)
  let evidenceCreators := evidence.map ReplayedBlock.creator |>.eraseDups
  if !sameStringSet evidenceCreators event.approvers then
    throw "certificate evidence creators do not match approvers"
  match event.kind with
  | "ratification" =>
      let ratifierHash ← match event.ratifierHash with
        | some hash => pure hash
        | none => throw "ratification certificate has null ratifier_hash"
      let ratifier ← requireKnownBlock state ratifierHash
      for approver in evidence do
        if !(state.approvals.any fun approval =>
            approval.approverHash == approver.rustHash &&
            approval.targetHash == event.leaderHash) then
          throw s!"ratification certificate uses unrecorded approval {approver.rustHash} → {event.leaderHash}"
        if !CMRef.checkObserves state.dag.blocklace state.dag.valid
            ratifier.formalId approver.formalId then
          throw s!"ratifier {ratifierHash} does not observe evidence block {approver.rustHash}"
        if !CMRef.checkApproves state.dag.blocklace state.dag.valid
            approver.formalId target.formalId then
          throw s!"certificate evidence {approver.rustHash} does not formally approve {event.leaderHash}"
      if !CMRef.checkRatifies state.config.bonds state.config.validatorSet
          state.dag.blocklace state.dag.valid ratifier.formalId target.formalId then
        throw s!"Lean KR4 ratification predicate rejects certificate {event.certificateId}"
  | "super_ratification" =>
      if event.ratifierHash.isSome then
        throw "super-ratification certificate must have null ratifier_hash"
      for ratifier in evidence do
        if !(state.certificates.any fun certificate =>
            certificate.kind == "ratification" &&
            certificate.leaderHash == event.leaderHash &&
            certificate.ratifierHash == some ratifier.rustHash) then
          throw s!"super-ratification certificate uses unrecorded ratifier {ratifier.rustHash}"
      let witness := evidence.map ReplayedBlock.formalId |>.toFinset
      if !CMRef.checkSuperRatifies state.config.bonds state.config.validatorSet
          state.dag.blocklace state.dag.valid witness target.formalId then
        throw s!"Lean KR4 super-ratification predicate rejects certificate {event.certificateId}"
  | kind => throw s!"unknown certificate kind '{kind}'"

private def checkEquivocationEvent (state : ReplayState)
    (event : DetectEquivocationEvent) : Except String Unit := do
  if event.conflictingBlockHashes.length < 2 || !uniqueStrings event.conflictingBlockHashes then
    throw "equivocation needs at least two distinct block hashes"
  let blocks ← event.conflictingBlockHashes.mapM (requireKnownBlock state)
  for block in blocks do
    if block.creator != event.equivocator then
      throw s!"block {block.rustHash} belongs to {block.creator}, not {event.equivocator}"
    if block.round != event.round then
      throw s!"block {block.rustHash} is round {block.round}, not {event.round}"
  let first ← match blocks with | block :: _ => pure block | [] => throw "empty equivocation"
  for other in blocks.drop 1 do
    if !CMRef.checkEquivocation state.dag.blocklace state.dag.valid
        first.formalId other.formalId then
        throw s!"Lean KR2 equivocation predicate rejects {first.rustHash} vs {other.rustHash}"

private def checkPackageEvent (nodeId peerId : String) (hashes : List String)
    (kind : String) : Except String Unit := do
  if nodeId.isEmpty then throw s!"{kind} has an empty node_id"
  if peerId.isEmpty then throw s!"{kind} has an empty peer_id"
  if hashes.isEmpty then throw s!"{kind} has no blocks"
  if !uniqueStrings hashes then throw s!"{kind} repeats a block hash"
  for hash in hashes do
    if hash.isEmpty then throw s!"{kind} contains an empty block hash"

private def computeTau (state : ReplayState) (wave : Nat) : Except String (ReplayedBlock × List String) := do
  let key := fun id => (state.dag.findFormal? id).map ReplayedBlock.rustHash |>.getD ""
  let domain := state.dag.blocks.map ReplayedBlock.formalId
  let (latestId, orderIds) ← CMRef.computeTau state.config.bonds state.config.validatorSet
    state.dag.blocklace state.dag.valid state.config.wavelength state.config.leader domain key wave
  let latest ← match state.dag.findFormal? latestId with
    | some block => pure block
    | none => throw "CMRef tau returned an unknown leader"
  let order ← orderIds.mapM fun id => match state.dag.findFormal? id with
    | some block => pure block.rustHash
    | none => throw "CMRef tau returned an unknown block"
  pure (latest, order)

private def eventNodeId : TraceEvent → String
  | .createBlock event => event.nodeId
  | .validateBlock event => event.nodeId
  | .insertBlock event => event.nodeId
  | .bufferBlock event => event.nodeId
  | .resolveMissingParent event => event.nodeId
  | .detectEquivocation event => event.nodeId
  | .acceptApproval event => event.nodeId
  | .buildThresholdCertificate event => event.nodeId
  | .computeFinality event => event.nodeId
  | .runTauOrder event => event.nodeId
  | .emitOutput event => event.nodeId
  | .sendPackage event => event.nodeId
  | .deliverPackage event => event.nodeId
  | .schedulerTick event => event.nodeId
  | .runWaveTask event => event.nodeId

private def checkEvent (state : ReplayState) (event : TraceEvent) : Except String ReplayState := do
  if (eventNodeId event).isEmpty then throw "event has an empty node_id"
  match event with
  | .createBlock event =>
      if !event.missingParentHashes.isEmpty then throw "created block has missing parents"
      checkOptionalWeightHash state event.weightTableHash
      checkKnownLifecycle state event.blockHash event.parentHashes event.round event.wave
      pure state
  | .validateBlock event =>
      checkOptionalWeightHash state event.weightTableHash
      if event.outcome != "valid" && event.outcome != "invalid" then
        throw s!"unknown validation outcome '{event.outcome}'"
      if (event.outcome == "valid") != event.errors.isEmpty then
        throw "validation outcome and errors disagree"
      if event.parentHashes.eraseDups.length != event.parentHashes.length then
        throw s!"validated block {event.blockHash} repeats a predecessor"
      let missing := event.parentHashes.filter fun hash => (state.dag.findHash? hash).isNone
      if missing.isEmpty then
        checkKnownLifecycle state event.blockHash event.parentHashes event.round event.wave
      else
        if event.outcome == "valid" then
          throw s!"valid block {event.blockHash} has unknown predecessors {missing}"
        if event.round.isSome || event.wave.isSome then
          throw s!"block {event.blockHash} reports round/wave despite unknown predecessors {missing}"
      pure state
  | .insertBlock event =>
      checkOptionalWeightHash state event.weightTableHash
      let dag ← insertBlock state.config state.dag event
      let buffered ← match state.buffered.find? (fun item => item.1 == event.blockHash) with
        | none => pure state.buffered
        | some (_, []) => pure (state.buffered.filter fun item => item.1 != event.blockHash)
        | some (_, missing) =>
            throw s!"buffered block {event.blockHash} inserted before resolving {missing}"
      pure { state with dag, buffered }
  | .bufferBlock event =>
      checkOptionalWeightHash state event.weightTableHash
      if event.round.isSome || event.wave.isSome then
        throw s!"buffered block {event.blockHash} reports a round/wave with missing predecessors"
      let actualMissing := event.parentHashes.filter fun hash =>
        (state.dag.findHash? hash).isNone
      if event.missingParentHashes.isEmpty ||
          !sameStringSet actualMissing event.missingParentHashes then
        throw s!"buffer missing-parent set is incomplete: Rust={event.missingParentHashes}, Lean={actualMissing}"
      pure { state with buffered := (event.blockHash, event.missingParentHashes) :: state.buffered }
  | .resolveMissingParent event =>
      let _ ← requireKnownBlock state event.resolvedParentHash
      let missing ← match state.buffered.find? (fun item => item.1 == event.blockHash) with
        | some item => pure item.2
        | none => throw s!"resolve event for unbuffered block {event.blockHash}"
      if !missing.contains event.resolvedParentHash then
        throw s!"{event.resolvedParentHash} was not missing for {event.blockHash}"
      let buffered := state.buffered.map fun item =>
        if item.1 == event.blockHash then (item.1, item.2.erase event.resolvedParentHash) else item
      pure { state with buffered }
  | .detectEquivocation event =>
      checkEquivocationEvent state event
      pure { state with equivocationChecks := state.equivocationChecks + 1 }
  | .acceptApproval event =>
      let pair := (event.approverHash, event.targetHash)
      let alreadyChecked := state.checkedApprovalPairs.contains pair
      if alreadyChecked then
        if !(state.approvals.contains event) then
          throw s!"repeated approval {event.approverHash} → {event.targetHash} has inconsistent metadata"
      else
        checkApprovalEvent state event
      let approvals := state.approvals ++ [event]
      let checkedApprovalPairs :=
        if alreadyChecked then state.checkedApprovalPairs else pair :: state.checkedApprovalPairs
      pure { state with approvals := approvals, checkedApprovalPairs := checkedApprovalPairs }
  | .buildThresholdCertificate event =>
      checkCertificateEvent state event
      pure { state with certificates := state.certificates ++ [event] }
  | .computeFinality event =>
      checkOptionalWeightHash state event.weightTableHash
      if event.wavelength != state.config.wavelength then
        throw s!"wavelength mismatch: Rust={event.wavelength}, config={state.config.wavelength}"
      let candidate ← requireKnownBlock state event.blockHash
      if event.nodeId != candidate.creator then
        throw s!"finality node mismatch: Rust={event.nodeId}, creator={candidate.creator}"
      if event.outputPrefixHash.isSome then
        throw "compute_finality output_prefix_hash must be null before tau/output execution"
      let leanFinal := CMRef.checkFinal state.config.bonds state.config.validatorSet
        state.dag.blocklace state.dag.valid event.wave event.wavelength
        state.config.leader candidate.formalId
      let rustFinal ← match event.decision with
        | "finalized" => pure true
        | "not_finalized" => pure false
        | value => throw s!"unknown finality decision '{value}'"
      if leanFinal != rustFinal then
        let ratifiers := CMRef.ratifyingCreators state.config.bonds state.config.validatorSet
          state.dag.blocklace state.dag.valid
          (CMRef.waveWitness state.dag.blocklace state.dag.valid event.wave event.wavelength)
          candidate.formalId
        let support := bondOf state.config.bonds ratifiers
        let total := bondOf state.config.bonds state.config.validatorSet
        throw s!"finality mismatch\n  Node: {event.nodeId}\n  Wave: {event.wave}\n  Block: {event.blockHash}\n  Rust: {event.decision}\n  Lean CMRef: {if leanFinal then "finalized" else "not_finalized"}\n  Reason: certificate weight={support}, total={total}, requires 3*support > 2*total"
      if rustFinal then
        let certificateId ← match event.certificateId with
          | some id => pure id
          | none => throw "finalized decision has null certificate_id"
        if !(state.certificates.any fun certificate =>
            certificate.certificateId == certificateId &&
            certificate.kind == "super_ratification" &&
            certificate.leaderHash == event.blockHash) then
          throw s!"finality references missing super-ratification certificate {certificateId}"
      else if event.certificateId.isSome then
        throw "not_finalized decision has a non-null certificate_id"
      pure { state with
        finalityChecks := state.finalityChecks + 1
        finalityDecisions := state.finalityDecisions ++ [rustFinal] }
  | .runTauOrder event =>
      if event.wavelength != state.config.wavelength then throw "tau wavelength mismatch"
      let (latest, leanOrder) ← computeTau state event.wave
      if latest.rustHash != event.latestLeaderHash then
        throw s!"tau leader mismatch: Rust={event.latestLeaderHash}, Lean={latest.rustHash}"
      if event.outputLen != event.orderedBlockHashes.length then
        throw "tau output_len disagrees with ordered_block_hashes"
      if leanOrder != event.orderedBlockHashes then
        throw s!"tau order mismatch: Rust={event.orderedBlockHashes}, Lean={leanOrder}"
      let tauResult : Option (Nat × List String) := some (event.wave, leanOrder)
      let nextTauChecks := state.tauChecks + 1
      let state := { state with tauChecks := nextTauChecks }
      let state := { state with lastTau := tauResult }
      pure { state with emitted := [] }
  | .emitOutput event =>
      let (tauWave, order) ← match state.lastTau with
        | some result => pure result
        | none => throw "emit_output appeared before run_tau_order"
      if event.wave != tauWave then
        throw s!"output wave mismatch: Rust={event.wave}, tau wave={tauWave}"
      let expected ← match order[event.outputIndex]? with
        | some hash => pure hash
        | none => throw s!"output index {event.outputIndex} is out of range"
      if expected != event.blockHash then
        throw s!"output mismatch at index {event.outputIndex}: Rust={event.blockHash}, Lean={expected}"
      if event.outputIndex != state.emitted.length then
        throw s!"non-contiguous output index {event.outputIndex}; expected {state.emitted.length}"
      let outputPrefix := state.emitted ++ [event.blockHash]
      let expectedPrefixHash := computeOutputPrefixHash outputPrefix
      if expectedPrefixHash != event.outputPrefixHash then
        throw s!"output prefix hash mismatch at {event.outputIndex}: Rust={event.outputPrefixHash}, Lean={expectedPrefixHash}"
      pure { state with emitted := outputPrefix, outputChecks := state.outputChecks + 1 }
  | .sendPackage event =>
      checkPackageEvent event.nodeId event.peerId event.blockHashes "send_package"
      pure state
  | .deliverPackage event =>
      checkPackageEvent event.nodeId event.peerId event.blockHashes "deliver_package"
      pure state
  | .schedulerTick event =>
      if event.nodeId.isEmpty then throw "scheduler_tick has an empty node_id"
      match state.lastSchedulerTick with
      | some previous =>
          if event.tick < previous then
            throw s!"scheduler tick moved backwards: previous={previous}, current={event.tick}"
      | none => pure ()
      pure { state with lastSchedulerTick := some event.tick }
  | .runWaveTask event =>
      if event.nodeId.isEmpty then throw "run_wave_task has an empty node_id"
      if state.config.leader event.wave |>.isNone then
        throw s!"run_wave_task names unknown wave {event.wave}"
      if !(["propose", "vote", "finalize"].contains event.task) then
        throw s!"unknown wave task '{event.task}'"
      pure state

def replayEvents (config : ReplayConfig) (events : List TraceEvent) : Except String ReplayState :=
  let rec go (state : ReplayState) (remaining : List TraceEvent) (index : Nat) :=
    match remaining with
    | [] =>
        if !state.buffered.isEmpty then
          .error s!"end of trace with unresolved buffered blocks {state.buffered.map Prod.fst}"
        else match state.lastTau with
          | some (_, order) =>
              if state.emitted == order then .ok state
              else .error s!"end of trace after emitting {state.emitted.length}/{order.length} tau blocks"
          | none => .ok state
    | event :: rest =>
        match checkEvent state event with
        | .ok state' => go state' rest (index + 1)
        | .error reason => .error s!"event {index} ({event.kind}): {reason}"
  go (ReplayState.empty config) events 1

private def scenarioRequirements (label : String) (state : ReplayState) : Except String Unit := do
  if state.dag.blocks.isEmpty then throw "scenario inserted no blocks"
  match label with
  | "normal" =>
      if state.approvals.isEmpty then throw "normal scenario emitted no approvals"
      if state.certificates.isEmpty then throw "normal scenario emitted no certificates"
      if !state.finalityDecisions.contains true then throw "normal scenario finalized no leader"
      if state.tauChecks == 0 then throw "normal scenario ran no tau ordering"
      if state.emitted.isEmpty then throw "normal scenario emitted no ordered output"
  | "equivocation" =>
      if state.equivocationChecks == 0 then throw "equivocation scenario detected no equivocation"
      if !state.finalityDecisions.contains true then
        throw "equivocation scenario did not retain safe finality"
  | "low_stake" =>
      if state.approvals.isEmpty then throw "low-stake scenario emitted no approvals"
      if state.finalityDecisions.isEmpty || state.finalityDecisions.contains true then
        throw "low-stake scenario did not demonstrate weighted rejection"
  | _ => pure ()

def replayFile (tracePath configPath label : String) : IO Bool := do
  let configResult ← readReplayConfig configPath
  let traceResult ← readTraceFile tracePath
  match configResult, traceResult with
  | .error reason, _ =>
      IO.println s!"[{label}] MISMATCH ✗\n  config: {reason}"
      pure false
  | _, .error reason =>
      IO.println s!"[{label}] MISMATCH ✗\n  parse: {reason}"
      pure false
  | .ok config, .ok events =>
      match replayEvents config events with
      | .ok state =>
          match scenarioRequirements label state with
          | .ok _ =>
              IO.println s!"[{label}] CONFORMANT ✓\n  events: {events.length}\n  finality checks: {state.finalityChecks}\n  equivocation checks: {state.equivocationChecks}\n  tau checks: {state.tauChecks}\n  output checks: {state.outputChecks}"
              pure true
          | .error reason =>
              IO.println s!"[{label}] MISMATCH ✗\n  incomplete scenario: {reason}"
              pure false
      | .error reason =>
          IO.println s!"[{label}] MISMATCH ✗\n  {reason}"
          pure false

def main : IO Unit := do
  let base := "../lean/traces"
  let scenarios := ["normal", "equivocation", "low_stake"]
  let results ← scenarios.mapM fun label =>
    replayFile s!"{base}/{label}.json" s!"{base}/{label}.weights.json" label
  if results.all id then IO.println "\n✓ All fixtures CONFORMANT"
  else
    IO.println "\n✗ One or more fixtures FAILED"
    IO.Process.exit 1

end CordialMiners
