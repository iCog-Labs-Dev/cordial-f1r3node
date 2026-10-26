# gRPC block ingestion

The adapter has two block input paths in
`crates/cordial-f1r3node-adapter/src/grpc_ingest.rs`. They use different hash
and signature domains and must not be mixed.

## Native f1r3node blocks

`GrpcBlockMapper::from_protobuf` accepts a real f1r3node `BlockMessage` without
parents. For a block with parents, call `from_protobuf_with_parents` with a
`NativeBlockIdentityIndex` built from previously accepted native blocks.

The mapper:

1. Recomputes the host wire hash with `casper::rust::util::proto_util::hash_block`
   and checks the message's declared `block_hash`.
2. Verifies the host signature against that wire hash and the sender key.
3. Translates the host body into a Cordial payload. The translated payload can
   omit host execution details, so its Cordial content hash is a separate hash.
4. Resolves every native parent and justification hash through the accepted
   index to a complete Cordial `BlockIdentity`. An unknown parent is an error.
   A justification that names the wrong creator is also an error.
5. Recomputes the Cordial content hash after replacing parent identities,
   because the predecessor set is part of Cordial block content.

The returned `ValidatedF1r3nodeBlock` carries the wire hash and wire signature
separately from `block.identity`. The translated Cordial identity has an empty
signature: the host signature proves the wire hash, not the Cordial content
hash. Insert it only through a boundary that has already checked the host
signature. A normal Cordial signature verifier cannot validate that empty
identity signature.

Call `NativeBlockIdentityIndex::check_candidate` before inserting a translated
block into the Cordial blocklace. After acceptance, call `record_accepted`
before translating children. The checks reject a wire hash remapped to another
Cordial identity and two distinct wire blocks that collapse to the same
Cordial identity. Checking before insertion avoids changing the blocklace on
a collision.

```text
signed host block -> verify host hash and signature
                  -> resolve accepted native parent hashes
                  -> compute Cordial content hash
                  -> check identity mapping for collisions
                  -> accept Cordial block
                  -> record wire hash to Cordial identity
```

`LiveIngress::ingest_native_block_message` owns that sequence for real host
messages in memory. For durable native ingress, open an
`RSpaceBlocklaceRepository`, initialize the ingress with
`LiveIngress::with_persistent_store`, and call
`ingest_native_block_message_and_persist(&message, &repo)` for each native
message. The persistent path verifies and resolves the block, checks mirror
closure and identity collisions, calls the adapter, writes the translated
block and both directions of its wire-hash mapping in one LMDB transaction,
then inserts it into the mirror. A known wire hash is idempotent and does not
repeat the adapter callback.

On restart, `with_persistent_store` replays Cordial blocks and rebuilds the
native index from the stored mappings. It fails startup if a mapping refers to
a block that did not replay or if recovered identities collide. Blocks ingested
through the in-memory method have no durable native mapping and cannot be
resumed as native blocks after restart. Unknown parents and an existing Cordial
block without its matching wire-hash mapping are rejected before the adapter
callback. If a block arrives before its parents, the caller must fetch or
buffer them. The local native mirror described below calls this entry point.

## Full native blocks from a local host node

The public `DeployService` block queries return `LightBlockInfo` summaries.
Those summaries do not contain the original block body, so rebuilding a
`BlockMessage` from them cannot reproduce its signed wire hash.

`LocalNativeBlockSource::open(node_data_dir)` opens the existing host block
storage on the same machine. `LiveGrpcBlockClient::native_blocks_by_heights`
queries block hashes by height, then reads the corresponding full
`BlockMessage` values from that storage. A missing block or a stored block
whose declared hash differs from the requested key is an error. The native
ingress mapper still verifies the full wire hash and signature before
acceptance. The source requires access to the host node's data directory.

`live_native_mirror` polls the public gRPC history by height and reads each
full block from the local host store. It fetches missing parents and
justifications by native hash before it persists a child. After it accepts
all returned blocks at each height, it advances a durable height cursor.
The host genesis has no signature. The runner admits it through a separate
root-only path after checking its wire hash, empty sender and signature,
height zero, and absence of parents. Later blocks still require valid
signatures.
On restart, it restores the Cordial blocklace and native identity index, then
resumes after that cursor. It also replays the last 16 heights by default to
pick up blocks that appeared late at an earlier height. A missing requested
height or a host head behind the cursor stops the runner.

Set `NODE_DATA_DIR` to the running node's `storage.data-dir`. For this
workspace's sibling `f1r3node-rust` checkout, `just run-standalone` uses
`../f1r3node-rust/run-local/data/standalone`. Use a separate directory for
the Cordial mirror:

```bash
NODE_DATA_DIR=../f1r3node-rust/run-local/data/standalone
cargo run -j 1 -p cordial-f1r3node-adapter --bin live_native_mirror -- \
  --grpc-url http://127.0.0.1:40401 \
  --node-data-dir "$NODE_DATA_DIR" \
  --data-dir "$HOME/cordial-native-mirror"
```

Run the same command after a restart. Add `--once` for a single catch-up pass.
For a Docker node, give the runner filesystem access to that node's data
volume and use its published gRPC port. The runner observes native blocks; it
does not activate PoR weights or alter host consensus.

On 2026-10-17, a live check against sibling `f1r3node-rust` 0.4.46
(`525f8e8`) mirrored native heights 0 through 17. Restarting with the same
mirror directory replayed the recent window and advanced to height 18. A
second restart at the same head admitted no duplicate blocks. This check used
the standalone node's gRPC port `40401` and local data directory above.

The default range is 32 heights. If the node's `api.max-blocks-limit` is
smaller, set `--batch-heights` to that limit or lower. An incomplete range
stops without advancing the cursor.

The cursor records heights seen through the public history query. That API
can omit old block bodies after host pruning. The bounded lookback catches
recent late blocks, but a late block older than the lookback can be missed.
The live validation slice must measure this behavior before treating the
mirror as complete.

## Adapter-owned blocks

`GrpcBlockMapper::from_adapter_message` handles locally constructed adapter
`BlockMessage` values. Their signatures cover Cordial's internal content hash.
Their declared hash can be the adapter snapshot hash or that internal content
hash. This path does not accept real host wire messages.

`LiveIngress::ingest_block_message` continues to use this adapter-owned path.
Real native blocks use `ingest_native_block_message`.

## Verification

Run the focused tests with:

```bash
cargo test -j 1 -p cordial-f1r3node-adapter --test test_grpc_ingest --offline
cargo test -j 1 -p cordial-f1r3node-adapter --test test_live_native_ingress --offline
cargo test -j 1 -p cordial-f1r3node-adapter --test test_native_block_source --offline
cargo test -j 1 -p cordial-f1r3node-adapter --test test_native_live_runner --offline
```

The signed parent–child test checks native hash verification, parent lookup,
Cordial predecessor closure, content rehashing, and justification creator
matching. A separate test checks that distinct host blocks cannot share one
translated Cordial identity.
The live-ingress tests cover signed parent–child admission, duplicates,
adapter rejection and retry, identity collisions, missing native mappings,
and LMDB restart recovery of a native parent before its child arrives.
