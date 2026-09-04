# Cordial App Runtime Docs

This folder contains the design and implementation notes for
`cordial-app-runtime`.

The crate is the generic application interface layer above Cordial Miners
finalized ordered output. It should stay app-neutral: future apps such as an AI
marketplace, payment ledger, or social feed should depend on this runtime
instead of depending directly on consensus or `f1r3node` adapter internals.

## Documents

1. [01-architecture.md](./01-architecture.md)
   - Defines the overall application interface architecture
   - Explains how `OrderedFinalizedOutput` becomes deterministic app events
   - Describes app events, receipts, cursors, replay, rejection handling, and
     multi-app routing
   - Positions future applications, including an AI marketplace, outside the
     generic runtime itself
2. [02-runtime-contract.md](./02-runtime-contract.md)
   - Documents the implemented in-memory event-processing behavior
   - Explains receipt, cursor, duplicate, rejection, and snapshot semantics
   - Records why the runtime uses stable collections such as `BTreeMap` and
     `BTreeSet`
   - Lists the verification commands for this implementation slice
3. [03-event-envelope.md](./03-event-envelope.md)
   - Defines the runtime-side meaning of the `AppEvent` envelope
   - Explains `event_id`, `ordered_index`, payload, signature, and anchor
     semantics
   - Clarifies that extraction belongs outside the generic runtime

## Planned Documents

Future notes should be added here as the crate grows:

4. `04-replay-and-persistence.md`
   - Snapshot format
   - Replay from finalized ordered history
   - Restart semantics
   - Storage backend options
5. `05-example-apps.md`
   - Tiny task board
   - AI marketplace direction
   - Payment ledger and social feed notes

## Current Implementation State

The current crate defines:

- `AppId`
- `AppEventId`
- `AppEvent`
- `AppCursor`
- `AppSnapshot`
- `AppReceipt`
- `AppReceiptStatus`
- `AppError`
- `CordialApp`
- `AppRuntime`

`AppRuntime` now includes in-memory event processing with:

- app registration
- ordered event application
- duplicate protection
- app-layer rejection receipts
- cursor advancement
- receipt and snapshot queries
- deterministic replay tests
