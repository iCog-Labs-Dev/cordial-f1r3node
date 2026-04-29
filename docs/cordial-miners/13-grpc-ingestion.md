# 13. gRPC Ingestion Layer: Network → Consensus Translation

**Implementation**: `crates/cordial-f1r3node-adapter/src/grpc_ingest.rs`  
**Module**: `cordial_f1r3node_adapter::grpc_ingest`  
**Tests**: `crates/cordial-f1r3node-adapter/tests/test_grpc_ingest.rs` (26 tests)  
**Purpose**: Safe translation from network-level messages (gRPC/bincode) to consensus-level blocks

## Architecture Overview

The gRPC ingestion layer implements a **fail-fast, stateless translation pipeline** that sits between the P2P network and the consensus core:

```
Network (P2P Node)
    ↓
  bincode deserialization
    ↓
  Message enum
    ↓
┌─────────────────────────────────────┐
│    GrpcBlockMapper<V, P, Id>        │  (pure, deterministic)
│                                     │  - Extract BroadcastBlock
│  Structural validation:             │  - Verify content hash (SHA-256)
│  • Message type                     │  - Verify ED25519 signature
│  • Content hash                     │  - Validate parent references
│  • ED25519 signature                │
│  • Parent integrity                 │
└─────────────────────────────────────┘
    ↓
  Block (valid or error)
    ↓
┌─────────────────────────────────────┐
│   BlocklaceAdapter<Id>              │  (stateful, side effects)
│                                     │
│  Semantic validation:               │  invoke:
│  • ConsensusEngine::on_block        │  • Closure axiom checks
│  • Blocklace insertion              │  • Fork choice updates
│  • Finality computation             │  • Tipset maintenance
└─────────────────────────────────────┘
    ↓
  Blocklace (updated or error)
```

## Design Principles

### 1. Fail-Fast
Blocks with invalid structure are rejected **immediately** with specific, actionable error messages:
- Content hash mismatch → clear expected vs. actual hashes
- Signature verification failure → clear reason
- Malformed parent references → specific field and validation rule

### 2. Pure Mapping
The `GrpcBlockMapper` is **stateless and deterministic**:
- No side effects (no database writes, no state mutations)
- Same input → same output (testable, reproducible)
- Can be cloned and shared across threads
- Multiple invocations with identical input produce identical results

### 3. Separation of Concerns
**Mapper** validates structure; **Adapter** handles semantics:
- Mapper: cryptographic signatures, hash integrity, reference well-formedness
- Adapter: closure axiom, chain axiom, cordial condition, finality rules

### 4. Type Genericity
Type parameters `<V, P, Id>` are reserved for future extension:
```rust
pub struct GrpcBlockMapper<V = (), P = (), Id = ()> { ... }
```
Current implementations use unit types `()`, but the API is forward-compatible for:
- Custom validator types `V`
- Custom payload types `P`  
- Custom block identity types `Id`

## Public API

### `GrpcBlockMapper<V, P, Id>`

**Constructor**:
```rust
pub fn new() -> Self
```

**Mapping method**:
```rust
pub fn to_block(&self, msg: &Message) -> Result<Block>
```

**Behavior**:
1. Accepts any `Message` enum variant
2. Rejects non-`BroadcastBlock` with `InvalidMessageType` error
3. Performs four sequential validation steps:
   - Content hash verification
   - ED25519 signature verification  
   - Parent reference integrity
4. Returns mapped `Block` on success, detailed error on failure

**Example**:
```rust
use cordial_miners_core::network::Message;
use cordial_f1r3node_adapter::grpc_ingest::GrpcBlockMapper;

let mapper = GrpcBlockMapper::new();
let network_msg = Message::BroadcastBlock { block: ... };

match mapper.to_block(&network_msg) {
    Ok(block) => println!("Valid block received: {}", block.identity.creator),
    Err(e) => eprintln!("Rejected: {}", e),
}
```

### `BlocklaceAdapter<Id>` Trait

**Required implementation**:
```rust
pub trait BlocklaceAdapter<Id> {
    fn on_block(&mut self, block: Block) -> Result<()>;
}
```

**Contract**:
- Receives a structurally-valid `Block` (guaranteed by mapper)
- Responsible for semantic validation and side effects
- Returns `Ok(())` on success, `Err` if consensus logic rejects the block

**Example**:
```rust
use cordial_miners_core::Block;
use cordial_miners_core::types::BlockIdentity;
use cordial_f1r3node_adapter::grpc_ingest::BlocklaceAdapter;

struct MyAdapter {
    engine: Box<dyn ConsensusEngine<BlockId = BlockIdentity>>,
    blocklace: Blocklace,
}

impl BlocklaceAdapter<BlockIdentity> for MyAdapter {
    fn on_block(&mut self, block: Block) -> Result<()> {
        // Semantic validation happens here
        self.blocklace.insert(block.clone())?;
        self.engine.on_block(block.identity)?;
        Ok(())
    }
}
```

## Validation Pipeline

### Step 1: Message Type Check
- **Input**: `&Message`
- **Validation**: Must be `Message::BroadcastBlock { block }`
- **Rejection**: Other variants (Ping, Hello, SyncRequest, etc.) immediately fail
- **Error**: `"Invalid message type: expected BroadcastBlock, got {:?}"`

### Step 2: Content Hash Verification
- **Input**: `Block { identity: { content_hash, ... }, content: { payload, predecessors } }`
- **Process**: Recompute `SHA-256(content)` deterministically
  - Payload is hashed with length prefix (8 bytes little-endian)
  - Predecessors are sorted by content_hash for determinism
  - Each predecessor encodes: [content_hash (32B), creator_len, creator, sig_len, signature]
- **Validation**: `identity.content_hash == computed_hash`
- **Rejection**: `"Content hash mismatch: expected {:?}, got {:?}"`

### Step 3: Signature Verification
- **Input**: `identity: { content_hash, creator (32-byte ED25519 public key), signature (64 bytes) }`
- **Process**: `ED25519.verify(content_hash, creator_pubkey, signature)`
- **Preconditions**:
  - Creator key must be exactly 32 bytes
  - Signature must be exactly 64 bytes
- **Validation**: Signature is valid under creator's public key
- **Rejection** reasons:
  - `"Invalid creator public key: expected 32 bytes, got N"`
  - `"Invalid signature: expected 64 bytes, got N"`
  - `"Signature verification failed for creator {:?}"`

### Step 4: Parent Reference Integrity
- **Input**: `content.predecessors: HashSet<BlockIdentity>`
- **Validation**: For each parent:
  - Content hash is 32 bytes (always true for SHA-256, but checked defensively)
  - Signature is non-empty
  - Creator key is exactly 32 bytes
- **Rejection** reasons:
  - `"Parent identity has invalid content hash size: N"`
  - `"Parent identity has empty signature"`
  - `"Parent creator has invalid key size: N"`

**Note**: This step validates **structure only**. Closure axiom (all parents exist in blocklace) and chain axiom (no equivocation) are semantic checks delegated to the adapter.

## Error Handling

All errors are `anyhow::Result<Block>`, providing:
- Detailed error messages for each validation failure
- Clear actionable guidance (e.g., "expected 32 bytes, got N")
- Opportunity for logging, metrics, and debugging

### Error Classes

| Error | Cause | Action |
|-------|-------|--------|
| Invalid message type | Non-BroadcastBlock variant | Drop message, continue |
| Content hash mismatch | Corrupted payload or signature | Drop block, log anomaly |
| Signature verification failed | Invalid signature or wrong creator | Drop block, potential Byzantine peer |
| Invalid key size | Malformed ED25519 key | Drop block, potential peer error |
| Parent integrity violation | Malformed parent reference | Drop block, protocol violation |

## Testing Strategy

**Test Location**: `crates/cordial-f1r3node-adapter/tests/test_grpc_ingest.rs`  
**Test Count**: 26 tests (16 unit tests + 10 integration tests)

### Unit Tests (16 cases)

**Valid block mapping** (3 tests):
- Genesis block (no predecessors) maps to identical Block
- Block with predecessors maps deterministically
- Mapper is stateless and idempotent (different instances, same result)

**Invalid message types** (3 tests):
- Ping message rejected
- Hello message rejected  
- SyncRequest message rejected

**Content hash validation** (1 test):
- Corrupted content hash rejected

**Signature validation** (4 tests):
- Corrupted signature rejected
- Wrong creator key rejected
- Truncated signature rejected
- Truncated creator key rejected

**Parent integrity** (2 tests):
- Empty parent signature rejected
- Malformed parent creator key rejected

**Mock adapter integration** (3 tests):
- Valid block triggers `on_block` callback exactly once
- Multiple valid blocks trigger multiple callbacks in order
- Invalid block never reaches adapter

### Integration Tests (10 cases)

**Full pipeline validation** (1 test):
- Valid block flows network → mapper → adapter successfully

**Message filtering** (1 test):
- Non-BroadcastBlock messages rejected before adapter

**Corruption detection** (1 test):
- Corrupted blocks rejected before adapter

**Multiple block sequencing** (1 test):
- Pipeline handles 5+ blocks in sequence correctly

**Predecessor handling** (2 tests):
- Single-predecessor blocks maintain relationships
- Complex chains (genesis → 3 children) preserve structure

**Adapter failure paths** (1 test):
- Adapter can reject even structurally-valid blocks

**Mapper properties** (2 tests):
- Determinism: different instances produce identical results
- Idempotence: same mapper, same input → same output

**Error clarity** (1 test):
- Error messages are descriptive for each failure type

### Test Execution

Run all gRPC ingestion tests (26 total):
```bash
cargo test -p cordial-f1r3node-adapter --test test_grpc_ingest
```

Run specific test:
```bash
cargo test -p cordial-f1r3node-adapter --test test_grpc_ingest valid_genesis_block_maps_to_block
```

Expected output:
```
running 26 tests
test full_pipeline_valid_block_from_network_to_adapter ... ok
test valid_genesis_block_maps_to_block ... ok
test valid_block_with_predecessors_maps_deterministically ... ok
...
test result: ok. 26 passed; 0 failed
```

## Cryptographic Assumptions

- **Hash function**: SHA-256 (deterministic, collision-resistant)
- **Signature scheme**: ED25519 (unforgeability under chosen-message attack)
- **Key format**: Raw 32-byte public keys (standard ED25519)
- **Signature format**: Raw 64-byte signatures (standard ED25519)

**Implications**:
- A block's identity is immutable once signed
- Blocks cannot be forged without the creator's private key
- Content cannot be modified without invalidating the signature

## Future Extensions

### Type Parameter Usage

The generic parameters `<V, P, Id>` enable future extensibility:

```rust
// Future: custom validator type for weighted voting
pub struct GrpcBlockMapper<V: Validator, P, Id> {
    validators: PhantomData<V>,
    ...
}

impl<V: Validator> GrpcBlockMapper<V, (), ()> {
    pub fn to_block_with_weights(&self, msg: &Message, weights: &[V]) -> Result<Block> {
        // Could validate block weight thresholds here
        ...
    }
}
```

### Blocklace Integration

The mapper works naturally with `Blocklace` for closure axiom enforcement:

```rust
impl BlocklaceAdapter<BlockIdentity> for MyAdapter {
    fn on_block(&mut self, block: Block) -> Result<()> {
        // Mapper guaranteed structural validity; we check semantic rules
        let missing = block.content.predecessors.iter()
            .filter(|pid| !self.blocklace.blocks.contains_key(pid))
            .collect::<Vec<_>>();
        
        if !missing.is_empty() {
            return Err(anyhow!("Closure violation: missing {:?}", missing));
        }
        
        self.blocklace.insert(block)?;
        Ok(())
    }
}
```

### Custom Payloads

In future phases, `P` can represent application-specific payload types:

```rust
#[derive(Serialize, Deserialize)]
pub struct CustomPayload {
    transaction: Vec<u8>,
    timestamp: u64,
}

// Map from network CustomPayload to domain model
impl From<CustomPayload> for AppPayload { ... }
```

## Related Modules

- **`cordial_miners_core::crypto`**: SHA-256 hashing and ED25519 signing/verification
- **`cordial_miners_core::Block`**: The consensus-layer block type
- **`cordial_miners_core::network::Message`**: The network message enum
- **`cordial_miners_core::Blocklace`**: In-memory block store (closure enforcement)
- **`cordial_miners_core::ConsensusEngine`**: Trait for consensus callbacks

## Summary

The gRPC ingestion layer provides a **production-ready translation boundary** between untrusted network input and trusted consensus state:

| Aspect | Guarantee |
|--------|-----------|
| **Correctness** | All structural invariants verified before passing to consensus |
| **Determinism** | Same input always produces same output |
| **Performance** | O(n) in payload + predecessors size, no allocation beyond message parsing |
| **Safety** | Pure mapping phase cannot corrupt consensus state |
| **Debuggability** | Detailed, actionable error messages for every rejection |
| **Extensibility** | Generic type parameters enable future customization |

Blocks that pass this layer are **guaranteed structurally sound** and ready for semantic validation by the consensus engine.
