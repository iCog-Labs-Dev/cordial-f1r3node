# Full Deploy Ingress Path

This doc describes the complete deploy ingress path for Cordial Miners:

how decoded deploys from HTTP and gRPC ingress are captured, staged in
adapter-side runtime state, and prepared for follow-up proposal work.

## Path Overview

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

## Components

### 1. Deploy Observation Types (`live_deploy_ingress.rs`)

- `DeployIngressSource` — enum: `Grpc` or `Http`
- `ObservedDeploy` — metadata record for each observed deploy (signature,
  deployer, shard, phlo price/limit, term length, source(s), count)
- `LiveDeployIngress` — staged container: `HashMap<signature, ObservedDeploy>`
  with insertion-order tracking
- `HttpDeployRequest` — mirror of f1r3node's JSON deploy request body
- `HttpDeployConversionError` — hex decode failure for deployer/signature

### 2. Conversion Functions (`live_deploy_ingress.rs`)

| Function | Input | Output |
|---|---|---|
| `http_request_to_signed_deploy` | `&HttpDeployRequest` | `Result<SignedDeployData, HttpDeployConversionError>` |
| `grpc_proto_to_signed_deploy` | `&DeployDataProto` | `SignedDeployData` |

Both produce `SignedDeployData` (the adapter's wire-format deploy type with
`DeployData`, public key, signature, and sig algorithm).

### 3. Observation Entrypoints (`live_deploy_ingress.rs`)

| Method | Source | Behavior |
|---|---|---|
| `observe_grpc_deploy(&mut self, &SignedDeployData)` | gRPC | Record metadata |
| `observe_http_deploy(&mut self, &SignedDeployData)` | HTTP | Record metadata |
| `observe_grpc_proto_deploy(&mut self, &DeployDataProto)` | gRPC | Decode + observe |
| `observe_http_request_deploy(&mut self, &HttpDeployRequest)` | HTTP | Decode + observe |

### 4. Combined Observation + Admission (`live_deploy_ingress.rs`)

These methods observe the deploy AND pass it through the adapter's `deploy()`:

| Method | Input |
|---|---|
| `admit_grpc_deploy(deploy, adapter)` | `SignedDeployData` |
| `admit_grpc_proto_deploy(proto, adapter)` | `DeployDataProto` |
| `admit_http_deploy(deploy, adapter)` | `SignedDeployData` |
| `admit_http_request_deploy(request, adapter)` | `HttpDeployRequest` |

The admission call goes through `CordialCasper::deploy()` → `DeployPool::add()`
which is the same path as native Casper admission. The adapter's admission
behavior is identical to calling `BlockAPI::deploy(...)` on the real node.

### 5. HTTP Ingress Server (`http_deploy_ingress.rs`)

- `HttpDeployIngressState` — shared state wrapping `Arc<Mutex<LiveDeployIngress>>`
- `deploy_router(state)` — returns an axum `Router` with `POST /api/deploy`
  and `POST /api/v1/deploy` endpoints
- `handle_deploy(state, Json(request))` — axum handler, parses JSON body as
  [`HttpDeployRequest`], calls [`route_http_deploy`]
- `route_http_deploy(state, request)` — converts request to `SignedDeployData`,
  records in `LiveDeployIngress`, returns `DeployResponse`
- `route_http_deploy_with_admission(state, request, adapter)` — same as above
  plus adapter admission

Binary: `cargo run -p cordial-f1r3node-adapter --bin live_http_deploy_server`
  binds to `127.0.0.1:40412` (default), accepts `POST /api/deploy`.

### 6. gRPC Ingress Handler (`grpc_deploy_ingress.rs`)

- `GrpcDeployIngressHandler` — implements `DeployService` directly (no proxy)
- Uses an `AdmitFn` closure: `Arc<dyn Fn(SignedDeployData) -> Result<Either<DeployError, DeployId>, CasperError>>`
- `handle_do_deploy_inner(proto)` — observe + admit, returns `(ObservedDeploy, Either<DeployError, DeployId>)`
- `do_deploy(request)` — gRPC handler: on success returns a `DeployResponse` with
  deploy ID; on admission failure returns `tonic::Status::invalid_argument`; on
  internal error returns `tonic::Status::internal`

This is the "direct" handler that does not proxy upstream — unlike `LiveDeployProxy`
which observes + forwards. The direct handler is for embedded use where the
adapter owns the deploy admission.

### 7. External gRPC Proxy (`live_deploy_proxy.rs`)

- `LiveDeployProxy` — implements `DeployService`, observes `doDeploy` requests
  through `LiveDeployIngress`, then forwards to a real upstream `f1r3node` node
- `with_shared_ingress(client, ingress)` allows sharing observer state with a
  `LiveHttpDeployProxy` so gRPC- and HTTP-observed deploys land in one
  unified staged view
- Used for sidecar deployment where the node is not modified

Binary: `cargo run -p cordial-f1r3node-adapter --bin live_deploy_proxy`

### 8. External HTTP Proxy (`live_http_deploy_proxy.rs`)

- `LiveHttpDeployProxy` — transparent HTTP sidecar: sits in front of a real
  f1r3node HTTP API, observes `POST /api/deploy` and `POST /api/v1/deploy`
  requests through `LiveDeployIngress`, then forwards the *original* request
  body **and path** to the upstream and relays the real response back unchanged
  (a call to `/api/v1/deploy` reaches the upstream as `/api/v1/deploy`, not
  silently rewritten to `/api/deploy`)
- Observation failure (e.g. malformed hex) does not block forwarding — the
  upstream always sees the raw request on the original path
- `with_shared_ingress(...)` allows sharing observer state with a gRPC proxy
  so HTTP- and gRPC-observed deploys land in one unified staged view

Binary: `cargo run -p cordial-f1r3node-adapter --bin live_http_deploy_proxy`
  binds to `127.0.0.1:40413` (default), forwards to `http://127.0.0.1:40403`.

## Adapter Admission Unchanged

All admittance paths ultimately call `CordialCasper::deploy()` which:

1. Translates `SignedDeployData` → core `CmSignedDeploy`
2. Locks the deploy pool
3. Calls `DeployPool::add(cm_signed)`
4. Returns `Either<DeployError, DeployId>`

This is the same logic used when `BlockAPI::deploy(...)` calls
`casper.deploy(deploy_data)` in f1r3node's native path. The adapter's
admission behavior is unchanged by the observation layer.

## Live Tests

All three options were verified end-to-end. Commands are run from the repo root.

### Port reference

| Binary | Binds to | Forwards to |
|---|---|---|
| `live_http_deploy_server` | `127.0.0.1:40412` | — (standalone) |
| `live_http_deploy_proxy` | `127.0.0.1:40413` | `:40403` (real node HTTP) |
| `live_deploy_proxy` | `127.0.0.1:40411` | `:40401` (real node gRPC) |

---

### Option 1 — Standalone HTTP server (no f1r3node required)

The Cordial server accepts and observes deploys entirely locally. Nothing else
needs to be running.

**Terminal 1 — start the server:**
```bash
cargo run -p cordial-f1r3node-adapter --bin live_http_deploy_server
```

Expected startup output:
```
Cordial HTTP deploy ingress server
==================================
Bind address: 127.0.0.1:40412

POST /api/deploy    — observe a deploy (JSON HttpDeployRequest)
POST /api/v1/deploy — same handler (versioned path)
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

Expected response (`200 OK`):
```json
{
  "success": true,
  "message": "deploy accepted",
  "signature_hex": "0202...02",
  "deployer_hex": "0101...01",
  "observation_count": 1
}
```

Re-sending the same deploy returns `"observation_count": 1` — same-source
duplicate is suppressed. Sending with an invalid hex deployer/signature returns
`400 Bad Request` with `"success": false`.

---

### Option 2 — HTTP proxy sidecar in front of a standalone server or real node

The proxy observes every deploy and forwards the original request body and path
to the upstream unchanged. The upstream handles admission and the proxy relays
the response back.

Run without a real f1r3node by pointing the proxy at the standalone server
from Option 1:

**Terminal 1 — start the standalone server:**
```bash
cargo run -p cordial-f1r3node-adapter --bin live_http_deploy_server
```

**Terminal 2 — start the proxy:**
```bash
# Pointing at the standalone server (no real node needed)
cargo run -p cordial-f1r3node-adapter --bin live_http_deploy_proxy \
  -- --upstream-http-url http://127.0.0.1:40412

# Or pointing at a real f1r3node HTTP API
cargo run -p cordial-f1r3node-adapter --bin live_http_deploy_proxy \
  -- --upstream-http-url http://127.0.0.1:40403
```

Expected startup output:
```
Cordial live HTTP deploy proxy
==============================
Bind address:      127.0.0.1:40413
Upstream HTTP URL: http://127.0.0.1:40412
```

**Terminal 3 — send a deploy through the proxy (port 40413):**
```bash
curl -s -X POST http://127.0.0.1:40413/api/deploy \
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

The proxy observes the deploy in `LiveDeployIngress`, forwards to the upstream
on the original path (`/api/deploy` or `/api/v1/deploy`), and relays the
upstream response back unchanged. Observation failure (e.g. malformed hex)
does not block forwarding.

---

### Option 3 — gRPC proxy + `submit_live_deploy` against a live f1r3node

The gRPC proxy intercepts `doDeploy` calls and forwards them to a real
f1r3node node. `submit_live_deploy` constructs a real `secp256k1`-signed
deploy and submits it.

**Prerequisite:** f1r3node must be running with its gRPC API on `127.0.0.1:40401`.

**Terminal 1 — start the gRPC proxy:**
```bash
cargo run -p cordial-f1r3node-adapter --bin live_deploy_proxy \
  -- --upstream-grpc-url http://127.0.0.1:40401
```

Expected startup output:
```
Cordial live deploy proxy
=========================
Bind address:      127.0.0.1:40411
Upstream gRPC URL: http://127.0.0.1:40401
```

**Terminal 2 — submit a real signed deploy through the proxy:**
```bash
cargo run -p cordial-f1r3node-adapter --bin submit_live_deploy \
  -- --grpc-url http://127.0.0.1:40411
```

To customise the deploy:
```bash
cargo run -p cordial-f1r3node-adapter --bin submit_live_deploy -- \
  --grpc-url http://127.0.0.1:40411 \
  --term '@0!("my rholang term")' \
  --phlo-price 1 \
  --phlo-limit 100000 \
  --shard-id root \
  --valid-after-block-number 0
```

Expected output (verified against a live node):
```
Submitting live deploy
======================
gRPC URL:    http://127.0.0.1:40411
timestamp:   1785136159289
shard id:    root
term:        @0!("hello cordial")
sig algo:    secp256k1
language:    rholang
Deploy accepted: Success!
DeployId is: 3044022061372cd1495e3764d7d1fffdef52e4f355982f95eb0f05d00a90ae3d9e29bd0...
```

The proxy observes the deploy in `LiveDeployIngress`, forwards the
`DeployDataProto` to the real node, and relays back the real `DeployId`.

---

### Troubleshooting

| Problem | Cause | Fix |
|---|---|---|
| `Address already in use` | Port already occupied | `lsof -ti :40412 \| xargs kill` |
| gRPC proxy exits on startup | Real node not reachable | Ensure f1r3node is running before starting the proxy |
| `400 Bad Request` on HTTP | Request body malformed | Check JSON matches the `HttpDeployRequest` shape |
| `invalid deployer hex` | deployer/signature field is not valid hex | Ensure both fields are lowercase hex-encoded bytes |
| `502 Bad Gateway` from proxy | Real upstream is down | Check the upstream node is healthy |

## Tests

| Test file | Coverage |
|---|---|
| `test_live_deploy_ingress.rs` | Observer types, conversions, combined observe+admit for gRPC/HTTP |
| `test_live_deploy_proxy.rs` | gRPC proxy: observe + forward + passthrough |
| `test_live_http_deploy_proxy.rs` | HTTP proxy: observe + forward, shared ingress, upstream error, v1 endpoint, observe failure passthrough |
| `test_http_deploy_ingress.rs` | HTTP handler: observation, admission, rejection, duplicate merge, server endpoint |
| `test_grpc_deploy_ingress.rs` | Direct gRPC handler: observe+admit, rejection, duplicate merge, DeployService integration |

To run all deploy ingress tests:

```bash
cargo test -p cordial-f1r3node-adapter --test test_live_deploy_ingress
cargo test -p cordial-f1r3node-adapter --test test_live_deploy_proxy
cargo test -p cordial-f1r3node-adapter --test test_live_http_deploy_proxy
cargo test -p cordial-f1r3node-adapter --test test_http_deploy_ingress
cargo test -p cordial-f1r3node-adapter --test test_grpc_deploy_ingress
```

Or run them all at once:

```bash
cargo test -p cordial-f1r3node-adapter
```

## Follow-Up Integration Steps

With this ingress path in place, the next steps are:

1. **Connect observed deploys to proposal** — wire `LiveDeployIngress::staged_deploys`
   into the proposer path in `proposer.rs` so the Cordial block producer can
   include observed (not just admitted) deploys.

2. **Deploy-to-block correlation** — compare `ObservedDeploy` signatures against
   deploys that appear in finalized blocks to measure end-to-end latency.

3. **Cordial proposal awareness** — if the adapter should propose its own blocks,
   the deploy observer state feeds into `CordialProposer::propose()` which
   already consumes `DeployPool::select_for_block()`.

4. **Native cutover** — once the deploy observer is validated in production,
   the interception can move from observation-only to becoming the primary
   deploy admission path, with f1r3node's `BlockAPI::deploy` as a fallback.
