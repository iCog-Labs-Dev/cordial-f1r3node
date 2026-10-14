# One-node PoR shadow runner

`live_por_shadow` connects to one real f1r3node gRPC endpoint and mirrors its
blocks into a local Cordial `LiveIngress`. It loads the node's authorized
membership from the bonds file and opens `PorRuntime` before ingesting live
traffic. The genesis reputation vector contains exactly those validators, each
with `PorConfig::default().initial_reputation`. `PorRuntime` persists and
activates the genesis projection; on restart it restores the exact activated
weights before computing finality.

The runner observes and reports PoR weights and the latest finalized Cordial
wave. It does not change the running f1r3node validator. Finalized-wave
reporting does not imply that intermediate waves were processed as PoR rating
rounds. With keyed observers and a shared exchange, it derives complete round
candidates. An explicit flag can commit and activate them in the local Cordial
observer.

For a validator observer, `--validator-rating-key-file /path/to/validator.key`
optionally loads its own secp256k1 private key at startup. The file contains
exactly 64 hexadecimal characters, with optional surrounding whitespace.
The runner derives the public key and requires it to match exactly one active
entry in the bonds file. Status then reports its public identity as
`configured_rating_rater`; the private key is never written to status or logs.
Keep one key file per observer and restrict access to its owner. This option
only validates the key for the existing rating-signing path. Reusing a data
directory with a different key is rejected.

To stage a local signed rating batch, add `--stage-local-ratings` alongside
`--validator-rating-key-file`. The runner uses PoR's existing round coordinator
and writes its signed envelopes under `<data-dir>/por/rating-outbox` before
publishing a new shadow status. It stages only the next round expected by the
durable PoR state. If the latest finalized leader is in a later wave, the
runner replays the expected wave's final leader and ordered prefix under the
currently active weights, then checks that prefix against the latest published
output.
Missing leaders or a prefix mismatch stop the runner. Start this opt-in check
with a fresh chain or a source whose full block history is available. Repeated
polls and restarts accept the same batch and reject conflicting ratings for
that wave. Without an exchange directory, nothing is sent to peers.
`active_por_round` remains unchanged without an exchange and the explicit
commit flag.

For host-side exchange between keyed observers, also pass the same
`--rating-exchange-dir` to each runner. Every observer needs its own data
directory and its own validator key file. The key file contains the 64-character
private key configured for that validator in the cluster's Compose environment;
the TLS `node.key.pem` is a different key. For example, run validator 1 with:

```sh
cargo run -j 1 -p cordial-f1r3node-adapter --bin live_por_shadow -- \
  --grpc-url http://127.0.0.1:51401 \
  --bonds-file docker/genesis/cordial-bonds.txt \
  --validator-rating-key-file "$HOME/por-keys/validator-1.key" \
  --stage-local-ratings \
  --rating-exchange-dir "$HOME/cordial-por-rating-exchange" \
  --data-dir "$HOME/cordial-por-four-node/validator-1"
```

Run validators 2, 3, and 4 with gRPC ports `52401`, `53401`, and `54401`,
their matching key files, separate data directories, and the **same** exchange
directory. Reuse that directory for observer restarts and choose a new one for
a fresh chain. The demo private keys and bonded public keys are in
`docker/.env.example` and `docker/genesis/cordial-bonds.txt`; use the actual
Compose-configured keys if they differ. Keep key files owner-readable only.
The runner republishes its immutable outbox envelopes after restart, verifies
peer signatures and finalized interaction evidence, and logs envelope counts,
complete raters, and quorum progress. When **every** eligible rater is complete,
it closes an in-memory candidate, replays the reputation transition, and logs
`candidate_por_round`, `candidate_checkpoint`, and
`candidate_weight_commitment`. Compare all three fields across four observer
logs. The weighted quorum alone does not trigger a candidate because observers
could otherwise close with different subsets. Without the flag below, no state
or active weights are committed.

To allow a complete candidate to advance durable PoR state, add
`--commit-complete-ratings` to **each** keyed observer command. The runner
commits only while the candidate's source wave is the latest finalized wave.
It first checks that the projected weights preserve the published final leader
and ordered prefix, then commits the audited reputation state and activates
the weights in its local Cordial ingress. The JSON status then reports the new
`active_por_round`, `weight_commitment`, and exact `weights` map. It defers
commit when a later wave has already finalized; such a candidate stays
visible in logs. Use a fresh cluster and start all keyed observers promptly to
exercise this gate. A restart restores the last active projection before
replaying blocks; if a commit was interrupted before activation, the runner
replays under the prior projection and retries activation before publishing
status. The durable reputation round is the cursor for the next wave.

This flag changes only the observer's Cordial view. The running upstream
f1r3node still uses its own consensus weights. A live four-observer activation
and restart smoke remains to be run. An exchange conflict or invalid peer
rating stops the runner for inspection.

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

This harness exercises the default observer mode. It does not pass validator
keys or start the shared rating exchange described above.

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

### Parallel keyed PoR smoke

The keyed harness can run against a fresh Compose project without stopping an
existing cluster. First build the current runner and prepare four owner-readable
key files named `validator-1.key` through `validator-4.key` under
`$HOME/por-keys`. Their contents must match the validator private keys in the
Compose configuration; the matching public keys must be in
`docker/genesis/cordial-bonds.txt`. Prepare the cluster certificates with
`./docker/scripts/generate-four-node-cluster-certs.sh` if needed.

In terminal A, start the observers **before** the fresh validators:

```sh
cargo build --release -j 1 -p cordial-f1r3node-adapter --bin live_por_shadow
smoke_root="$(mktemp -d /tmp/por-keyed-smoke.XXXXXX)"
python3 docker/scripts/verify-four-node-por-shadow.py \
  --data-root "$smoke_root/observers" \
  --binary target/release/live_por_shadow \
  --rating-keys-dir "$HOME/por-keys" \
  --rating-exchange-dir "$smoke_root/exchange" \
  --commit-complete-ratings --min-por-round 1 \
  --grpc-ports 61001 62001 63001 64001 \
  --compose-override docker/four-node-por-smoke-override.yml \
  --compose-project por-keyed-smoke --timeout-seconds 900
```

In terminal B, start the separate cluster:

```sh
COMPOSE_PROJECT_NAME=por-keyed-smoke docker compose --env-file docker/.env \
  -f docker/four-node-cluster.yml \
  -f docker/four-node-por-smoke-override.yml up -d
```

After terminal A finishes or fails, stop only the smoke project:

```sh
COMPOSE_PROJECT_NAME=por-keyed-smoke docker compose --env-file docker/.env \
  -f docker/four-node-cluster.yml \
  -f docker/four-node-por-smoke-override.yml down
```

The override uses separate container names, host ports, network, and named
volumes. Plain `down` preserves its volumes for diagnosis. The harness requires
all four observers to reach at least PoR round 1 with the same active
checkpoint, weight commitment, weight map, and finalized prefix; it then
checks observer and validator restarts. Logs and durable PoR data remain under
`$smoke_root`.

**Observed blocker on 2026-10-14:** four keyed observers launched before a
fresh cluster, then all exited with `no final leader can be replayed for wave
0` after a later wave finalized. The node's height-0 genesis block has an
empty sender, so it cannot be the validator elected as Cordial's wave-0
leader. The current PoR protocol requires wave `k` to produce rating round
`k+1` with consecutive reputation rounds; it has no representation for a
wave without a final leader. No observer committed round 1 in this run. A
deterministic skipped-wave rule must be designed and audited before this smoke
can pass. Do not treat a timeout or a genesis-only status as PoR convergence.

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

The default harness does not supply validator rating keys, so its observers
retain the genesis reputation round. A keyed exchange can derive later
candidates and, with `--commit-complete-ratings`, activate them locally.
Agreement in the default harness tests deterministic observation and durable
recovery, not enforcement of PoR weights by f1r3node.
