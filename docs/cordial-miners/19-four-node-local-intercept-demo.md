# Retired local-intercept demo

The local-intercept demo required a modified `f1r3node` binary with an
in-node `--consensus cordial-miners` selector. The current `f1r3node-rust`
checkout does not provide that selector, so the old Compose file and verifier
are no longer part of this repository.

Use the [connected four-node cluster](../../docker/README.md) and its
[host-side Cordial ordering check](integration/13-four-node-ordering-convergence.md)
for the current convergence test. The node keeps its native consensus. The
Cordial adapter mirrors finalized blocks through gRPC.
