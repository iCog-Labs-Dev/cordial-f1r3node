# PoR observatory

The [dashboard](por-weighted-slashing.html) is a local, buildless simulation
explorer. It uses no external fonts, chart libraries, analytics, or uploads.
The interface keeps three evidence sources separate:

- **Guided example:** the checked-in two-round fixture. It shows exact recorded
  scores, isolated slashing, retained capital, exclusion, and recovery. It is
  historical evidence, not a result of the newly added tests.
- **Imported scenarios:** JSON exported by the multi-round Rust runner. Honest,
  unequal-start, isolated-fault, and correlated-fault files can be imported
  together. Empty scenarios never receive invented trajectories.
- **Live observers:** optional shadow status snapshots. Freshness, agreement,
  catch-up, and conflicts are reported independently of imported results.

## Open locally

From the repository root:

    python3 -m http.server 8765 --bind 127.0.0.1 --directory docs

Open [the observatory](http://127.0.0.1:8765/demos/por-weighted-slashing.html).
Serving the docs root also makes the linked run instructions available.
The dashboard does not launch tests or a cluster.

If the server runs on a remote host, forward the loopback port from your laptop:

    ssh -N -o ExitOnForwardFailure=yes -L 8876:127.0.0.1:8765 USER@REMOTE_NODE

Then open http://127.0.0.1:8876/demos/por-weighted-slashing.html.

## Explore a run

1. Choose **Import results** and select one or more JSON files, or drag files into
   the import dialog. Files stay in memory and are cleared on refresh.
2. Choose a loaded scenario. Each card shows its number of recorded rounds.
3. Switch between **Reputation**, **Voting share**, and **Stability**.
4. Scrub the timeline, click the chart, or play recorded rounds at 1x, 2x, or 5x.
   **Fault** jumps to the first recorded exclusion. The inspector explains the
   selected checkpoint; the ledger shows exact values.
5. Inspect the checkpoint hash and, in the saved example, the signed proof
   identities and penalty commitment.

Arrow keys step through rounds and Space toggles playback while the chart is in
view and an input is not focused. Playback pauses when the tab becomes hidden.
The layout supports narrow screens; the exact-value table scrolls horizontally
when necessary.

Stability charts divide score and voting-share deltas by the file's acceptance
bounds. The vertical axis is logarithmic; 1x is the bound and 0 is no change.
Ranking must also remain stable for the recorded acceptance window.
A line connects recorded samples; it does not create intermediate observations.

## Export results (run manually)

    POR_CONVERGENCE_OUTPUT="$PWD/docs/demos/traces" cargo test -j 1 -p cordial-f1r3node-adapter --test test_por_checkpoint_penalties multi_round_ranking_and_slashing_convergence -- --exact --ignored

Import honest.json, unequal.json, isolated.json, and correlated.json.
The versioned format has kind "por_convergence_trace", scenario name,
observer count, thresholds, completion status, and a rounds array.
Each round includes active weights, checkpoint identity, measured deltas,
ranking stability, observer agreement, events, and explicitly labeled
reputation entries. Restart markers identify the first record after recovery.

The runner writes a scenario file only after its assertions finish successfully.
A missing file is not a failure report or a passing result. The UI labels
completion status as **reported by the file**; importing JSON is not an audit
or a new test execution.

Earlier array-only exports remain readable. If those entries lack validator
labels, retained balances and exclusions are left unknown rather than guessed
from public-key order. Files are limited to 5 MB and 10,000 rounds.

To regenerate the historical example after changing its fixture:

    bash docker/scripts/build-por-demo-fixture.sh

See [convergence validation](../por-convergence-validation.md) for the complete
adapter, ordering, penalty, and restart commands.

## Optional live feed

Start the isolated cluster and keyed shadow observers using the
[shadow runner guide](../por-shadow-runner.md). The smoke verifier stops its
observer processes when it exits; keep or restart those observers if you want
a continuously updating dashboard.

    python3 docker/scripts/export-por-demo-live.py --data-root PATH --min-por-round 4 --watch-seconds 2

Use the actual observer data root. This exports
docs/demos/por-live-shadow.json; the dashboard polls it every 2.5 seconds.
The exporter does not start or stop validators.

Different active rounds are labeled **Catching up**. Conflicting finalized
prefixes or different projections at the same round are labeled **Reports
conflict**. Old snapshots remain distinguishable from live evidence. The
progress line only reports an advance after the common finalized prefix
actually grows during the browser session.

Neither imported reputation traces nor shadow agreement establish live
f1r3node validator-side weight enforcement or host-chain slash execution.

## Animated walkthrough

Choose **Animated walkthrough** next to **Results explorer**. Both tabs share
scenario selection and the selected recorded round. Import the same scenario
JSON files as usual. Play the six-stage walkthrough, step backward or forward,
select a stage directly, scrub rounds, or jump to the first fault or final round.
Playback pauses on tab changes, scenario changes, import, and when the page is hidden.

The ledger, voting shares, checkpoint identity, and convergence chart use
recorded snapshots. The DAG and processing order are explicitly schematic:
exports do not contain block edges, rating deliveries, or intermediate observer
states. Observer badges repeat the exported agreement result rather than
pretending to replay independent snapshots. The current convergence fixture
uses a single-producer chain; the animation does not invent a multi-producer DAG.
Motion timing is illustrative, and reduced-motion preferences disable transitions.

## Actual event replay and live viewing

The **Animation & live replay** tab now includes a separate event cinema.
It reads append-only JSONL emitted by the convergence test when explicitly enabled.
Existing result JSON can be opened for **outcomes only**; it cannot reconstruct
DAGs, observer exchanges, or actual timing. The previous schematic walkthrough
remains available in a collapsed section.

Stop the plain HTTP server if it occupies port 8765. Start the read-only replay
server from the repository root in WSL:

    python3 docker/scripts/serve-por-replay.py --events docs/demos/traces --port 8765

Open http://localhost:8765/demos/por-weighted-slashing.html#animation.
In another WSL terminal, explicitly run the instrumented test:

    POR_EVENT_OUTPUT="$PWD/docs/demos/traces" POR_CONVERGENCE_OUTPUT="$PWD/docs/demos/traces" cargo test --release -p cordial-f1r3node-adapter --test test_por_checkpoint_penalties multi_round_ranking_and_slashing_convergence -- --exact --ignored --nocapture

Choose a live scenario, then **Connect live**. The test runs honest, unequal,
isolated, and correlated scenarios sequentially; a later scenario may be waiting
for its file. Each scenario produces NAME.events.jsonl in the event directory.
A new run replaces that scenario's event file; save prior traces before rerunning.
The server uses a run identity and byte cursor to detect replacement and avoid
duplicate delivery. A partial final line is withheld until the writer finishes it.

**Follow live** moves to the newest captured event. Turn it off or use the timeline
to examine earlier activity; **Play** advances through captured events at the
selected rate. Events received while paused remain available for later playback.
Use **Next checkpoint** to skip fixture construction, **Jump to slash** for the
first recorded slashing outcome, and **Save event trace** to retain received data.
Opening a saved JSONL file needs only the ordinary static server; live connection
requires the Python replay server. The Python server does not start tests or Docker.

Captured events include verified equivocation proof identities, signed fixture
block insertions with parent links, checkpoint windows, local rating staging,
closure checks, peer-batch publication (including duplicate counts), checkpoint
closure, per-observer durable commits, exact penalty verification, common round
outcomes, state recovery, and scenario completion. The diagnostic stream is
opt-in; when disabled, event payloads are not constructed.

Evidence boundaries:

- The DAG is the shared single-producer fixture. Per-observer local DAG receipt
  is not part of this test and is not invented by the viewer.
- Fixture blocks and equivocation proofs are constructed before checkpoint
  processing. Trace order preserves this actual sequence.
- Peer events record publication into an observer's file exchange. The animation
  illustrates movement; it is not a trace of production-network packets.
- Observers execute sequentially in this harness. A checkpoint commit event does
  not mean the other observers have committed; the common ledger updates only
  after their states have been compared.
- Execution timestamps are captured elapsed time; playback spacing is adjustable.
  Snapshot-only playback uses illustrative timing and labels it explicitly.
- Missing completion markers mean incomplete capture, not a pass. Browser import
  and file-reported completion are not independent verification of the protocol.
- Tracing and its stream format have been formatted and statically reviewed;
  the new Rust instrumentation still needs compilation and an explicit test run.

Trace output is stored in `docs/demos/traces` relative to the repository root.
The commands use `$PWD` to give Cargo an absolute output path, because integration
tests may run with a crate directory as their working directory. The replay
server defaults to this repository output directory even when launched elsewhere.
