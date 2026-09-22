# App Event Extraction Pipeline

## Summary

This note documents the adapter-side helper that composes block scanning and
finalized-output extraction into one call.

The implemented path is:

```text
OrderedFinalizedOutput + &[BlockMessage] + starting_ordered_index
  -> app_event_block_scan::scan_app_deploys_by_block_hash
  -> app_event_extractor::extract_app_events
  -> AppEventExtraction
```

This is still an adapter bridge. It does not validate app payloads, execute
applications, verify signatures, or recompute consensus order.

## Implemented Files

| File | Role |
|------|------|
| `crates/cordial-f1r3node-adapter/src/app_event_extraction_pipeline.rs` | Composes block scanning with app-event extraction |
| `crates/cordial-f1r3node-adapter/tests/test_app_event_extraction_pipeline.rs` | Pipeline behavior tests |
| `crates/cordial-f1r3node-adapter/src/lib.rs` | Exports `app_event_extraction_pipeline` |

## API

The helper is:

```rust
pub fn extract_app_events_from_blocks(
    ordered_output: OrderedFinalizedOutput,
    blocks: &[BlockMessage],
    starting_ordered_index: u64,
) -> Result<AppEventExtraction, AppEventExtractionPipelineError>
```

The successful result is:

```rust
pub struct AppEventExtraction {
    pub events: Vec<AppEvent>,
    pub envelope_errors: Vec<AppEventEnvelopeScanError>,
}
```

`events` are ready for `cordial-app-runtime`.

`envelope_errors` are diagnostics for malformed deploys that declared
`cordial_app` data but could not be parsed. They are non-fatal, so unrelated
valid app deploys can still become app events.

## Ordering Contract

Final event order comes only from:

```text
OrderedFinalizedOutput.blocks
```

The `blocks` slice supplies deploy bodies and metadata. Its order does not
control app-event order.

Within a finalized block, app deploy order is preserved from
`BlockMessage.body.deploys`.

`starting_ordered_index` is passed through to the extractor, so callers can
extract finalized output in chunks without resetting the app-event cursor.

## Error Semantics

The pipeline error type wraps the lower-level fatal errors:

```rust
pub enum AppEventExtractionPipelineError {
    BlockScan(AppEventBlockScanError),
    Extraction(AppEventExtractionError),
}
```

Fatal scan errors include:

- duplicate block hashes

Fatal extraction errors include:

- finalized block body data missing from the scan result
- `ordered_index` overflow

Malformed app envelopes are not fatal at the pipeline level. They are returned
in `AppEventExtraction::envelope_errors`.

## Rust Concepts

### `From` And `?`

The pipeline implements:

```rust
impl From<AppEventBlockScanError> for AppEventExtractionPipelineError
impl From<AppEventExtractionError> for AppEventExtractionPipelineError
```

That lets the helper use `?` with both lower-level functions:

```rust
let scan = scan_app_deploys_by_block_hash(blocks)?;
let events = extract_app_events(input)?;
```

When either call returns an error, Rust converts it into
`AppEventExtractionPipelineError` before returning from the helper.

### Borrowing Blocks

The helper receives:

```rust
blocks: &[BlockMessage]
```

That means callers keep ownership of block messages. The pipeline borrows the
slice while scanning and returns owned app events and owned diagnostics.

## Tests

Pipeline tests cover:

- successful scan plus extraction
- finalized order taking priority over block slice order
- non-fatal malformed envelope reporting
- missing finalized block body rejection
- duplicate block hash rejection
- `starting_ordered_index` offset behavior

Run:

```text
cargo test -j1 -p cordial-f1r3node-adapter --test test_app_event_extraction_pipeline
```

Use `-j1` locally if linker resource pressure causes `rust-lld` to fail while
building the adapter's heavier binary targets.
