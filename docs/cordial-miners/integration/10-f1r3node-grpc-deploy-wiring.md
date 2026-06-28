# f1r3node gRPC Deploy Wiring

This note records the first host-side wiring step that connects a real
`f1r3node` deploy handler to the Cordial Miners deploy observer seam.

## What Changed

The `f1r3node` gRPC deploy service now records decoded deploys in
`LiveDeployIngress` before handing them off to native Casper admission.

Patched host-side files:

- `f1r3node/node/Cargo.toml`
- `f1r3node/node/src/rust/api/deploy_grpc_service_v1.rs`

## Wiring Point

The integration happens at the exact seam identified earlier in the tracing
note:

- after `DeployDataProto` is decoded into `Signed<DeployData>`
- before `BlockAPI::deploy(...)` is called

This preserves the original `f1r3node` execution path while giving the Cordial
adapter a live deploy-side observation point.

## Host-Side Behavior

The patched gRPC service now:

1. receives `DeployDataProto`
2. decodes it into `Signed<DeployData>`
3. projects that decoded deploy into adapter-side `SignedDeployData`
4. records it in `LiveDeployIngress`
5. continues into `BlockAPI::deploy(...)` unchanged

In short:

`DeployDataProto -> decode -> observe in Cordial ingress -> native BlockAPI admission`

## Why This Matters

This is the first point where the live host node itself participates in the
Cordial deploy ingress story.

Before this change, the deploy observer existed only inside the adapter crate.
After this change, a real `f1r3node` gRPC deploy path can feed that observer in
live execution.

## What Stayed Unchanged

This wiring intentionally does **not**:

- replace native Casper deploy admission
- change proposer scheduling
- alter the node's deploy success / failure semantics
- route deploys into Cordial proposal logic yet

It is still an observer-first integration step.

## Verification

The following checks passed after the change:

- adapter deploy ingress tests
- adapter clippy checks
- `cargo check -p node` in the patched `f1r3node` repository

## Next Step

Run a live `f1r3node`, submit a real deploy through gRPC, and confirm that the
deploy is visible through the Cordial deploy ingress observer state.
