# Blocklace Implementation Documentation

This document describes the current state of the blocklace implementation, based on the formal definitions from the blocklace paper.

## Overview

The blocklace is a DAG-based data structure used in Byzantine fault-tolerant distributed systems. Each node in the network creates cryptographically signed blocks that reference predecessor blocks, forming a directed acyclic graph (a "lace" of blocks).

---

## Project Structure

```
src/
  lib.rs             -- Crate root; re-exports public types
  main.rs            -- Binary entry point (placeholder)
  block.rs           -- Block struct and free functions
  blocklace.rs       -- Blocklace struct (the core data structure)
  crypto.rs          -- SHA-256 hashing, ED25519 signing and verification
  types/
    mod.rs           -- Re-exports NodeId, BlockIdentity, BlockContent
    node_id.rs       -- NodeId type
    identity_id.rs   -- BlockIdentity type
    content_id.rs    -- BlockContent type
tests/
  mod.rs             -- Shared test helpers (genesis, block_on, make_identity)
  test_block.rs      -- Unit tests for Block
  test_blocklace.rs  -- Unit tests for Blocklace
  test_hash.rs       -- Unit tests for SHA-256 content hashing
  test_sign.rs       -- Unit tests for ED25519 signing and verification
```

---

## Core Types

### `NodeId` (src/types/node_id.rs)

Represents a node's identity in the network (in practice, a public key).

```rust
pub struct NodeId(pub Vec<u8>);
```

Derives: `Debug`, `Clone`, `PartialEq`, `Eq`, `Hash`

### `BlockIdentity` (src/types/identity_id.rs)

The cryptographic identity of a block: `i = signedhash((v, P), k_p)`.

| Field          | Type       | Description                                |
|----------------|------------|--------------------------------------------|
| `content_hash` | `[u8; 32]` | SHA-256 hash of the serialized BlockContent |
| `creator`      | `NodeId`   | The node that signed this block            |
| `signature`    | `Vec<u8>`  | `sign(content_hash, creator_private_key)`  |

Derives: `Debug`, `Clone`, `PartialEq`, `Eq`, `Hash`

### `BlockContent` (src/types/content_id.rs)

The block content `C = (v, P)` -- a payload and a set of predecessor identities.

| Field          | Type                    | Description                              |
|----------------|-------------------------|------------------------------------------|
| `payload`      | `Vec<u8>`               | Arbitrary value `v` (operations, txns)   |
| `predecessors` | `HashSet<BlockIdentity>`| Set `P` of predecessor block identities  |

A block is **initial (genesis)** when `predecessors` is empty.

### `Block` (src/block.rs)

A complete block combining identity and content.

| Field      | Type            | Description        |
|------------|-----------------|--------------------|
| `identity` | `BlockIdentity` | The block's id `i` |
| `content`  | `BlockContent`  | The content `C`    |

**Methods:**

| Method              | Signature                              | Description                                         |
|---------------------|----------------------------------------|-----------------------------------------------------|
| `is_initial()`      | `&self -> bool`                        | True if the block has no predecessors (genesis)     |
| `node()`            | `&self -> &NodeId`                     | Returns the creator of the block: `node(b) = p`    |
| `id()`              | `&self -> &BlockIdentity`              | Returns the block's identity: `id(b) = i`          |
| `is_pointed_from()` | `&self, other: &Block -> bool`         | True if `other` lists `self` as a predecessor       |

**Free functions:**

| Function   | Signature                              | Description                                     |
|------------|----------------------------------------|-------------------------------------------------|
| `nodes()`  | `&[Block] -> HashSet<&NodeId>`         | `nodes(S)` -- set of creators in a block slice  |
| `ids()`    | `&[Block] -> HashSet<&BlockIdentity>`  | `id(S)` -- set of identities in a block slice   |

Equality and hashing are based solely on `BlockIdentity`, so blocks can live in `HashSet`.

---

## Blocklace (src/blocklace.rs)

The central data structure -- a set of blocks stored as `HashMap<BlockIdentity, BlockContent>`.

### Invariants

Two invariants are enforced at all times:

1. **CLOSED**: Every predecessor referenced by any block must exist in the blocklace.
   `forall (i, (v, P)) in B: P is a subset of dom(B)`

2. **CHAIN**: All blocks from a correct node are totally ordered under the precedence relation.
   `node(a) = node(b) = p => a precedes b OR b precedes a`

### Construction

| Method  | Signature    | Description               |
|---------|-------------|---------------------------|
| `new()` | `-> Self`   | Creates an empty blocklace |

### Map-View Accessors (Definition 2.3)

| Method      | Signature                                           | Description                                       |
|-------------|-----------------------------------------------------|---------------------------------------------------|
| `content()` | `&self, id: &BlockIdentity -> Option<&BlockContent>` | `B(b)` -- get content by identity                 |
| `get()`     | `&self, id: &BlockIdentity -> Option<Block>`         | `B[b]` -- get full block by identity              |
| `get_set()` | `&self, ids: &HashSet<BlockIdentity> -> HashSet<Block>` | `B[P]` -- get all blocks matching a set of ids |
| `dom()`     | `&self -> HashSet<&BlockIdentity>`                   | `dom(B)` -- set of all known block identities     |

### Insertion and Closure Axiom

| Method       | Signature                               | Description                                                  |
|--------------|-----------------------------------------|--------------------------------------------------------------|
| `insert()`   | `&mut self, block: Block -> Result<(), String>` | Insert a block; returns `Err` if any predecessor is missing |
| `is_closed()`| `&self -> bool`                         | Verify the closure axiom holds for the entire blocklace      |

### Pointed Relation (Definition 2.2)

| Method           | Signature                                             | Description                                         |
|------------------|-------------------------------------------------------|-----------------------------------------------------|
| `predecessors()` | `&self, id: &BlockIdentity -> HashSet<Block>`          | `<-b` -- direct predecessors of block `b`           |
| `ancestors()`    | `&self, id: BlockIdentity -> HashSet<Block>`           | `<b` -- transitive closure (all ancestors, not `b`) |
| `ancestors_inclusive()` | `&self, id: &BlockIdentity -> HashSet<Block>`    | `<=b` -- ancestors of `b` including `b` itself      |
| `ancestors_of_set()`   | `&self, ids: &HashSet<BlockIdentity> -> HashSet<Block>` | `<S` -- ancestors of any block in set `S`    |
| `precedes()`     | `&self, a: &BlockIdentity, b: &BlockIdentity -> bool`  | `a < b` -- true if `a` is in `b`'s ancestry        |
| `preceedes_or_equals()` | `&self, a: &BlockIdentity, b: &BlockIdentity -> bool` | `a <= b` -- precedes or equal              |

### Chain Axiom and Byzantine Detection

| Method                      | Signature                          | Description                                              |
|-----------------------------|------------------------------------|----------------------------------------------------------|
| `blocks_by()`               | `&self, node: &NodeId -> Vec<Block>` | All blocks created by node `p`                          |
| `satisfies_chain_axiom()`   | `&self, node: &NodeId -> bool`     | Check CHAIN axiom for a specific node                    |
| `satisfies_chain_axiom_all()` | `&self -> bool`                  | Check CHAIN axiom for every node                         |
| `find_equivacators()`       | `&self -> HashSet<NodeId>`         | Returns nodes violating CHAIN (Byzantine equivocators)   |
| `tip_of()`                  | `&self, node: &NodeId -> Option<Block>` | The most recent block of node `p` (chain tip)       |
| `all_nodes()`               | `&self -> HashSet<NodeId>` (private) | Collect all node ids present in the blocklace          |

---

## Cryptography (src/crypto.rs)

Real cryptographic operations using `sha2` and `ed25519-dalek` crates.

### Dependencies

| Crate            | Version | Purpose                        |
|------------------|---------|--------------------------------|
| `sha2`           | 0.10    | SHA-256 content hashing        |
| `ed25519-dalek`  | 2       | ED25519 signing & verification |
| `rand`           | 0.8     | Key generation (tests)         |

### Functions

| Function         | Signature                                              | Description                                                       |
|------------------|--------------------------------------------------------|-------------------------------------------------------------------|
| `hash_content()` | `content: &BlockContent -> [u8; 32]`                   | Deterministic SHA-256 hash: length-prefixed payload + sorted predecessors |
| `sign()`         | `hash: &[u8; 32], private_key: &[u8] -> Vec<u8>`      | ED25519 signature (64 bytes) over a content hash                  |
| `verify()`       | `hash: &[u8; 32], public_key: &[u8], signature: &[u8] -> bool` | Verify an ED25519 signature against a public key         |

### Hashing Serialization Format

The hash is computed over a deterministic byte layout:
1. `payload_len` (8 bytes, little-endian) + `payload` bytes
2. `num_predecessors` (8 bytes, little-endian)
3. For each predecessor (sorted by `content_hash`):
   - `content_hash` (32 bytes)
   - `creator_len` (8 bytes) + `creator` bytes
   - `signature_len` (8 bytes) + `signature` bytes

Sorting predecessors by `content_hash` ensures the hash is independent of `HashSet` iteration order.

---

## Test Coverage

### Block Tests (tests/test_block.rs) -- 9 tests

| Test                                    | What it verifies                                       |
|-----------------------------------------|--------------------------------------------------------|
| `genesis_block_is_initial`              | `is_initial()` returns true for empty predecessors     |
| `block_with_predecessors_is_not_initial`| `is_initial()` returns false when predecessors exist   |
| `node_returns_creator`                  | `node()` returns the block's creator NodeId            |
| `id_returns_identity`                   | `id()` returns the block's BlockIdentity               |
| `pointed_from_directs_direct_reference` | `is_pointed_from()` detects direct predecessor link    |
| `pointed_from_is_false_when_no_reference`| `is_pointed_from()` returns false with no link        |
| `same_identity_means_equal_blocks`      | Blocks with same identity are equal                    |
| `different_tag_means_different_blocks`  | Blocks with different identity are not equal           |
| `block_can_live_in_hashset`             | Hash/Eq impl allows blocks in HashSet                  |

### Blocklace Tests (tests/test_blocklace.rs) -- 7 tests

| Test                                        | What it verifies                                        |
|---------------------------------------------|---------------------------------------------------------|
| `genesis_can_be_inserted_into_empty_blocklace` | Genesis block (no predecessors) inserts successfully |
| `block_with_known_predecessor_can_be_inserted` | Block referencing a known predecessor inserts; closure holds |
| `inserting_block_with_unknown_predecessor_fails` | Insert returns Err for unknown predecessor (CLOSED axiom) |
| `content_returns_none_for_unknown_id`       | `content()` returns None for an id not in the blocklace |
| `get_returns_full_block_after_insert`       | `get()` returns the full block after insertion          |
| `get_set_returns_all_requested_blocks`      | `get_set()` returns all blocks matching requested ids   |
| `dom_contains_all_inserted_identities`      | `dom()` contains identities of all inserted blocks      |

### Hash Tests (tests/test_hash.rs) -- 5 tests

| Test                                          | What it verifies                                          |
|-----------------------------------------------|-----------------------------------------------------------|
| `hash_of_empty_genesis_is_deterministic`      | Same content always produces the same hash                |
| `different_payloads_produce_different_hashes`  | Different payloads yield different hashes                 |
| `different_predecessors_produce_different_hashes` | Different predecessor sets yield different hashes      |
| `hash_is_independent_of_predecessor_insertion_order` | Hash is deterministic regardless of set ordering  |
| `hash_output_is_32_bytes`                     | Output is always 32 bytes (SHA-256)                       |

### Sign/Verify Tests (tests/test_sign.rs) -- 6 tests

| Test                                          | What it verifies                                          |
|-----------------------------------------------|-----------------------------------------------------------|
| `sign_and_verify_roundtrip`                   | Sign then verify succeeds with correct keypair            |
| `signature_is_64_bytes`                       | ED25519 signature output is 64 bytes                      |
| `verify_fails_with_wrong_public_key`          | Verification rejects a different public key               |
| `verify_fails_with_tampered_hash`             | Verification rejects a modified hash                      |
| `verify_fails_with_tampered_signature`        | Verification rejects a modified signature                 |
| `different_content_produces_different_signatures` | Different hashes produce different signatures         |

### Test Helpers (tests/mod.rs)

Shared utilities to reduce boilerplate in tests:

| Helper            | Description                                              |
|-------------------|----------------------------------------------------------|
| `node(byte)`      | Build a `NodeId` from a single byte                     |
| `private_key(byte)` | Build a stub private key                              |
| `make_identity(creator, tag)` | Build a `BlockIdentity` without real crypto   |
| `genesis(creator, tag)` | Build a genesis block for a given creator           |
| `block_on(creator, tag, predecessors)` | Build a block pointing to predecessors  |

---

## What Is Not Yet Implemented

- **Persistence / serialization**: The blocklace is entirely in-memory with no disk storage.
- **Networking**: No peer-to-peer communication or block propagation.
- **Conflict resolution / consensus**: The structure detects Byzantine equivocators but does not implement a consensus protocol.
