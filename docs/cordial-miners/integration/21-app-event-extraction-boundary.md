# App Event Extraction Boundary

## Summary

This note documents the adapter-side boundary that turns finalized Cordial
ordering plus app deploy metadata into the app-neutral event stream consumed by
`cordial-app-runtime`.

The implemented path is:

```text
OrderedFinalizedOutput
  + decoded app deploy metadata by block hash
  -> cordial-f1r3node-adapter::app_event_extractor
  -> Vec<AppEvent>
  -> cordial-app-runtime::AppRuntime
```

The extractor is intentionally small. It preserves finalized order, projects
metadata into `AppEvent`, and leaves payload decoding and app semantics to later
layers.

## Why This Boundary Lives In The Adapter

`cordial-app-runtime` owns the app-neutral vocabulary and deterministic runtime:

- `AppEvent`
- `AppEventId`
- `AppId`
- `AppRuntime`
- `CordialApp`
- receipts, cursors, and snapshots

The adapter owns the `f1r3node` and Cordial Miners integration side:

- deploy and block observation
- mirrored blocklace state
- finalized tau ordering
- `OrderedFinalizedOutput`
- block hash and deploy metadata correlation

That means the dependency direction is:

```text
cordial-f1r3node-adapter -> cordial-app-runtime
```

The runtime must not depend on the adapter. Applications should be able to use
`cordial-app-runtime` without pulling in `f1r3node`, gRPC, blocklace, tau
ordering, or PoR internals.

## Implemented Files

| File | Role |
|------|------|
| `crates/cordial-f1r3node-adapter/src/app_event_extractor.rs` | Extraction module |
| `crates/cordial-f1r3node-adapter/tests/test_app_event_extractor.rs` | Boundary behavior tests |
| `crates/cordial-f1r3node-adapter/src/app_event_envelope.rs` | Minimal deploy-envelope parser that can produce `ExtractableAppDeploy` |
| `crates/cordial-f1r3node-adapter/tests/test_app_event_envelope.rs` | Parser behavior tests |
| `crates/cordial-f1r3node-adapter/src/app_event_block_scan.rs` | Scans block messages into `deploys_by_block_hash` |
| `crates/cordial-f1r3node-adapter/tests/test_app_event_block_scan.rs` | Scanner behavior tests |
| `crates/cordial-f1r3node-adapter/Cargo.toml` | Adds the adapter dependency on `cordial-app-runtime` |
| `crates/cordial-f1r3node-adapter/src/lib.rs` | Exports app-event parser, scanner, and extractor modules |

## Input Model

The extractor receives:

```rust
pub struct AppEventExtractionInput {
    pub ordered_output: OrderedFinalizedOutput,
    pub deploys_by_block_hash: BTreeMap<Vec<u8>, Vec<ExtractableAppDeploy>>,
}
```

`ordered_output.blocks` provides finalized block order.

`deploys_by_block_hash` provides app deploy metadata keyed by block content
hash. The deploy vector for each block is already in the block-local order that
must be preserved.

The extractor does not inspect block payloads directly. The minimal envelope
parser documented in
[22-app-event-envelope-parser.md](./22-app-event-envelope-parser.md) can produce
`ExtractableAppDeploy` values from deploy terms. The scanner documented in
[23-app-event-block-scan.md](./23-app-event-block-scan.md) can group those
parsed values by block hash.

## Extractable App Deploy

```rust
pub struct ExtractableAppDeploy {
    pub app_id: AppId,
    pub event_type: String,
    pub payload: Vec<u8>,
    pub submitter: Vec<u8>,
    pub deploy_signature: Option<Vec<u8>>,
}
```

This is the app metadata needed to create an `AppEvent`.

The payload is opaque bytes. The extractor does not know whether those bytes
represent JSON, bincode, Rholang data, marketplace actions, payment operations,
or anything else.

## Extraction Rules

`extract_app_events` follows these rules:

1. Iterate `OrderedFinalizedOutput.blocks` exactly as provided.
2. For each block, look up app deploy metadata by block hash.
3. Skip blocks with no app deploy metadata.
4. Preserve deploy order within each block.
5. Create one `AppEvent` per `ExtractableAppDeploy`.
6. Copy the finalized anchor hash into each emitted event.
7. Copy the containing block hash into each emitted event.
8. Assign contiguous app-event `ordered_index` values.
9. Generate deterministic `AppEventId` values.
10. Leave payload bytes untouched.

The extractor does not sort finalized blocks. It trusts the caller to provide
`OrderedFinalizedOutput` from the already-finalized adapter path.

## Ordered Index

`AppEvent::ordered_index` is a global app-event index emitted by the extractor,
not a block index.

Example:

```text
block A contains app deploys A0, A1
block B contains no app deploys
block C contains app deploy C0
```

The emitted indexes are:

```text
A0 -> ordered_index 0
A1 -> ordered_index 1
C0 -> ordered_index 2
```

This matches `AppRuntime`, which consumes a stream of app events rather than a
stream of blocks. Blocks with no app events do not create cursor gaps at the
application layer.

## Event ID

`AppEventId` is a SHA-256 hex digest over a canonical byte encoding of:

```text
domain tag: "cordial-app-event:v1"
block_hash
deploy_index within the block
app_id
event_type
payload
submitter
deploy_signature
```

Each byte slice is length-prefixed before hashing. Length prefixes prevent
ambiguous concatenation. For example, these logical inputs must not share an
encoding:

```text
["ab", "c"]
["a", "bc"]
```

Optional deploy signatures include a presence byte:

```text
None        -> missing signature
Some([])    -> present but empty signature
Some(bytes) -> present signature bytes
```

The extractor does not validate whether a signature is acceptable. It only
preserves the distinction so downstream validation can reason about the exact
event identity.

## Determinism Rules

The extractor avoids nondeterministic behavior:

- no wall-clock time
- no randomness
- no network calls
- no hash-map iteration dependence
- no app-specific payload parsing
- no consensus recomputation

`BTreeMap` is used for app deploy metadata input so tests and callers can keep
stable map behavior. Actual event order still comes from
`OrderedFinalizedOutput.blocks` plus the deploy vector order inside each block.

## Relationship To App Runtime

The extractor's output can be passed directly into:

```rust
AppRuntime::process_events(events)
```

The runtime then handles:

- app routing by `app_id`
- duplicate `event_id` protection
- app validation
- app application
- applied or rejected receipts
- cursor advancement
- snapshot queries

This keeps extraction and application execution separate.

## Out Of Scope

This boundary does not implement:

- deciding the final app envelope format
- validating signatures
- validating app payload semantics
- mutating `OrderedFinalizedOutput`
- choosing finality or tau order
- direct `f1r3node` runtime execution
- marketplace, ledger, social feed, or reputation app logic

## Tests

The extractor behavior tests cover:

- empty ordered output produces no events
- ordered blocks produce events in finalized order
- multiple deploys in one block preserve deploy order
- blocks without app deploys are skipped
- finalized anchor is copied into each event
- event IDs are deterministic across repeated extraction
- opaque payload bytes are not decoded or mutated
- event fields are projected from `ExtractableAppDeploy`

Run:

```text
cargo test -p cordial-f1r3node-adapter --test test_app_event_extractor
```

## Next Step

The next implementation slice should compose scanned block deploy metadata with
`OrderedFinalizedOutput` in one convenience helper that returns `Vec<AppEvent>`.
