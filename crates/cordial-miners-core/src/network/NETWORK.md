# Network Module Documentation

This document covers the P2P networking layer for the blocklace, including peer communication, block propagation, and synchronization.

---

## Module Structure

```
src/network/
  mod.rs        -- Module root; re-exports Message, Peer, Node
  message.rs    -- Wire protocol message types
  peer.rs       -- TCP peer: bind, connect, handshake, send/recv
  node.rs       -- Node: ties Peer (networking) + Blocklace (state)
  NETWORK.md    -- This file

tests/
  test_message.rs              -- Serialization tests for handshake/keepalive messages
  test_message_propagation.rs  -- Serialization tests for block propagation messages
  test_peer.rs                 -- TCP peer binding, handshake, and connection tests
  test_node.rs                 -- Node message handling and block propagation tests
```

---

## Dependencies

| Crate    | Version | Purpose                                   |
|----------|---------|-------------------------------------------|
| `tokio`  | 1       | Async runtime, TCP listener/stream        |
| `serde`  | 1       | Serialize/Deserialize for wire types      |
| `bincode`| 1       | Compact binary encoding for TCP framing   |

---

## Wire Protocol (message.rs)

All messages are sent as **length-prefixed bincode**: 4 bytes big-endian length, then the bincode-serialized payload.

### Message Variants

| Variant           | Fields                              | Direction        | Description                                          |
|-------------------|-------------------------------------|------------------|------------------------------------------------------|
| `Hello`           | `node_id: Vec<u8>`, `listen_port: u16` | initiator -> listener | Handshake: announce identity and listening port   |
| `HelloAck`        | `node_id: Vec<u8>`                  | listener -> initiator | Handshake acknowledgement                         |
| `Ping`            | --                                  | either           | Keepalive heartbeat                                  |
| `Pong`            | --                                  | either           | Keepalive response                                   |
| `BroadcastBlock`  | `block: Block`                      | creator -> peers | Push a newly created block                           |
| `RequestBlock`    | `id: BlockIdentity`                 | either           | Request a specific block (missing predecessor)       |
| `BlockResponse`   | `block: Option<Block>`              | either           | Deliver requested block, or None if unknown          |
| `SyncRequest`     | --                                  | either           | Request peer's full set of block identities          |
| `SyncResponse`    | `block_ids: Vec<BlockIdentity>`     | either           | Peer's full dom() set for discovery                  |

---

## Peer (peer.rs)

A TCP peer that manages connections and message framing.

### Lifecycle

1. `Peer::bind(node_id, addr)` -- bind a TCP listener, spawn the accept loop
2. `peer.connect(remote_addr)` -- connect outbound, perform Hello/HelloAck handshake
3. `peer.send(addr, msg)` / `peer.recv()` -- exchange messages with connected peers

### API

| Method                 | Signature                                              | Description                                    |
|------------------------|--------------------------------------------------------|------------------------------------------------|
| `bind()`               | `async (Vec<u8>, &str) -> io::Result<Self>`            | Create peer, bind TCP listener                 |
| `listen_addr()`        | `&self -> SocketAddr`                                  | Address this peer listens on                   |
| `node_id()`            | `&self -> &[u8]`                                       | This peer's identity                           |
| `connect()`            | `async (&self, &str) -> io::Result<()>`                | Connect to remote, perform handshake           |
| `send()`               | `async (&self, SocketAddr, &Message) -> io::Result<()>`| Send a message to a peer                       |
| `recv()`               | `async (&self) -> Option<(SocketAddr, Message)>`       | Receive next incoming message                  |
| `connection_count()`   | `async (&self) -> usize`                               | Number of active connections                   |
| `connected_peers()`    | `async (&self) -> Vec<Vec<u8>>`                        | Node ids of connected peers                    |
| `connected_peer_addrs()` | `async (&self) -> Vec<SocketAddr>`                   | Socket addresses of connected peers            |

### Handshake Flow

```
  Initiator                    Listener
     |                            |
     |--- Hello {id, port} ------>|
     |                            |
     |<--- HelloAck {id} ---------|
     |                            |
     |   (connection registered   |
     |    on both sides)          |
```

### Wire Framing Functions

| Function         | Description                                              |
|------------------|----------------------------------------------------------|
| `send_message()` | Write 4-byte big-endian length + bincode payload         |
| `recv_message()` | Read 4-byte length, then read and deserialize payload    |

---

## Node (node.rs)

Ties a `Peer` (networking) and a `Blocklace` (state) together into a network participant.

### API

| Method             | Signature                                                   | Description                                               |
|--------------------|-------------------------------------------------------------|-----------------------------------------------------------|
| `bind()`           | `async (Vec<u8>, &str) -> io::Result<Self>`                 | Create node with peer and empty blocklace                 |
| `connect()`        | `async (&self, &str) -> io::Result<()>`                     | Connect to a remote peer                                  |
| `create_block()`   | `async (&self, Block) -> Result<(), String>`                | Insert locally + broadcast to all peers                   |
| `handle_message()` | `async (&self, SocketAddr, Message) -> Option<Message>`     | Dispatch incoming message, return optional response       |
| `sync_with()`      | `async (&self, SocketAddr) -> io::Result<()>`               | Send SyncRequest to discover missing blocks               |
| `run()`            | `async (&self)`                                             | Message processing loop (recv + handle + respond)         |

### Message Handling

| Incoming Message   | Action                                                      | Response             |
|--------------------|-------------------------------------------------------------|----------------------|
| `Ping`             | --                                                          | `Pong`               |
| `BroadcastBlock`   | Insert block (or request missing predecessors)              | None                 |
| `RequestBlock`     | Look up block in local blocklace                            | `BlockResponse`      |
| `BlockResponse`    | Insert block if present                                     | None                 |
| `SyncRequest`      | Collect dom() set                                           | `SyncResponse`       |
| `SyncResponse`     | Request any blocks we don't have                            | None (sends requests)|
| `Hello/HelloAck`   | Handled by Peer layer                                       | None                 |

### Block Propagation Flow

```
  Node A (creator)              Node B (receiver)
     |                               |
     | create_block(block)           |
     |  1. blocklace.insert(block)   |
     |  2. broadcast to all peers    |
     |                               |
     |--- BroadcastBlock {block} --->|
     |                               | receive_block(block)
     |                               |  if predecessors present:
     |                               |    blocklace.insert(block)
     |                               |  else:
     |<--- RequestBlock {pred_id} ---|    request missing predecessors
     |                               |
     |--- BlockResponse {block} --->|
     |                               | insert predecessor, then retry
```

### Sync Flow

```
  Node A                        Node B
     |                               |
     |--- SyncRequest -------------->|
     |                               |
     |<--- SyncResponse {ids} ------|
     |                               |
     | (compare with local dom)      |
     | (request missing blocks)      |
     |--- RequestBlock {id} -------->|
     |<--- BlockResponse {block} ---|
```

---

## Test Coverage

### Message Serialization (test_message.rs) -- 6 tests

| Test                                    | What it verifies                                |
|-----------------------------------------|-------------------------------------------------|
| `hello_serializes_and_deserializes`     | Hello roundtrips through bincode                |
| `hello_ack_serializes_and_deserializes` | HelloAck roundtrips                             |
| `ping_serializes_and_deserializes`      | Ping roundtrips                                 |
| `pong_serializes_and_deserializes`      | Pong roundtrips                                 |
| `different_messages_produce_different_bytes` | Each variant encodes uniquely              |
| `hello_with_empty_node_id_roundtrips`   | Edge case: empty node id                        |

### Propagation Message Serialization (test_message_propagation.rs) -- 7 tests

| Test                                    | What it verifies                                |
|-----------------------------------------|-------------------------------------------------|
| `broadcast_block_serializes_and_deserializes` | BroadcastBlock roundtrips                 |
| `request_block_serializes_and_deserializes`   | RequestBlock roundtrips                   |
| `block_response_with_block_roundtrips`  | BlockResponse with Some(block) roundtrips       |
| `block_response_none_roundtrips`        | BlockResponse with None roundtrips              |
| `sync_request_roundtrips`               | SyncRequest roundtrips                          |
| `sync_response_roundtrips`              | SyncResponse with ids roundtrips                |
| `sync_response_empty_roundtrips`        | SyncResponse with empty ids roundtrips          |

### Peer Tests (test_peer.rs) -- 7 tests

| Test                                    | What it verifies                                |
|-----------------------------------------|-------------------------------------------------|
| `peer_binds_and_reports_listen_addr`    | Bind assigns a real port                        |
| `peer_reports_its_node_id`              | node_id() returns correct identity              |
| `new_peer_has_zero_connections`         | Fresh peer has no connections                   |
| `two_peers_can_connect_via_handshake`   | Hello/HelloAck handshake registers both sides   |
| `peer_a_receives_hello_from_peer_b`     | Hello message is forwarded to application layer |
| `connect_to_nonexistent_peer_fails`     | Connect to bad address returns error            |
| `multiple_peers_can_connect_to_one`     | Server accepts multiple concurrent clients      |

### Node Tests (test_node.rs) -- 13 tests

| Test                                        | What it verifies                                    |
|---------------------------------------------|-----------------------------------------------------|
| `node_starts_with_empty_blocklace`          | New node has empty blocklace                        |
| `create_block_inserts_into_local_blocklace` | create_block() inserts into local state             |
| `create_block_with_missing_predecessor_fails` | Closure axiom enforced on create_block            |
| `create_block_chain_succeeds`               | Sequential blocks with predecessors insert OK       |
| `handle_ping_returns_pong`                  | Ping dispatches to Pong response                    |
| `handle_request_block_returns_known_block`  | RequestBlock returns the block if known             |
| `handle_request_block_returns_none_for_unknown` | RequestBlock returns None for missing block     |
| `handle_sync_request_returns_dom`           | SyncRequest returns all block identities            |
| `handle_broadcast_block_inserts_genesis`    | BroadcastBlock inserts a genesis block              |
| `handle_block_response_inserts_block`       | BlockResponse inserts the delivered block           |
| `handle_block_response_none_is_noop`        | BlockResponse with None does nothing                |
| `handle_hello_is_noop`                      | Hello is passed through (handled by Peer)           |
| `sync_request_on_empty_returns_empty`       | SyncRequest on empty blocklace returns empty list   |

---

## Known Limitations

- **No pending-block retry**: When a block is rejected due to missing predecessors, the original block is dropped after requesting predecessors. Once predecessors arrive, the original block is not re-attempted automatically.
- **No transitive sync**: SyncResponse triggers RequestBlock for missing ids, but those blocks may have predecessors we're also missing (would need to walk the chain recursively).
- **send() opens a new TCP connection**: Each `peer.send()` call opens a fresh TCP connection rather than reusing the established one.
