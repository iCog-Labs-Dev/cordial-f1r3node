# Live PoR shadow runner and four-node smoke tests

This guide covers a single observer and the four-observer checkpoint smoke.

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
f1r3node still uses its own consensus weights. The future-only checkpoint
smoke below has now been run with four observers; the legacy wave path remains
subject to the blocker noted below. An exchange conflict or invalid peer
rating stops the runner for inspection.

## Checkpoint rating PoC

For a fresh cluster and fresh observer data directories, add
`--checkpoint-ratings` to each keyed observer. This selects v2 ratings bound
to consecutive 16-block checkpoints in Cordial's finalized prefix. It
requires `--validator-rating-key-file`, `--stage-local-ratings`, and a shared
`--rating-exchange-dir`. The v2 outbox and exchange use separate namespaces
from the older wave format. Do not reuse a data directory with a committed
wave-format PoR round.

Checkpoint ratings retain their v2 signing and exchange format. The committed
checkpoint reputation block, its wire envelope, and the durable snapshot use
v3, which includes the finalized-prefix source and penalty commitment. Data
directories containing checkpoint v2 blocks from before the `dev` rebase
require an explicit migration; start the current four-observer smoke with fresh
observer directories. There is no automatic conversion of those histories.

Each observer stages its own signed checkpoint batch and closes the round only
when every authorized rater that was not ejected in the preceding PoR state
has supplied its expected ratings. Ejected keys stay in Cordial membership
with zero weight; their observers can receive and audit rounds without signing
ratings or blocking closure. With
`--commit-complete-ratings`, the runner rechecks the source checkpoint against
its current finalized prefix, audits the checkpoint transition, and commits only if
the projected weights preserve already published Cordial finality. It then
activates those weights in its local ingress and updates `shadow-status.json`.
If the projection would rewrite the published leader or ordered prefix, the
runner logs a deferred candidate and keeps the current active round. On
restart, it waits for the complete committed source checkpoint before
activating a pending round. An incomplete replay retries on later polls without
publishing status; a conflicting replayed prefix stops the runner. Direct
activation through `DurablePorState` applies the same checkpoint identity check.

In this smoke, one keyed **shadow observer** runs beside each validator. It
signs a rating for each *other* active validator that produced a block in the
checkpoint's exact 16-block finalized window. It selects one canonical block
as evidence per recipient, gives the current fixed block-production score,
and does not rate itself or a validator with no qualifying block. The four
observers exchange and verify these signed ratings through the shared rating
directory. Round closure requires every authorized observer's complete
evidence-derived batch. The f1r3node processes produce the blocks; PoR rating
exchange and weight activation run in the observers, not inside f1r3node.

For a four-observer convergence and restart check, use the isolated Compose
project in [Parallel keyed PoR smoke](#parallel-keyed-por-smoke). Its command
passes `--checkpoint-ratings`, starts the observers before the validators,
and waits for at least one committed checkpoint round.

The script compares the active round, checkpoint hash, exact weights,
commitment, and finalized prefix across all four observers, then restarts an
observer and a validator. This is an observer convergence check; it does not
show that the running f1r3node nodes consume PoR weights.

To exercise the future-only cutover, add the same
`--checkpoint-epoch-delay-waves N` to the keyed harness command below. The
runner derives the effective wave from the rating source wave plus `N`, waits
for the finalized wave immediately before it, then seals the old-weight
prefix and commits the checkpoint. A late proposal is rejected; choose `N`
large enough for all four rating exchanges to finish before that boundary.
The proposal and reputation state recover together after observer restart.
This PoC stores one epoch, so observers keep mirroring blocks after round 1
without committing round 2. The four-observer check still measures observer
agreement and local Cordial ordering; it does not prove node-side enforcement.

The current single-table activation guard can defer a valid checkpoint when
its new weights would change an already published Cordial leader. To verify
checkpoint calculation separately, replace `--min-por-round 1` in the command
below with `--min-por-round 0 --min-candidate-round 1`. All four observers must
report the same complete source checkpoint, reputation checkpoint, projected
weights, and weight commitment. The harness checks that candidate again after
the observer restart. Add `--skip-node-restart` when testing against an existing
cluster whose validator processes should stay running. This candidate gate
does not claim that round 1 weights became active.

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
  --bonds-file ../f1r3node-rust/run-local/data/standalone/genesis/bonds.txt \
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

## Checkpoint equivocation authorization

The checkpoint path authenticates finalized equivocation carriers before
replaying, building, auditing, or applying a penalty-bearing reputation block.
Arbitrary proof bytes and unapproved verified proofs cannot authorize penalties.
`SlashSystemDeploy` remains a separate host-chain path.

A producer calls `queue_por_checkpoint_equivocation_authorization` with a
verified core proof, shard, and target PoR round. The v2 authorization message
(`cordial-por:equivocation-authorization:v2`) is covered by the containing
Cordial block's signature. It encodes the shard, target round, offender, and
canonical proof bytes with big-endian u64 length prefixes; the target round
is a big-endian u64. The existing carrier reserves `source_finalized_wave = 0`
for v2 intents; the message domain distinguishes them from wave-zero v1 messages.

For round `r`, the carrier must occur in finalized output positions
`[(r - 1) * 16, r * 16)`. A carrier outside that window does not carry forward.
The intent cannot include its own resulting prefix hash; admission binds the
verified penalty to the finalized checkpoint's round, prefix length, and hash.
Wave authorizations cannot be reused for checkpoint transitions or vice versa.

The runtime decodes each in-window intent as an unverified claim, then verifies
both conflicting blocks, signatures, derived evidence round, complete ancestry,
and incomparability. The offender must be a member and must not be excluded in
the preceding PoR state; an active member with zero weight is still subject to
permanent ejection. The publisher must have positive Cordial weight, its block
signature must verify, and neither conflicting block may be newer than its
authorization block.

Malformed proofs and semantically ineligible claims, including unbonded offenders,
are accounted for without creating penalty events. Previously ejected offenders
are also no-ops, preserving their retained balances. Missing proof ancestry is
an error that prevents completion until the node recovers that history; it is
never treated as evidence of invalidity. Proof verification does not apply the
DAG insertion rule that rejects another signature variant of a stored block.

Closure retains the original complete in-window carrier set unchanged. Admission
tracks eligible and ignored claims separately and requires exact accounting of
both sets. An explicit proof list must cover every eligible authorization and
cannot supply its own ignored-claim decisions. Duplicate publications are
deduplicated. If several eligible proofs name the same offender, the
lexicographically first canonical message selects one penalty for that round.
The penalty retains its core evidence round for correlated-slashing calculations.
Runtime commit reauthenticates against its own published finalized view and checks
the preceding state used for classification. Durable application checks that
state and the complete accounting before changing reputation. Reauthorization is
idempotent. Ejection, retained balances, and zero weights survive restart in the
existing v3 checkpoint state format.

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

The keyed harness uses a fresh Compose project and preserves an existing
project's volumes. The override fixes container names and host ports, so stop
any older cluster using those names or ports before starting this one. First
build the current runner and prepare four owner-readable
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
  --checkpoint-ratings --commit-complete-ratings \
  --checkpoint-epoch-delay-waves 12 \
  --min-finalized-blocks 16 --min-por-round 1 \
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

The first harness run may pass at the sealed boundary, before a new-weight
wave finalizes. Keep the same cluster running and rerun the harness with the
same observer data and exchange. Set `--min-finalized-blocks` above the first
run's finalized hash count to require a later finalized suffix. For the
2026-10-22 run, the sealed prefix had 139 hashes, so the second gate was 150:

```sh
python3 docker/scripts/verify-four-node-por-shadow.py \
  --data-root "$smoke_root/observers" \
  --binary target/release/live_por_shadow \
  --rating-keys-dir "$HOME/por-keys" \
  --rating-exchange-dir "$smoke_root/exchange" \
  --checkpoint-ratings --commit-complete-ratings \
  --checkpoint-epoch-delay-waves 12 \
  --min-finalized-blocks 150 --min-por-round 1 \
  --grpc-ports 61001 62001 63001 64001 \
  --compose-override docker/four-node-por-smoke-override.yml \
  --compose-project por-keyed-smoke --skip-node-restart --timeout-seconds 900
```

For another fresh run, replace `150` with a number greater than the first
run's finalized hash count. Check each `shadow-status.json` for equal hashes,
weights, and commitment, and confirm `observed_finalized_wave` reached or
passed the `effective_wave` stored in each observer's
`por/future-weight-epoch.bin`. The second harness run also restarts one
observer and verifies its longer finalized prefix.

After both runs finish or fail, stop only the smoke project:

```sh
COMPOSE_PROJECT_NAME=por-keyed-smoke docker compose --env-file docker/.env \
  -f docker/four-node-cluster.yml \
  -f docker/four-node-por-smoke-override.yml down
```

The override uses separate container names, host ports, network, and named
volumes. Plain `down` preserves its volumes for diagnosis. The command above
requires all four observers to reach active PoR round 1 with the same
checkpoint, weight commitment, weight map, and finalized prefix; it then
checks observer and validator restarts. The candidate gate described above
passes when an equal complete candidate remains pending. Logs and durable PoR
data remain under `$smoke_root`.

**Observed on 2026-10-22:** four observers derived the same round-1 checkpoint
and weight commitment. They sealed 139 finalized hashes at wave 12, with
effective wave 13. The first harness passed both observer and validator
restarts. The second run recovered the same state and agreed on 158 finalized
hashes at wave 14, including 19 hashes after the sealed prefix. Every
validator's local PoR weight changed from 200,000,000 to 402,738,615. Because
all four new weights were equal, this run does not show an unequal-weight
ordering change or f1r3node enforcing those weights.

**Legacy wave-path blocker observed on 2026-10-14:** four keyed observers
launched before a fresh cluster, then all exited because no final leader could
be replayed for wave 0 after a later wave finalized. The node's height-0
genesis block has an empty sender. The legacy format mapped wave `k` to PoR
round `k+1`, so no observer committed round 1 in that run. The checkpoint
mode above selects its source from the finalized block prefix and passed the
four-observer run recorded above. Do not treat a timeout or a genesis-only
status as PoR convergence.

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
