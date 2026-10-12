# One-node PoR shadow runner

`live_por_shadow` connects to one real f1r3node gRPC endpoint and mirrors its
blocks into a local Cordial `LiveIngress`. It loads the node's authorized
membership from the bonds file and opens `PorRuntime` before ingesting live
traffic. The genesis reputation vector contains exactly those validators, each
with `PorConfig::default().initial_reputation`. `PorRuntime` persists and
activates the genesis projection; on restart it restores the exact activated
weights before computing finality.

The runner observes and reports PoR weights and the latest finalized Cordial
wave. It does not change the running f1r3node validator, produce ratings, or
advance reputation rounds. Finalized-wave reporting does not imply that the
intermediate waves were processed as PoR rating rounds. Signed rating exchange
and round closure are later slices.

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
`finalized_anchor`, `observed_finalized_wave`, and `finalized_hashes`.
`observed_finalized_wave` is null until Cordial exports a final leader; it is
the wave of that leader, not the active PoR reputation round. The full
finalized hash sequence is retained so startup can reject a replay that
changes or loses the previously published prefix. A recovered observed wave
must not regress, and a leader change within the same wave is rejected.

For a restart check, save the status file, stop the runner, and start it again
with the same arguments. Compare the old and new `active_por_round`,
`weight_commitment`, `weights`, and `observed_finalized_wave`. The new
`finalized_hashes` must begin
with the old sequence. The runner checks these conditions before publishing its
first recovered status. It replays blocks from height zero because the mirror
itself is process-local. A node that cannot supply its complete history cannot
satisfy this restart check.

The bonds file is required and remains the authority for membership. Changing
the file's validator set while reusing a data directory causes PoR startup to
fail closed. The runner never adds senders observed in blocks to the membership
map. Keep the data directory and its activation record across restarts.

## Four-node recovery smoke

Start and verify a recently started connected cluster, then build the runner
once and run the four-observer harness from the repository root:

```sh
just demo-cordial-four-node-cluster-up
just demo-cordial-four-node-cluster-verify
cargo build --release -j 1 -p cordial-f1r3node-adapter --bin live_por_shadow
python3 docker/scripts/verify-four-node-por-shadow.py \
  --data-root "$HOME/cordial-por-four-node" \
  --binary target/release/live_por_shadow
```

The harness starts one observer for each validator gRPC port (51401, 52401,
53401, and 54401), using a separate persistent directory under `--data-root`
for each. It waits for at least two finalized hashes by default, then compares
the active PoR round, weight commitment, full weight map, and common finalized
hash prefix. It restarts the first observer with the same data directory and
checks that its old finalized prefix and projection survived. It then runs
`docker compose restart cordial-validator-1`, waits for a new observer status,
and checks recovery and four-node agreement again. The harness stops only its
own observer processes when it exits; node volumes and PoR data remain.

Use a data root outside the repository and reuse it for subsequent recovery
runs. Logs are written to `<data-root>/<node>/runner.log`; each status is at
`<data-root>/<node>/por/shadow-status.json`. `--skip-node-restart` runs only the
observer restart check. `--timeout-seconds` and `--min-finalized-blocks` adjust
the convergence gate. The cluster must be running before this command; the
harness does not start or reset it.

### Isolated fresh-cluster reproduction

The Compose file fixes container names and host ports, so only one copy can
run at a time. To test a fresh chain while preserving an existing cluster,
stop the existing containers without removing their named volumes. Use a new
Compose project name and a new PoR data root for the smoke, then restore the
original project. Run these commands from the repository root in one shell:

```sh
./docker/scripts/generate-four-node-cluster-certs.sh
cargo build --release -j 1 -p cordial-f1r3node-adapter --bin live_por_shadow
just demo-cordial-four-node-cluster-down

smoke_project="por-smoke-$(date +%s)"
smoke_root="$(mktemp -d /tmp/por-shadow-four-node.XXXXXX)"
COMPOSE_PROJECT_NAME="$smoke_project" docker compose --env-file docker/.env \
  -f docker/four-node-cluster.yml up -d
COMPOSE_PROJECT_NAME="$smoke_project" python3 \
  docker/scripts/verify-four-node-por-shadow.py \
  --data-root "$smoke_root" \
  --binary target/release/live_por_shadow \
  --timeout-seconds 300

COMPOSE_PROJECT_NAME="$smoke_project" docker compose --env-file docker/.env \
  -f docker/four-node-cluster.yml down
just demo-cordial-four-node-cluster-up
```

Run the final two commands even if the harness fails or is interrupted. Plain
`down` preserves both projects' node volumes; the smoke's PoR status and logs
remain under `$smoke_root`. The harness waits for post-genesis finality, checks
all four observers' round, weight commitment, weights, and finalized prefix,
then restarts one observer and its validator. Set `COMPOSE_PROJECT_NAME` on the
harness command because its validator restart uses Docker Compose too.

### Long-history resource limit

The observer restores durable PoR weights immediately, but its Cordial
blocklace is process-local. Each start replays blocks from height zero and
computes finalized ordering over the complete mirrored history before its
first status publication. The `--timeout-seconds` option only extends the
wait; it does not reduce replay memory or work. A live test at roughly 3,200
heights mirrored around 12,000 blocks per observer, then consumed about 19 GiB
of resident memory per optimized observer during ordering without publishing
a status. That run was stopped, so long-history recovery is not yet verified.
Runner logs now report each completed height batch to distinguish replay from
the subsequent ordering calculation. Use the isolated fresh-cluster procedure
for the recovery smoke until the long-history path has bounded memory and a
durable mirror checkpoint.

All observers currently retain the genesis reputation round because the live
shadow runner does not yet produce ratings or activate later rounds. Agreement
here tests deterministic observation and durable recovery, not enforcement of
PoR weights by f1r3node.
