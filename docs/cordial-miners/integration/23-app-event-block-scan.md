# App Event Block Scan

## Summary

This note documents the scanner that builds the extractor's app-deploy input
from adapter block messages.

The implemented path is now:

```text
Vec<BlockMessage>
  -> app_event_block_scan::collect_app_deploys_by_block_hash
  -> BTreeMap<block_hash, Vec<ExtractableAppDeploy>>
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
pub fn collect_app_deploys_by_block_hash(
    blocks: &[BlockMessage],
) -> Result<BTreeMap<Vec<u8>, Vec<ExtractableAppDeploy>>, AppEventBlockScanError>
```

For each block, it reads:

```text
BlockMessage.body.deploys
```

Each processed deploy is passed to:

```rust
parse_app_deploy_envelope(&processed_deploy.deploy)
```

Only `Ok(Some(app_deploy))` values are inserted into the result map. Ordinary
non-app deploys return `Ok(None)` from the parser and are skipped.

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

It fails fast for two cases:

| Error | Meaning |
|------|---------|
| `DuplicateBlockHash` | The same block hash appeared twice in the scanned block list |
| `Envelope` | A deploy declared `cordial_app` data but its envelope was malformed |

Envelope errors include both:

- the containing block hash
- the deploy index within that block

That context makes malformed app deploys traceable without changing the app
runtime's deterministic processing model.

## Rust Concepts

### Borrowed Slices

The scanner accepts:

```rust
blocks: &[BlockMessage]
```

That means it borrows the block list. The caller keeps ownership of the blocks,
and the scanner clones only the fields needed in the returned map.

### `?` Error Propagation

The scanner calls the parser like this:

```rust
let parsed = parse_app_deploy_envelope(&processed_deploy.deploy).map_err(|source| {
    AppEventBlockScanError::Envelope {
        block_hash: block.block_hash.clone(),
        deploy_index,
        source,
    }
})?;
```

The `?` operator means:

```text
if Ok(value), keep going with value
if Err(error), return that error immediately
```

`map_err` adds scanner-specific context before the error leaves the function.

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
- reporting malformed envelopes with block hash and deploy index
- rejecting duplicate block hashes

Run:

```text
cargo test -p cordial-f1r3node-adapter --test test_app_event_block_scan
```

## Next Step

The next implementation slice should provide a small composition helper that
takes finalized ordered output plus the corresponding block messages and returns
the final `Vec<AppEvent>` in one call.
