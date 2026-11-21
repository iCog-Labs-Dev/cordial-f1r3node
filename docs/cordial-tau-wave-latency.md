# Four-node Cordial τ wave timing

This baseline measures live Cordial finalized ordering on four native
`f1r3node-rust` validators. Four host-side `ordered_output_server` processes
mirror their blocks and run `weighted_tau` with the four equal genesis bonds.
The measurement uses Cordial rounds and waves; one wave contains three rounds.

The probe timestamps the first mirror to observe a new wave and the first
poll where all four mirrors have finalized that wave with the same τ anchor
and ordered hash sequence. The difference is *wave observation to four-mirror
τ agreement*. It is separate from deploy submission latency, native block
inclusion time, and PoR checkpoint timing. The one-second mirror polling and
host-side τ computation are included in this baseline.

## Run

From the repository root, with the cluster certificates and local image
prepared:

```sh
docker compose --env-file docker/.env -f docker/four-node-cluster.yml \
  -p tau-wave-baseline up -d \
  cordial-boot cordial-validator-1 cordial-validator-2 \
  cordial-validator-3 cordial-validator-4
just demo-cordial-four-node-cluster-wait
cargo build -j 1 -p cordial-f1r3node-adapter --bin ordered_output_server
python3 -u docker/scripts/measure-four-node-tau-waves.py \
  --evidence-dir /tmp/cordial-tau-wave-baseline-run \
  --waves 3 --timeout-seconds 600
```

Use a new evidence directory for each run. Start measuring while genesis is
still among the latest 256 native blocks, or increase `--depth`. The probe
checks that each node exposes genesis before it starts its mirrors. It skips
the initial replayed wave, requires exact anchor and ordered-hash agreement
at each measured wave, and saves the four raw mirror logs plus provenance and
results. Progress lines show each observed and finalized wave as it runs.

After the run, stop this project's containers while retaining its volumes:

```sh
docker compose --env-file docker/.env -f docker/four-node-cluster.yml \
  -p tau-wave-baseline down
```

## Recorded run

The [2026-11-21 evidence](evidence/cordial-tau-wave-baseline-2026-11-21/README.md)
contains a three-wave run. All four mirrors agreed at each wave. The median
wave observation-to-agreement time was **37.254 seconds**, with a range of
**34.878–44.313 seconds**. This is a small, live equal-weight baseline; the
separate [deploy run](evidence/por-deploy-latency-2026-11-21/README.md)
measured submission-to-final-position latency with active PoR weights.

