# Cordial Miners Conformance Testing

## Purpose

The conformance harness is the adapter-level sanity check before running
Cordial Miners on a shared f1r3node testnet. Unit tests prove individual
predicates, but conformance scenarios prove that the adapter, blocklace,
finality, tau ordering, equivocation evidence, and f1r3node-facing snapshot
surface agree on the same result.

Implementation lives in:

```text
crates/cordial-f1r3node-adapter/tests/conformance.rs
```

## Current Harness Shape

Each `ConformanceScenario` contains:

- `name`: a human-readable scenario identifier.
- `bonds`: the validator stake map used by weighted f1r3node integration.
- `network_blocks`: mocked blocks fed to the adapter in network arrival order.
- `expected_finalized_leader`: the final leader expected by the Cordial Miners
  paper logic, or `None` if the attack prevents finality.
- `expected_tau_prefix`: the exact ordered block identities expected from tau.

`run_scenario` creates a real `CordialCasperAdapter`, feeds each mocked network
block into the adapter-owned blocklace, then checks the f1r3node-facing
`CasperSnapshot`:

```text
snapshot.dag.last_finalized_block_hash == expected_finalized_leader
snapshot.ordered_finalized_blocks == expected_tau_prefix
```

It also verifies the `last_finalized_block` adapter method when a final leader
is expected.

## Covered Scenarios

### Honest Majority

Four equal-stake validators build a three-round wave. The elected wave-0 leader
is observed, ratified, and super-ratified by an honest supermajority. The
adapter must report that leader as finalized and expose the exact tau prefix
through `ordered_finalized_blocks`.

### Equivocation Attack

The wave-0 leader creates two same-round genesis blocks. Honest witnesses see
both branches. Because a block that observes conflicting same-creator branches
does not approve either branch, neither equivocation branch can be
super-ratified. The scenario also records the raw conflicting blocks in the
core `EvidencePool` and formats them through the adapter `SlashDeployFormatter`
to prove the slashing path has retained usable evidence.

### Tau Prefix Invariance

After the honest majority scenario emits a tau prefix, a second wave is appended
to the DAG. The newly computed ordered output may grow, but the old prefix must
remain byte-for-byte unchanged. This verifies the monotonicity property used by
clients that consume finalized output incrementally.

## Fixture Authoring Guidance

The current scenarios are Rust builders because they need direct access to
typed `BlockIdentity` values. QA teams can add JSON or YAML fixtures later with
the same logical shape:

```yaml
name: honest-majority-wave0
bonds:
  - validator: "01"
    stake: 1
  - validator: "02"
    stake: 1
blocks:
  - id: "leader"
    creator: "01"
    hash: "0100..."
    round: 0
    parents: []
  - id: "v2-r1"
    creator: "02"
    hash: "1200..."
    round: 1
    parents: ["leader"]
expected:
  finalized_leader: "leader"
  tau_prefix: ["leader", "v2-r1"]
```

A fixture loader should convert the symbolic block IDs into real
`BlockIdentity` values, create `CordialBlockPayload` values with f1r3node-style
state hashes and bonds, then call the same `run_scenario` function. Keep the
fixture format deterministic:

- parent lists should be explicit,
- block hashes should be stable 32-byte values,
- validator IDs should be canonical hex strings,
- expected tau output should be written in the exact expected order,
- equivocation fixtures should include all conflicting branches, not only the
  branch selected for slash formatting.

## Relationship to f1r3node

The harness checks the adapter outputs that f1r3node consumes:

- `dag.last_finalized_block_hash`,
- `ordered_finalized_blocks`,
- `last_finalized_block`,
- slash system deploy bytes produced by `F1r3SlashDeployFormatter`.

This keeps the test fast and deterministic while still exercising the boundary
that the f1r3node node runtime, proposer, block processor, and HTTP/gRPC query
surfaces depend on.
