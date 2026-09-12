# App Event Block Scan

## Summary

This note documents the scanner that builds the extractor's app-deploy input
from adapter block messages.

The implemented path is now:

```text
Vec<BlockMessage>
  -> app_event_block_scan::scan_app_deploys_by_block_hash
  -> AppEventBlockScan
  + OrderedFinalizedOutput
  -> app_event_extractor::extract_app_events
  -> Vec<AppEvent>
```

This slice does not change finality, tau ordering, app validation, or runtime
application semantics. It only connects finalized block contents to the
already-implemented extraction boundary.

## Implemented Files

| File | Role |
|------|------|
| `crates/cordial-f1r3node-adapter/src/app_event_block_scan.rs` | Scans block messages and groups parsed app deploys by block hash |
| `crates/cordial-f1r3node-adapter/tests/test_app_event_block_scan.rs` | Scanner behavior tests |
| `crates/cordial-f1r3node-adapter/src/lib.rs` | Exports `app_event_block_scan` |

## Scanner Contract

The scanner accepts borrowed adapter block messages:

```rust
pub fn scan_app_deploys_by_block_hash(
    blocks: &[BlockMessage],
) -> Result<AppEventBlockScan, AppEventBlockScanError>
```

The successful scan result is:

```rust
pub struct AppEventBlockScan {
    pub deploys_by_block_hash: BTreeMap<Vec<u8>, Vec<ExtractableAppDeploy>>,
    pub envelope_errors: Vec<AppEventEnvelopeScanError>,
}
```

For each block, it reads:

```text
BlockMessage.body.deploys
```

Each processed deploy is passed to:

```rust
parse_app_deploy_envelope(&processed_deploy.deploy)
```

Only `Ok(Some(app_deploy))` values are inserted into that block's deploy vector.
Ordinary non-app deploys return `Ok(None)` from the parser and are skipped.

The scanner inserts a map entry for every scanned block. If a block has no valid
app deploys, its value is an empty vector. This is important because the
extractor treats an empty vector as "scanned and empty," while a missing entry
means the finalized block body was not available to the scanner.

## Ordering Rules

The scanner preserves deploy order inside each block.

The scanner does not decide final app-event order. Final app-event order still
comes from:

```text
OrderedFinalizedOutput.blocks
```

That separation matters:

- `app_event_block_scan` knows how to find app deploy envelopes in blocks
- `app_event_extractor` knows how to combine those deploys with finalized order
- `cordial-app-runtime` knows how to route and apply app events

## Map Key

The result is keyed by:

```text
BlockMessage.block_hash
```

The extractor later looks up deploys by each finalized block's content hash.
Adapter callers must pass block messages in the same hash domain used to build
`OrderedFinalizedOutput`.

## Error Semantics

The scanner returns:

```rust
Result<_, AppEventBlockScanError>
```

It fails fast only for errors that make the scan map ambiguous:

| Error | Meaning |
|------|---------|
| `DuplicateBlockHash` | The same block hash appeared twice in the scanned block list |

Malformed app envelopes are non-fatal. They are recorded in:

```rust
AppEventBlockScan::envelope_errors
```

Each envelope scan error includes:

- the containing block hash
- the deploy index within that block
- the parser error from `app_event_envelope`

That means one malformed app deploy cannot block unrelated valid finalized app
events from being extracted.

## Rust Concepts

### Borrowed Slices

The scanner accepts:

```rust
blocks: &[BlockMessage]
```

That means it borrows the block list. The caller keeps ownership of the blocks,
and the scanner clones only the fields needed in the returned map.

### Matching Parser Outcomes

The scanner calls the parser and handles the three parser outcomes explicitly:

```rust
match parse_app_deploy_envelope(&processed_deploy.deploy) {
    Ok(Some(app_deploy)) => app_deploys.push(app_deploy),
    Ok(None) => {}
    Err(source) => envelope_errors.push(AppEventEnvelopeScanError {
        block_hash: block.block_hash.clone(),
        deploy_index,
        source,
    }),
}
```

This is deliberately more forgiving than `?` propagation. `?` would return from
the whole scan on the first malformed app envelope. Here, malformed app
envelopes are collected for reporting while valid deploys from other blocks
remain available to the extractor.

Duplicate block hashes are still fatal because they could otherwise overwrite a
previous map entry for the same hash.

### `BTreeSet`

The scanner uses a `BTreeSet` to remember block hashes it has already seen.
That prevents a duplicate block hash from silently overwriting a previous map
entry.

## Tests

Scanner tests cover:

- empty block lists
- skipping ordinary non-app deploys
- grouping app deploys by block hash
- preserving app-deploy order inside a block
- reporting malformed envelopes without discarding unrelated valid deploys
- rejecting duplicate block hashes

Run:

```text
cargo test -p cordial-f1r3node-adapter --test test_app_event_block_scan
```

## Next Step

The next implementation slice should provide a small composition helper that
takes finalized ordered output plus the corresponding block messages and returns
the final `Vec<AppEvent>` in one call.
