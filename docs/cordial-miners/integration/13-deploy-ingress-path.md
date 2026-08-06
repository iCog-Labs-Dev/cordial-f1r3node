# Deploy Ingress Path — End-to-End Lifecycle Trace

This document describes the **complete end-to-end deploy ingress lifecycle** for
Cordial Miners: how a deploy submitted via an external gRPC/HTTP proxy sidecar is
tracked from first observation through to finalized ordered output, without
modifying f1r3node source code.

---

## Architecture Overview

```
External Submitter
       │
       ├─ gRPC: doDeploy ──────────► LiveDeployProxy (port 40411)
       │                                    │  intercept & forward
       │                                    ▼
       │                            f1r3node gRPC (port 40401)
       │                                    │
       └─ HTTP: POST /api/deploy ──► LiveHttpDeployProxy (port 40413)
                                           │  intercept & forward
                                           ▼
                                   f1r3node HTTP (port 40403)

Both proxy sidecars call:
       LiveDeployIngress::observe_deploy()
                    │
                    ▼
             DeployTracer::record_observed()   ← Lifecycle state: Observed

On f1r3node DeployId response:
       LiveDeployIngress::observe_and_admit()
                    │
                    ▼
             DeployTracer::record_accepted()   ← Lifecycle state: Accepted

On block ingestion (LiveIngress::ingest_block_message / ingest_trusted_block):
       DeployTracer::correlate_block()         ← Lifecycle state: BlockIncluded

On finalized ordered output (LiveIngress::latest_finalized_ordered_output):
       DeployTracer::correlate_finalized_output() ← Lifecycle state: FinalizedOrdered
```

---

## Lifecycle State Machine

```
Observed  ──►  Accepted  ──►  BlockIncluded  ──►  FinalizedOrdered
   │               │                │                     │
 Seen by        f1r3node        Deploy sig          Block B appears
 ingress        returned        found in a          in ordered tau
 proxy          DeployId        mirrored block      output
```

States are **monotonically advancing** — a deploy never regresses. The
`DeployTraceState` enum encodes this ordering:

| State              | Meaning                                                              |
|--------------------|----------------------------------------------------------------------|
| `Observed`         | Signature seen by ingress proxy (gRPC or HTTP)                       |
| `Accepted`         | f1r3node returned a valid `DeployId` response                        |
| `BlockIncluded`    | Signature found in a mirrored `BlockMessage` body                    |
| `FinalizedOrdered` | Containing block present in `OrderedFinalizedOutput` under an anchor |

---

## Module: `deploy_trace.rs`

**Location:** `crates/cordial-f1r3node-adapter/src/deploy_trace.rs`

### Key types

```rust
pub enum DeployTraceState { Observed, Accepted, BlockIncluded, FinalizedOrdered }

pub struct DeployTraceReport {
    pub signature_hex:        String,
    pub state:                DeployTraceState,
    pub observed_at_secs:     u64,
    pub ingress_source:       TraceIngressSource,   // Grpc | Http | Unknown
    pub accepted_at_secs:     Option<u64>,
    pub block_hash_hex:       Option<String>,
    pub block_height:         Option<i64>,
    pub included_at_secs:     Option<u64>,
    pub finalized_anchor_hex: Option<String>,
    pub finalized_at_secs:    Option<u64>,
}

#[derive(Clone)]
pub struct DeployTracer { /* Arc<Mutex<HashMap<sig_hex, TraceEntry>>> */ }
```

### API

```rust
// Transition methods
tracer.record_observed(&sig, TraceIngressSource::Grpc);
tracer.record_accepted(&sig);
tracer.record_block_included(&sig, &block_hash, height);
tracer.record_finalized(&sig, &anchor_hash);

// Bulk correlation (advance all matching traces from a block / output)
tracer.correlate_block(deploy_sigs.iter().map(|v| v.as_slice()), &block_hash, height);
tracer.correlate_finalized_output(&finalized_block_hash_hexes, &anchor_hash);

// Query
let report: Option<DeployTraceReport> = tracer.get_deploy_trace(&sig);
let all:    Vec<DeployTraceReport>    = tracer.list_active_traces();
let pending: Vec<DeployTraceReport>   = tracer.list_pending_traces();
```

---

## Wiring into Existing Components

### `LiveDeployIngress` — Observed & Accepted

Attach a tracer via the builder method. All subsequent `observe_*` and
`observe_and_admit*` calls automatically advance the lifecycle:

```rust
use cordial_f1r3node_adapter::deploy_trace::{DeployTracer, TraceIngressSource};
use cordial_f1r3node_adapter::live_deploy_ingress::LiveDeployIngress;

let tracer = DeployTracer::new();
let mut ingress = LiveDeployIngress::new()
    .with_tracer(tracer.clone());

// Records Observed:
ingress.observe_grpc_deploy(&deploy);

// Records Observed + Accepted (if f1r3node returns DeployId):
ingress.observe_and_admit(DeployIngressSource::Grpc, deploy, &adapter)?;
```

### `LiveIngress` — BlockIncluded & FinalizedOrdered

Attach the **same shared tracer** to the live ingress mirror:

```rust
use cordial_f1r3node_adapter::live_ingress::LiveIngress;

let live = LiveIngress::new(adapter)
    .with_deploy_tracer(tracer.clone());  // same DeployTracer instance

// On each block ingestion (automatically calls correlate_block):
live.ingest_block_message(&block_msg)?;
// or:
live.ingest_trusted_window_block(block)?;

// On each ordering poll (automatically calls correlate_finalized_output):
let output = live.latest_finalized_ordered_output(wave_length)?;
```

---

## Sequence Diagram

```
Submitter     LiveDeployProxy     LiveDeployIngress    DeployTracer    f1r3node     LiveIngress
    │               │                    │                 │              │              │
    │── doDeploy ──►│                    │                 │              │              │
    │               │── observe ────────►│                 │              │              │
    │               │                   │── record_obs ──►│              │              │
    │               │                   │                 │              │              │
    │               │── forward ────────────────────────────────────────►│              │
    │               │                   │                 │    DeployId  │              │
    │               │◄─────────────────────────────────────────── resp ──│              │
    │               │── record_acc ─────────────────────► │              │              │
    │◄─── DeployId ─│                   │                 │              │              │
    │               │                   │                 │              │              │
    │               │                   │         (block published)      │              │
    │               │                   │                 │              │── block ────►│
    │               │                   │                 │              │              │
    │               │                   │          correlate_block ──────────────────► │
    │               │                   │                 │◄─────────────────────────  │
    │               │                   │ [BlockIncluded] │              │              │
    │               │                   │                 │              │              │
    │               │                   │    (consensus finalization)    │              │
    │               │                   │   correlate_finalized_output ──────────────►│
    │               │                   │                 │◄────────────────────────── │
    │               │                   │  [FinalizedOrdered]            │              │
```

---

## Demo Binary: `live_deploy_trace_demo`

**Location:** `crates/cordial-f1r3node-adapter/src/bin/live_deploy_trace_demo.rs`

### Harness mode (no real node required)

Synthetically advances 4 deploys through all four lifecycle states and prints
a timing & status report:

```bash
cargo run -p cordial-f1r3node-adapter --bin live_deploy_trace_demo -- --harness
```

Expected output:

```
╔══════════════════════════════════════════════════════════════╗
║          Cordial Deploy Lifecycle Trace Demo                 ║
╚══════════════════════════════════════════════════════════════╝

Lifecycle stages:
  Observed  ──►  Accepted  ──►  BlockIncluded  ──►  FinalizedOrdered

Mode: offline harness (synthetic lifecycle simulation)
Deploys: 4

Phase 1 — Observing 4 deploys via gRPC ingress proxy…
  [1] Observed  sig=0x01010101…
  [2] Observed  sig=0x02020202…
  [3] Observed  sig=0x03030303…
  [4] Observed  sig=0x04040404…
  State summary: Observed=4

Phase 2 — f1r3node accepted all deploys (DeployId returned)…
  State summary: Accepted=4

Phase 3 — Block produced containing the deploys…
  Block 0x00010203… @height=42 — advanced 4 trace(s) to BlockIncluded
  State summary: BlockIncluded=4

Phase 4 — Block appears in FinalizedOrdered output…
  Anchor 0x80818283… — advanced 4 trace(s) to FinalizedOrdered
  State summary: FinalizedOrdered=4

═══ Final Deploy Trace Report (0.00s) ═══
  [FinalizedOrdered]  sig=0x01010101…  block=0x00010203… @height=42  anchor=0x80818283…  +0s
  [FinalizedOrdered]  sig=0x02020202…  block=0x00010203… @height=42  anchor=0x80818283…  +0s
  [FinalizedOrdered]  sig=0x03030303…  block=0x00010203… @height=42  anchor=0x80818283…  +0s
  [FinalizedOrdered]  sig=0x04040404…  block=0x00010203… @height=42  anchor=0x80818283…  +0s

  Total traced:     4
  FinalizedOrdered: 4
  Pending:          0
  Wall clock:       0.00s

✓ All deploys reached FinalizedOrdered successfully.
```

### Live mode (requires a running f1r3node node)

```bash
# Terminal 1 — start the gRPC proxy sidecar
cargo run -p cordial-f1r3node-adapter --bin live_deploy_proxy \
  -- --upstream-grpc-url http://127.0.0.1:40401

# Terminal 2 — run the trace demo against the proxy
cargo run -p cordial-f1r3node-adapter --bin live_deploy_trace_demo \
  -- --grpc-url http://127.0.0.1:40411 \
     --node-grpc-url http://127.0.0.1:40401 \
     --timeout 120
```

CLI reference:

| Flag                | Default                         | Description                                    |
|---------------------|---------------------------------|------------------------------------------------|
| `--harness`         | `false`                         | Run offline simulation instead of live mode    |
| `--harness-deploys` | `4`                             | Number of synthetic deploys in harness mode    |
| `--grpc-url`        | `http://127.0.0.1:40411`        | gRPC proxy sidecar URL                         |
| `--node-grpc-url`   | `http://127.0.0.1:40401`        | f1r3node gRPC URL for block mirroring          |
| `--timeout`         | `120`                           | Max seconds to wait for `FinalizedOrdered`     |
| `--term`            | `@0!("cordial trace demo")`     | Rholang term for the submitted deploy          |
| `--shard-id`        | `root`                          | Shard identifier                               |

### Live Mirror Check Tracer CLI Flags (`live_mirror_check`)

The `live_mirror_check` binary includes built-in deploy tracing and gRPC `FindDeploy` resolution:

```bash
cargo run -p cordial-f1r3node-adapter --bin live_mirror_check -- \
  --grpc-url http://127.0.0.1:40401 \
  --skip-http-compare \
  --show-deploy-trace \
  --trace-deploy-sig <deploy_sig_hex>
```

| Flag | Default | Description |
|---|---|---|
| `--show-deploy-trace` | `false` | Print detailed per-deploy trace reports in the summary |
| `--trace-deploy-sig` | `<none>` | Pin-watch specific hex-encoded deploy signature(s) (can be passed multiple times) |

---

## Integration Tests: `test_deploy_trace.rs`

**Location:** `crates/cordial-f1r3node-adapter/tests/test_deploy_trace.rs`

Run all deploy trace integration tests:

```bash
cargo test -p cordial-f1r3node-adapter --test test_deploy_trace
```

### Test coverage map

| Test name                                                  | Lifecycle transition         |
|------------------------------------------------------------|------------------------------|
| `t1_record_observed_advances_to_observed_state`            | Observed                     |
| `t1_live_deploy_ingress_with_tracer_records_observed_on_observe_deploy` | Observed (via LiveDeployIngress) |
| `t1_http_ingress_records_http_source`                      | Observed (HTTP source)       |
| `t2_record_accepted_advances_to_accepted_state`            | Accepted                     |
| `t2_observe_and_admit_with_tracer_advances_to_accepted`    | Accepted (via adapter)       |
| `t2_rejected_deploy_stays_at_observed_not_accepted`        | Accepted (rejection case)    |
| `t3_record_block_included_advances_state_and_records_hash_and_height` | BlockIncluded          |
| `t3_correlate_block_advances_only_matching_traces`         | BlockIncluded (bulk)         |
| `t3_ingest_block_message_with_tracer_advances_included_deploys` | BlockIncluded (via mirror) |
| `t4_record_finalized_advances_to_finalized_ordered`        | FinalizedOrdered             |
| `t4_correlate_finalized_output_advances_block_included_traces` | FinalizedOrdered (bulk)  |
| `full_lifecycle_all_four_transitions`                      | All 4 transitions end-to-end |
| `state_does_not_regress_once_finalized`                    | Monotonicity invariant       |
| `list_active_traces_returns_all_entries`                   | Query API                    |
| `list_pending_traces_excludes_finalized_entries`           | Query API                    |
| `get_deploy_trace_returns_none_for_unknown_sig`            | Query API                    |
| `elapsed_secs_for_finalized_deploy_is_non_negative`        | Timing                       |

---

## Prior Deploy Ingress Path

The sections below document the previously-existing ingress path components
(HTTP and gRPC proxy sidecars, standalone server). These are unchanged and
complementary to the new deploy tracing module.

### Path Overview

```
HTTP POST /api/deploy              gRPC doDeploy(DeployDataProto)
         |                                        |
         ▼                                        ▼
  HttpDeployRequest                    DeployDataProto
         |                                        |
         ▼                                        ▼
  http_request_to_signed_deploy()     grpc_proto_to_signed_deploy()
         |                                        |
         └──────────┬─────────────────────────────┘
                    │ SignedDeployData
                    ▼
          LiveDeployIngress::observe_deploy()
                    │
                    ├── stores ObservedDeploy metadata
                    │     • signature, deployer, shard
                    │     • phlo price/limit, term length
                    │     • ingress source(s), observation count
                    │
                    ▼
          CordialCasper::deploy()
                    │
                    ├── DeployPool::add()  (unchanged admission)
                    │
                    ▼
          Either<DeployError, DeployId>
```

### Components

**1. Deploy Observation Types (`live_deploy_ingress.rs`)**

- `DeployIngressSource` — enum: `Grpc` or `Http`
- `ObservedDeploy` — metadata record for each observed deploy (signature,
  deployer, shard, phlo price/limit, term length, source(s), count)
- `LiveDeployIngress` — staged container with optional `DeployTracer`
- `HttpDeployRequest` — mirror of f1r3node's JSON deploy request body
- `HttpDeployConversionError` — hex decode failure for deployer/signature

**2. Conversion Functions (`live_deploy_ingress.rs`)**

| Function | Input | Output |
|---|---|---|
| `http_request_to_signed_deploy` | `&HttpDeployRequest` | `Result<SignedDeployData, HttpDeployConversionError>` |
| `grpc_proto_to_signed_deploy` | `&DeployDataProto` | `SignedDeployData` |

**3. HTTP Ingress Server (`http_deploy_ingress.rs`)**

Binary: `cargo run -p cordial-f1r3node-adapter --bin live_http_deploy_server`
  binds to `127.0.0.1:40412` (default), accepts `POST /api/deploy`.

**4. External gRPC Proxy (`live_deploy_proxy.rs`)**

Binary: `cargo run -p cordial-f1r3node-adapter --bin live_deploy_proxy`

**5. External HTTP Proxy (`live_http_deploy_proxy.rs`)**

Binary: `cargo run -p cordial-f1r3node-adapter --bin live_http_deploy_proxy`
  binds to `127.0.0.1:40413` (default), forwards to `http://127.0.0.1:40403`.

---

### Port Reference

| Binary | Binds to | Forwards to |
|---|---|---|
| `live_http_deploy_server` | `127.0.0.1:40412` | — (standalone) |
| `live_http_deploy_proxy` | `127.0.0.1:40413` | `:40403` (real node HTTP) |
| `live_deploy_proxy` | `127.0.0.1:40411` | `:40401` (real node gRPC) |

---

### Option 1 — Standalone HTTP server (no f1r3node required)

**Terminal 1 — start the server:**
```bash
cargo run -p cordial-f1r3node-adapter --bin live_http_deploy_server
```

**Terminal 2 — send a deploy:**
```bash
curl -s -X POST http://127.0.0.1:40412/api/deploy \
  -H 'Content-Type: application/json' \
  -d '{
    "data": {
      "term": "@0!(\"hello cordial\")",
      "time_stamp": 1000,
      "phlo_price": 1,
      "phlo_limit": 10000,
      "valid_after_block_number": 0,
      "shard_id": "root"
    },
    "deployer": "010101010101010101010101010101010101010101010101010101010101010101",
    "signature": "02020202020202020202020202020202020202020202020202020202020202020202020202020202020202020202020202020202020202020202020202020202",
    "sigAlgorithm": "ed25519"
  }' | python3 -m json.tool
```

---

### Option 2 — HTTP proxy sidecar

```bash
# Terminal 1 — start the standalone server
cargo run -p cordial-f1r3node-adapter --bin live_http_deploy_server

# Terminal 2 — start the proxy (pointing at standalone server)
cargo run -p cordial-f1r3node-adapter --bin live_http_deploy_proxy \
  -- --upstream-http-url http://127.0.0.1:40412

# Terminal 3 — send a deploy through the proxy (port 40413)
curl -s -X POST http://127.0.0.1:40413/api/deploy \
  -H 'Content-Type: application/json' \
  -d '{ ... same body as Option 1 ... }' | python3 -m json.tool
```

---

### Option 3 — gRPC proxy + `submit_live_deploy` against a live f1r3node

**Prerequisite:** f1r3node must be running with its gRPC API on `127.0.0.1:40401`.

```bash
# Terminal 1 — start the gRPC proxy
cargo run -p cordial-f1r3node-adapter --bin live_deploy_proxy \
  -- --upstream-grpc-url http://127.0.0.1:40401

# Terminal 2 — submit a real signed deploy through the proxy
cargo run -p cordial-f1r3node-adapter --bin submit_live_deploy \
  -- --grpc-url http://127.0.0.1:40411
```

---

## Troubleshooting

| Problem | Cause | Fix |
|---|---|---|
| `Address already in use` | Port already occupied | `lsof -ti :40412 \| xargs kill` |
| gRPC proxy exits on startup | Real node not reachable | Ensure f1r3node is running before starting the proxy |
| `400 Bad Request` on HTTP | Request body malformed | Check JSON matches the `HttpDeployRequest` shape |
| `invalid deployer hex` | deployer/signature field is not valid hex | Ensure both fields are lowercase hex-encoded bytes |
| `502 Bad Gateway` from proxy | Real upstream is down | Check the upstream node is healthy |
| Trace stuck at `BlockIncluded` | Block not yet finalized | Normal — wait for consensus to finalize the block |
| Trace stuck at `Accepted` | Block not yet produced | Normal — wait for the node to create a block |

---

## All Deploy Ingress Tests

```bash
# Run the new deploy trace integration tests:
cargo test -p cordial-f1r3node-adapter --test test_deploy_trace

# Run all deploy ingress tests:
cargo test -p cordial-f1r3node-adapter --test test_live_deploy_ingress
cargo test -p cordial-f1r3node-adapter --test test_live_deploy_proxy
cargo test -p cordial-f1r3node-adapter --test test_live_http_deploy_proxy
cargo test -p cordial-f1r3node-adapter --test test_http_deploy_ingress
cargo test -p cordial-f1r3node-adapter --test test_grpc_deploy_ingress

# Or run the entire adapter test suite:
cargo test -p cordial-f1r3node-adapter
```
