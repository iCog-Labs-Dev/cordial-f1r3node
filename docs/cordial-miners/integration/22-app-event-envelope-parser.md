# App Event Envelope Parser

## Summary

This note documents the first concrete app-event envelope parser in the
`cordial-f1r3node-adapter`.

The parser turns a minimal JSON wrapper inside a `SignedDeployData` term into
the app deploy metadata consumed by the app-event extractor:

```text
SignedDeployData.data.term
  -> app_event_envelope::parse_app_deploy_envelope
  -> Option<ExtractableAppDeploy>
  -> app_event_extractor::extract_app_events
  -> Vec<AppEvent>
```

This is still a small bridge slice. It does not validate signatures, execute
Rholang, or decode app-specific payload semantics.

## Implemented Files

| File | Role |
|------|------|
| `crates/cordial-f1r3node-adapter/src/app_event_envelope.rs` | Minimal JSON deploy-envelope parser |
| `crates/cordial-f1r3node-adapter/tests/test_app_event_envelope.rs` | Parser behavior tests |
| `crates/cordial-f1r3node-adapter/src/lib.rs` | Exports `app_event_envelope` |

## Envelope Shape

The first supported envelope is a JSON object with a top-level `cordial_app`
field:

```json
{
  "cordial_app": {
    "version": 1,
    "app_id": "identity.registry",
    "event_type": "NameRegistered",
    "payload_hex": "616c696365"
  }
}
```

The `payload_hex` field is decoded into opaque payload bytes. In the example
above, `616c696365` becomes:

```text
alice
```

The parser does not interpret those bytes. The concrete application decides how
to decode its own payload later.

## Field Mapping

The parser maps the envelope and deploy metadata into:

```rust
pub struct ExtractableAppDeploy {
    pub app_id: AppId,
    pub event_type: String,
    pub payload: Vec<u8>,
    pub submitter: Vec<u8>,
    pub deploy_signature: Option<Vec<u8>>,
}
```

| Source | Destination |
|--------|-------------|
| `cordial_app.app_id` | `ExtractableAppDeploy::app_id` |
| `cordial_app.event_type` | `ExtractableAppDeploy::event_type` |
| `cordial_app.payload_hex` decoded as bytes | `ExtractableAppDeploy::payload` |
| `SignedDeployData.pk` | `ExtractableAppDeploy::submitter` |
| `SignedDeployData.sig` | `ExtractableAppDeploy::deploy_signature` |

## Parser Result Shape

The parser returns:

```rust
Result<Option<ExtractableAppDeploy>, AppEventEnvelopeError>
```

This shape is intentional because there are three distinct outcomes:

| Outcome | Meaning |
|---------|---------|
| `Ok(Some(deploy))` | The deploy contains a valid supported app envelope |
| `Ok(None)` | The deploy is not a Cordial app event |
| `Err(error)` | The deploy declared app-event data, but the envelope was malformed |

This is more precise than returning only `Option<T>` or only `Result<T, E>`.

`Option<T>` alone would be too weak because it could not explain malformed app
envelopes.

`Result<T, E>` alone would be too strict because ordinary non-app deploys are
not errors.

## Supported Version

The first parser supports:

```text
version = 1
```

Any other version returns:

```rust
AppEventEnvelopeError::UnsupportedVersion { version }
```

This gives us a stable upgrade path. A later parser can support version `2`
without changing how version `1` is interpreted.

## Error Semantics

The parser owns these error cases:

| Error | Meaning |
|------|---------|
| `InvalidJson` | The term looks like JSON but cannot be parsed |
| `InvalidEnvelope` | The `cordial_app` field exists but does not match the expected shape |
| `UnsupportedVersion` | The envelope version is not supported |
| `InvalidPayloadHex` | `payload_hex` is not valid hex |

Non-JSON deploy terms return `Ok(None)`. This lets ordinary Rholang deploys pass
through the adapter without being treated as app-runtime input.

JSON deploy terms without `cordial_app` also return `Ok(None)`.

## Rust Concepts

This slice uses a few important Rust patterns.

### `Result<Option<T>, E>`

This is a common Rust shape for parsers that need to distinguish:

```text
not mine
valid
mine but invalid
```

The outer `Result` says whether parsing failed.

The inner `Option` says whether the input was actually a Cordial app deploy.

### `serde::Deserialize`

The parser defines a small private struct:

```rust
#[derive(Debug, Deserialize)]
struct CordialAppEnvelope {
    version: u64,
    app_id: String,
    event_type: String,
    payload_hex: String,
}
```

`serde` maps JSON fields into this Rust struct. Keeping the struct private
means callers depend on `ExtractableAppDeploy`, not on the temporary JSON
format internals.

### `map_err`

The parser uses `map_err` to turn dependency errors into adapter-owned errors.

That means callers see:

```rust
AppEventEnvelopeError::InvalidPayloadHex(...)
```

instead of a raw `hex` crate error.

This keeps the public API stable even if the internal parsing library changes.

### Borrowing With `&SignedDeployData`

The parser accepts:

```rust
deploy: &SignedDeployData
```

That borrows the deploy instead of taking ownership. The caller can keep using
the deploy after parsing. The parser clones only the fields that become part of
the returned `ExtractableAppDeploy`.

## Determinism Rules

The parser avoids nondeterministic behavior:

- no wall-clock time
- no randomness
- no networking
- no app-specific state
- no runtime execution

Given the same deploy term, deployer key, and signature bytes, the parser
returns the same `ExtractableAppDeploy`.

## Relationship To Extraction

The parser creates one `ExtractableAppDeploy` from one deploy term.

The extractor later combines those deploys with finalized block order:

```text
Vec<ExtractableAppDeploy> by block hash
  + OrderedFinalizedOutput
  -> Vec<AppEvent>
```

This keeps parsing and ordering separate:

- parser: understands the deploy envelope shape
- extractor: understands finalized ordered block position
- runtime: understands app routing, receipts, cursor, and snapshots

## Out Of Scope

This parser does not implement:

- signature verification
- Rholang parsing
- app-specific payload decoding
- app-specific validation
- final production envelope format
- scanning finalized blocks for deploys
- building `deploys_by_block_hash`

Those are separate follow-up slices.

## Tests

Parser tests cover:

- non-JSON deploy terms are not app events
- JSON without `cordial_app` is not an app event
- valid envelopes project app metadata
- invalid JSON is reported
- malformed `cordial_app` fields are reported
- unsupported versions are reported
- invalid payload hex is reported

Run:

```text
cargo test -p cordial-f1r3node-adapter --test test_app_event_envelope
```

## Next Step

The next implementation slice should scan processed deploys from finalized
blocks, parse any `cordial_app` envelopes, and build the
`deploys_by_block_hash` input expected by the extractor.
