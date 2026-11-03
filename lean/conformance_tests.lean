import LeanVerification.Replay

open CordialMiners

private def replaceFirst (events : List TraceEvent)
    (replacement : TraceEvent → Option TraceEvent) : Option (List TraceEvent) :=
  match events with
  | [] => none
  | event :: rest =>
      match replacement event with
      | some changed => some (changed :: rest)
      | none => (event :: ·) <$> replaceFirst rest replacement

private def mustReplace (events : List TraceEvent)
    (replacement : TraceEvent → Option TraceEvent) : List TraceEvent :=
  (replaceFirst events replacement).getD events

private def containsText (value fragment : String) : Bool :=
  (value.splitOn fragment).length > 1

private def isError : Except ε α → Bool
  | .error _ => true
  | .ok _ => false

private def expectFailure (name expected : String) (config : ReplayConfig)
    (events : List TraceEvent) : IO Bool := do
  match replayEvents config events with
  | .ok _ =>
      IO.println s!"[negative/{name}] FAIL: mutated trace was accepted"
      pure false
  | .error reason =>
      if containsText reason expected then
        IO.println s!"[negative/{name}] rejected ✓"
        pure true
      else
        IO.println s!"[negative/{name}] FAIL: unexpected diagnostic: {reason}"
        pure false

private def loadScenario (base label : String) : IO (Except String (ReplayConfig × List TraceEvent)) := do
  let config ← readReplayConfig s!"{base}/{label}.weights.json"
  let events ← readTraceFile s!"{base}/{label}.json"
  pure <| Prod.mk <$> config <*> events

private def parserTests (base : String) : IO Bool := do
  let schema ← readTraceFile s!"{base}/schema_all_events.json"
  let coverageOk := match schema with
    | .ok [
        .createBlock _, .validateBlock _, .insertBlock _, .bufferBlock buffer,
        .resolveMissingParent _, .detectEquivocation _, .acceptApproval _,
        .buildThresholdCertificate _, .computeFinality _, .runTauOrder _,
        .emitOutput _, .sendPackage _, .deliverPackage _, .schedulerTick _,
        .runWaveTask _] => buffer.wave.isNone && buffer.round.isNone
    | _ => false
  let malformed := isError (parseLine "{")
  let unknown := isError (parseLine "{\"event\":\"future_event\"}")
  let missing := isError (parseLine
    "{\"event\":\"scheduler_tick\",\"node_id\":\"n\",\"tick\":1}")
  let wrongType := isError (parseLine
    "{\"event\":\"scheduler_tick\",\"node_id\":\"n\",\"tick\":\"one\",\"wave\":null}")
  let extra := isError (parseLine
    "{\"event\":\"scheduler_tick\",\"node_id\":\"n\",\"tick\":1,\"wave\":null,\"extra\":0}")
  let escaped := match parseLine
      "{\"event\":\"scheduler_tick\",\"node_id\":\"quoted\\\"node\",\"tick\":1,\"wave\":null}" with
    | .ok (.schedulerTick event) => event.nodeId == "quoted\"node"
    | _ => false
  let nullWeight := match parseLine
      "{\"event\":\"compute_finality\",\"node_id\":\"n\",\"wave\":0,\"wavelength\":3,\"block_hash\":\"b\",\"decision\":\"not_finalized\",\"certificate_id\":null,\"output_prefix_hash\":null,\"weight_table_hash\":null}" with
    | .ok (.computeFinality event) => event.weightTableHash.isNone
    | _ => false
  let blank := isError (parseLine "")
  let ok := coverageOk && malformed && unknown && missing && wrongType && extra && escaped && nullWeight && blank
  IO.println s!"[parser/all-15-and-errors] {if ok then "PASS ✓" else "FAIL"}"
  if !ok then
    IO.println s!"  coverage={coverageOk}, malformed={malformed}, unknown={unknown}, missing={missing}, wrongType={wrongType}, extra={extra}, escaped={escaped}, nullWeight={nullWeight}, blank={blank}, schema={reprStr schema}"
  pure ok

private def negativeTests (base : String) : IO Bool := do
  let normal ← loadScenario base "normal"
  let equivocation ← loadScenario base "equivocation"
  let lowStake ← loadScenario base "low_stake"
  match normal, equivocation, lowStake with
  | .ok (normalConfig, normalEvents), .ok (equivConfig, equivEvents),
      .ok (lowConfig, lowEvents) =>
    let invalidFinality := mustReplace normalEvents fun
      | .computeFinality event => some (.computeFinality { event with decision := "not_finalized" })
      | _ => none
    let insufficientQuorum := mustReplace normalEvents fun
      | .buildThresholdCertificate event => some (.buildThresholdCertificate
          { event with approvers := ["01"], approverCount := 1, approverWeight := 100 })
      | _ => none
    let invalidCertificate := mustReplace normalEvents fun
      | .buildThresholdCertificate event => some (.buildThresholdCertificate
          { event with certificateId := "broken-certificate" })
      | _ => none
    let duplicateApprover := mustReplace normalEvents fun
      | .buildThresholdCertificate event => some (.buildThresholdCertificate
          { event with approvers := event.approvers ++ event.approvers })
      | _ => none
    let duplicateEvidence := mustReplace normalEvents fun
      | .buildThresholdCertificate event => some (.buildThresholdCertificate
          { event with approverHashes := event.approverHashes ++ event.approverHashes })
      | _ => none
    let falseEquivocation := mustReplace equivEvents fun
      | .detectEquivocation event => some (.detectEquivocation
          { event with equivocator := "01" })
      | _ => none
    let missingPredecessor := mustReplace normalEvents fun
      | .insertBlock event => some (.insertBlock
          { event with round := some 1, parentHashes := ["missing-parent"] })
      | _ => none
    let incorrectTau := mustReplace normalEvents fun
      | .runTauOrder event => some (.runTauOrder
          { event with orderedBlockHashes := ["wrong-block"] })
      | _ => none
    let incorrectOutput := mustReplace normalEvents fun
      | .emitOutput event => some (.emitOutput
          { event with outputPrefixHash := "wrong-prefix" })
      | _ => none
    let wrongWeightHash := mustReplace normalEvents fun
      | .computeFinality event => some (.computeFinality
          { event with weightTableHash := "wrong-weights" })
      | _ => none
    -- A separate semantic negative case: a false finalized claim over the
    -- low-stake execution. The actual compiled Rust threshold mutation is
    -- exercised by scripts/issue188_mutation_test.sh.
    let lowStakeFalseFinality := mustReplace lowEvents fun
      | .computeFinality event => some (.computeFinality
          { event with decision := "finalized", certificateId := some "mutated" })
      | _ => none
    let checks ← [
      expectFailure "invalid-finality" "finality mismatch" normalConfig invalidFinality,
      expectFailure "insufficient-quorum" "insufficient quorum" normalConfig insufficientQuorum,
      expectFailure "invalid-certificate" "certificate id mismatch" normalConfig invalidCertificate,
      expectFailure "duplicate-approver" "certificate repeats an approver" normalConfig duplicateApprover,
      expectFailure "duplicate-evidence" "certificate repeats an evidence block" normalConfig duplicateEvidence,
      expectFailure "false-equivocation" "belongs to" equivConfig falseEquivocation,
      expectFailure "missing-predecessor" "unknown block hash" normalConfig missingPredecessor,
      expectFailure "incorrect-tau" "tau order mismatch" normalConfig incorrectTau,
      expectFailure "incorrect-output-prefix" "output prefix hash mismatch" normalConfig incorrectOutput,
      expectFailure "wrong-weight-table" "weight table hash mismatch" normalConfig wrongWeightHash,
      expectFailure "low-stake-false-finality" "finality mismatch" lowConfig lowStakeFalseFinality
    ].mapM id
    pure (checks.all id)
  | .error reason, _, _ | _, .error reason, _ | _, _, .error reason =>
      IO.println s!"[negative/setup] FAIL: {reason}"
      pure false

def main : IO Unit := do
  let base := "traces"
  let parserOk ← parserTests base
  let negativeOk ← negativeTests base
  if parserOk && negativeOk then
    IO.println "\n✓ Parser and negative conformance tests passed"
  else
    IO.println "\n✗ Parser or negative conformance test failed"
    IO.Process.exit 1
