# One-node PoR shadow runner

`live_por_shadow` connects to one real f1r3node gRPC endpoint and mirrors its
blocks into a local Cordial `LiveIngress`. It loads the node's authorized
membership from the bonds file and opens `PorRuntime` before ingesting live
traffic. The genesis reputation vector contains exactly those validators, each
with `PorConfig::default().initial_reputation`. `PorRuntime` persists and
activates the genesis projection; on restart it restores the exact activated
weights before computing finality.

The runner observes and reports PoR weights. It does not change the running
f1r3node validator, produce ratings, or advance reputation rounds. Finalized
wave rating lifecycle and peer transport are later slices.

## Live smoke procedure

Start the four-node cluster using `docker/four-node-cluster.yml`, then run
from the repository root:

```sh
cargo run -j 1 -p cordial-f1r3node-adapter --bin live_por_shadow -- \
  --grpc-url http://127.0.0.1:51401 \
  --bonds-file docker/genesis/cordial-bonds.txt \
  --data-dir /tmp/cordial-por-validator-1
```

For a local `just run-standalone` node in the sibling checkout, use its own
one-validator bonds file and gRPC port instead:

```sh
cargo run -j 1 -p cordial-f1r3node-adapter --bin live_por_shadow -- \
  --grpc-url http://127.0.0.1:40401 \
  --bonds-file ../f1r3node/run-local/data/standalone/genesis/bonds.txt \
  --data-dir /tmp/cordial-por-standalone
```

Use a separate persistent data directory for each observed node, outside the
repository. The runner polls every two seconds by default. It emits a JSON
status line after each successful poll and atomically writes the same document
to `<data-dir>/por/shadow-status.json`. The document contains
`active_por_round`, `weight_commitment`, the exact hex-keyed `weights` map,
`finalized_anchor`, and `finalized_hashes`. The full finalized hash sequence
is retained so startup can reject a replay that changes or loses the previously
published prefix.

For a restart check, save the status file, stop the runner, and start it again
with the same arguments. Compare the old and new `active_por_round`,
`weight_commitment`, and `weights`. The new `finalized_hashes` must begin
with the old sequence. The runner checks these conditions before publishing its
first recovered status. It replays blocks from height zero because the mirror
itself is process-local. A node that cannot supply its complete history cannot
satisfy this restart check.

The bonds file is required and remains the authority for membership. Changing
the file's validator set while reusing a data directory causes PoR startup to
fail closed. The runner never adds senders observed in blocks to the membership
map. Keep the data directory and its activation record across restarts.
