# App Event Envelope

## Summary

`AppEvent` is the app-neutral envelope consumed by `cordial-app-runtime`.

It is the boundary between finalized Cordial ordering and deterministic
application state machines:

```text
finalized ordered Cordial output
  -> AppEvent stream
  -> AppRuntime
  -> CordialApp validate/apply
```

The runtime does not know how an event was encoded inside a deploy. It only
requires a stable `AppEvent` value.

## Event Shape

The current event type is:

```rust
pub struct AppEvent {
    pub event_id: AppEventId,
    pub app_id: AppId,
    pub event_type: String,
    pub payload: Vec<u8>,
    pub submitter: Vec<u8>,
    pub ordered_index: u64,
    pub block_hash: Vec<u8>,
    pub deploy_signature: Option<Vec<u8>>,
    pub finalized_anchor: Vec<u8>,
}
```

## Field Semantics

| Field | Meaning |
|------|---------|
| `event_id` | Stable deterministic identity for this app event |
| `app_id` | Application namespace used by `AppRuntime` to route the event |
| `event_type` | App-defined event kind, such as `TaskPosted` or `PaymentSent` |
| `payload` | Opaque app payload bytes |
| `submitter` | Submitter identity bytes projected from deploy metadata |
| `ordered_index` | Position of this event in the finalized app-event stream |
| `block_hash` | Hash of the finalized block that contained the source deploy |
| `deploy_signature` | Optional deploy signature bytes, if available |
| `finalized_anchor` | Hash of the finalized leader anchor for this ordered output |

## Runtime Boundary

`cordial-app-runtime` consumes `AppEvent`s but does not extract them from
blocks. Extraction currently belongs to:

```text
crates/cordial-f1r3node-adapter/src/app_event_extractor.rs
```

This keeps the dependency direction clean:

```text
cordial-f1r3node-adapter -> cordial-app-runtime
```

The runtime remains free of `f1r3node`, gRPC, blocklace, tau ordering, finality,
and PoR internals.

## Ordered Index

`ordered_index` is the position in the app-event stream delivered to
`AppRuntime`.

It is not necessarily the block position. One finalized block may contain many
app events, and some finalized blocks may contain none.

Example:

```text
finalized block 0: two app events
finalized block 1: no app events
finalized block 2: one app event
```

The app-event indexes are:

```text
0, 1, 2
```

This gives the runtime a contiguous cursor over the event stream it actually
processes.

## Event ID Requirements

`AppEventId` must be deterministic. Given the same finalized output and the
same app deploy metadata, every node should derive the same event IDs.

The current adapter extractor derives event IDs by hashing a canonical
length-prefixed encoding of:

```text
domain tag
block_hash
deploy_index
app_id
event_type
payload
submitter
deploy_signature
```

Length prefixes prevent ambiguous byte concatenation. Optional fields must also
encode whether the value is present, so these remain distinct:

```text
deploy_signature = None
deploy_signature = Some([])
deploy_signature = Some([bytes])
```

The runtime treats `event_id` as the duplicate-protection key. It does not
recompute event IDs itself.

## Payload Semantics

`payload` is opaque to the generic runtime.

Concrete applications decide how to decode it. For example:

```text
ai.marketplace     -> marketplace event codec
payments.ledger    -> payment event codec
identity.registry  -> identity event codec
```

The generic runtime should not contain these codecs.

## Validation Semantics

Consensus validity and app validity are separate.

An event can be finalized by Cordial consensus but rejected by an application.
In that case:

```text
AppRuntime stores a rejected receipt
AppRuntime advances the cursor
The app state transition is not applied
```

This preserves auditability without rewriting finalized history.

## Out Of Scope

This document does not define:

- the final deploy envelope format
- Rholang term parsing
- JSON or binary app payload codecs
- signature verification rules
- app-specific validation logic
- persistence or replay storage format

Those are separate follow-up design and implementation slices.
