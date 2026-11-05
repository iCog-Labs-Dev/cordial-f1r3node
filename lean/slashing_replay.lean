import LeanVerification.SlashingReplay

open Lean CordialMiners.SlashingReplay

def main (args : List String) : IO Unit := do
  let path ← match args with
    | [] => pure "slashing/fixtures.v1.json"
    | [path] => pure path
    | _ => throw (IO.userError "usage: slashing_replay [fixtures.json]")
  let input ← IO.FS.readFile path
  match Json.parse input >>= replaySuite with
  | .error error =>
    IO.eprintln s!"slashing conformance FAILED: {error}"
    IO.Process.exit 1
  | .ok names =>
    for name in names do IO.println s!"[slashing/{name}] PASS"
    IO.println s!"{names.length} slashing fixtures conform"
