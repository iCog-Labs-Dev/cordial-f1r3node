# Four-Node Local-Intercept Demo

This demo exists for the quarter's convergence KR:

```text
Run a 4-node local cluster and verify that all nodes produce the exact same
total order for a set of intercepted transactions.
```

The implementation is intentionally a logic prototype. It does not replace the
production f1r3node consensus network. Instead, four local f1r3node runtimes are
started with `--consensus cordial-miners`; each runtime enters the Cordial
adapter path through the normal node API surface, proposes the same local input
stream, and exposes its Cordial ordered view through the existing block query
endpoint.

## Files

- `docker/four-node-intercept.yml` starts the four runtimes and the verifier.
- `docker/conf/cordial-four-node.conf` selects `cordial-miners` and sets the
  demo network id to `cordial-demo-four-node`.
- `docker/scripts/verify-four-node-order.sh` performs the convergence check.
- `demo.md` contains the human-facing runbook.

## What The Verifier Checks

The verifier runs in the same Docker network as the four nodes:

1. Wait for every node's HTTP API.
2. Assert every runtime reports `networkId = cordial-demo-four-node`.
3. Assert every runtime is a bonded validator.
4. Call the admin proposal endpoint on each runtime.
5. Read `/api/blocks/10` from each runtime.
6. Compare the ordered block projections exactly.

The compared projection is:

```json
{
  "blockNumber": 0,
  "blockHash": "...",
  "sender": "...",
  "seqNum": 0,
  "deployCount": 0,
  "isFinalized": true
}
```

For this Docker KR demo, the four containers use four distinct bonded demo
validator keys. That makes the run closer to a real multi-validator local
cluster while still keeping the convergence check simple: after local proposal,
the ordered block view exposed by each node should match. The broader
four-validator protocol behavior is also covered by
`crates/cordial-f1r3node-adapter/tests/conformance.rs`, which tests
honest-majority finality, equivocation rejection, slash evidence, and tau
prefix invariance.

## Commands

Build the local integration image once:

```bash
just demo-cordial-up
just demo-cordial-down
```

Run the four-node KR demo:

```bash
just demo-cordial-four-node-config
just demo-cordial-four-node-up
just demo-cordial-four-node-verify
just demo-cordial-four-node-blocks
just demo-cordial-four-node-down
```

Expected verifier result:

```text
PASS: four local f1r3node runtimes produced the same Cordial Miners ordered view.
```

## Interpreting `peers` And `nodes`

The four containers are local runtimes, not a production peer-discovery shard.
It is acceptable for `peers` and `nodes` to remain zero in this demo. The KR is
verified by the ordered-view comparison, while the networking replacement path
is outside the scope of this logic prototype.
