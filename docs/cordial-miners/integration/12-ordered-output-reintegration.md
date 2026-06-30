# Ordered Output Reintegration Seam

This note documents the next architectural step after deploy observation,
block mirroring, finality, and local ordering.

The current system can already:

- observe deploy ingress before proposal
- mirror live blocks from `f1r3node`
- compute local Cordial finality
- compute local Cordial ordering over mirrored blocks

What it does not do yet is reconnect that ordered Cordial output back into a
node-facing execution or proposal path.

## Goal

Identify the first clean reintegration seam where Cordial-ordered output can
move from observer-side state into node-facing behavior.

This document is about choosing the seam, not yet implementing it.

## What "reintegration" means

Reintegration does **not** mean simply mirroring more blocks.

It means that Cordial output stops being read-only and begins to influence one
of the downstream stages of node behavior.

At a high level, that could mean influencing:

- what deploys are considered ready for proposal
- what block content is constructed
- what finalized output is treated as authoritative
- or what ordered transaction sequence is exposed to higher layers

## Current split of responsibilities

Today the system is split like this:

### On the Cordial side

- deploys can be observed at ingress
- blocks can be mirrored into a local blocklace
- finality can be computed locally
- ordered output can be produced locally

### On the `f1r3node` side

- deploy admission is still authoritative
- proposal scheduling is still authoritative
- block construction is still authoritative
- execution and final output are still authoritative

So Cordial currently explains and reconstructs the flow, but does not yet
drive it.

## Candidate reintegration seams

There are three realistic seams.

### 1. Pre-proposal deploy staging

This seam sits between deploy observation and proposer selection.

Conceptually:

`deploy observed -> Cordial-side staging / filtering -> proposer sees staged deploys`

Pros:

- earliest possible influence point
- aligns with the deploy observer and proxy work already completed
- keeps the integration close to transaction ingress

Cons:

- requires stronger agreement on how Cordial deploy staging should affect
  proposer behavior
- does not directly use Cordial ordered block output yet

### 2. Proposer / block-creation seam

This seam sits where the node constructs a block from deploys.

Conceptually:

`deploy pool -> block creation -> Cordial-guided block content / selection`

Pros:

- very natural node-facing integration point
- close to existing adapter-side proposer/runtime work
- lets Cordial affect what gets packaged into blocks

Cons:

- more invasive than observation-only work
- still not the clearest place to apply already-produced ordered output

### 3. Post-finality ordered-output seam

This seam sits after Cordial has already produced finalized local ordering.

Conceptually:

`mirrored blocks -> Cordial finality -> Cordial ordering -> node-facing ordered output consumer`

Pros:

- directly matches the output Cordial already knows how to compute
- cleanest conceptual extension of current observer mode
- minimal ambiguity about what Cordial is contributing: ordered finalized output

Cons:

- later in the pipeline
- does not influence deploy admission or block creation immediately

## Recommended first seam

The recommended first reintegration seam is:

## `post-finality ordered-output export`

That means the first implementation should not try to take over proposer logic
yet. Instead, it should:

1. compute ordered finalized output from the mirrored blocklace
2. package that output in a node-facing representation
3. expose it through a stable adapter/runtime boundary
4. let later work decide how node components consume it

This is the best first seam because it matches the strongest thing the current
implementation already does reliably: produce local finalized ordering.

## Why this seam is better than jumping directly into proposer replacement

If we try to influence proposer logic too early, we mix several concerns:

- deploy admission
- scheduling
- block construction
- consensus output

That makes it harder to tell whether Cordial is being integrated cleanly or
whether we are just re-implementing node internals.

By starting at the ordered-output seam, we keep the first reintegration step
small and conceptually honest:

- Cordial computes ordered finalized output
- the adapter exports that output
- downstream consumers can be attached incrementally

## Existing components we can reuse

In this repository:

- `crates/cordial-miners-core/src/consensus/finality.rs`
  - finalized leader logic
- `crates/cordial-miners-core/src/consensus/ordering.rs`
  - tau ordering logic
- `crates/cordial-f1r3node-adapter/src/snapshot.rs`
  - snapshot and finalized view helpers
- `crates/cordial-f1r3node-adapter/src/live_ingress.rs`
  - live mirrored blocklace state
- `crates/cordial-f1r3node-adapter/src/bin/live_mirror_check.rs`
  - already demonstrates live finality/order evaluation against a running node

In `f1r3node`:

- proposer-related code is useful context, but does not need to be the first
  reintegration target

## Proposed implementation sequence

The next implementation sequence should be:

1. define an adapter-side ordered-output export type
2. expose the latest finalized ordered fragment from live mirrored state
3. document or prototype one node-facing consumer of that ordered output
4. only after that, consider whether proposer-side integration is needed

## Suggested next issue

`Expose Cordial finalized ordered output through a stable adapter-side export seam`

That issue should focus on:

- defining the ordered-output representation
- exporting ordered finalized fragments from live adapter state
- clarifying what downstream node-facing consumer will read that output

## Outcome

This note establishes that:

- reintegration should be deliberate, not mixed into deploy observation work
- the cleanest first seam is post-finality ordered-output export
- proposer replacement or deploy-driven control should come later, after
  ordered output has a stable consumer boundary
