# 13. gRPC Ingestion Layer: Network → Consensus Translation

**Implementation**: `crates/cordial-f1r3node-adapter/src/grpc_ingest.rs`  
**Module**: `cordial_f1r3node_adapter::grpc_ingest`  
**Tests**: `crates/cordial-f1r3node-adapter/tests/test_grpc_ingest.rs` (23 tests)  
**Purpose**: Safe translation from f1r3node protobuf wire format (`BlockMessage`) to consensus-level blocks

## Architecture Overview

The gRPC ingestion layer implements a **fail-fast, stateless translation pipeline** that sits between the P2P network and the consensus core:

```
Network (f1r3node / P2P)
    ↓
  protobuf deserialization
    ↓
  BlockMessage (wire format)
    ↓
┌─────────────────────────────────────┐
│    GrpcBlockMapper<V, P, Id>        │  (pure, deterministic)
│                                     │
│  Translation:                       │
│  • message_to_block() protobuf →    │
│    internal Block format            │
│  • Extract sig_algorithm field      │
│                                     │
│  Structural validation:             │
│  • Wire block_hash vs recomputed    │
│    Blake2b-256 content hash         │
│  • Secp256k1 / Ed25519 signature    │
│  • Parent creator key byte lengths  │
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
- Wire hash length wrong or mismatch against recomputed → clear expected vs. actual hashes
- Signature verification failure → creator key and algorithm named
- Malformed parent creator key → specific byte length and valid range

### 2. Pure Mapping
The `GrpcBlockMapper` is **stateless and deterministic**:
- No side effects (no database writes, no state mutations)
- Same input → same output (testable, reproducible)
- Can be cloned and shared across threads
- Multiple invocations with identical input produce identical results

### 3. Separation of Concerns
**Mapper** validates structure; **Adapter** handles semantics:
- Mapper: cryptographic signatures, hash integrity, parent key well-formedness
- Adapter: closure axiom, chain axiom, cordial condition, finality rules

### 4. Protobuf-First Input
The mapper accepts `BlockMessage` (f1r3node protobuf wire format) directly — there is no intermediate `Message` enum. Algorithm selection is driven by the `sig_algorithm` field on the wire message, defaulting to `"secp256k1"` if absent.

### 5. Type Genericity
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
pub fn from_protobuf(&self, block_msg: &BlockMessage) -> Result<Block>
```

**Behavior**:
1. Accepts a `BlockMessage` (f1r3node protobuf wire format)
2. Translates to internal `Block` via `message_to_block()`
3. Extracts `sig_algorithm` from the wire message (default: `"secp256k1"`)
4. Performs three sequential structural validation steps:
   - Wire content hash verification (Blake2b-256)
   - Signature verification (algorithm-driven)
   - Parent creator key byte-length check
5. Returns the mapped `Block` on success, a detailed error on failure

**Example**:
```rust
use cordial_f1r3node_adapter::block_translation::BlockMessage;
use cordial_f1r3node_adapter::grpc_ingest::GrpcBlockMapper;

let mapper = GrpcBlockMapper::new();
let block_msg = BlockMessage { sig_algorithm: "secp256k1".into(), /* ... */ };

match mapper.from_protobuf(&block_msg) {
    Ok(block) => println!("Valid block: {:?}", block.identity.creator),
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
        self.blocklace.insert(block.clone())?;
        self.engine.on_block(block.identity)?;
        Ok(())
    }
}
```

## Validation Pipeline

### Step 1: Protobuf Translation
- **Input**: `&BlockMessage` (f1r3node wire format)
- **Process**: `message_to_block(block_msg)` — decodes protobuf fields, merges
  `header.parents_hash_list` and `justifications` into the unified `predecessors` set,
  and populates the internal `Block` struct
- **Also**: `sig_algorithm` is extracted from the wire message and normalised to
  lowercase; an empty field defaults to `"secp256k1"`
- **Rejection**: translation errors (malformed protobuf, missing required fields)

### Step 2: Content Hash Verification
- **Input**: `block_msg.block_hash` (wire) and the translated `Block`
- **Process**: Recompute `Blake2b-256(content)` deterministically from the translated
  `BlockContent`, then check both:
  1. `block_msg.block_hash` (32 bytes) matches the recomputed hash — catches tampering
     of the wire hash field before translation discards it
  2. `block.identity.content_hash` matches the recomputed hash — sanity-checks the
     translation itself
- **Rejection**:
  - `"Content hash mismatch: wire block_hash has invalid length N"`
  - `"Content hash mismatch: wire block_hash [...] does not match recomputed [...]"`
  - `"Content hash mismatch: translated identity [...] does not match recomputed [...]"`

### Step 3: Signature Verification
- **Input**: `identity: { content_hash, creator (public key), signature }`
- **Algorithm**: dispatched from `block_msg.sig_algorithm`:
  - `"secp256k1"` (default): Secp256k1 ECDSA, DER-encoded; key 33 bytes (compressed)
    or 65 bytes (uncompressed)
  - `"ed25519"`: EdDSA; key 32 bytes, signature 64 bytes
- **Preconditions**: signature must be non-empty
- **Validation**: `SignatureScheme.verify(content_hash, creator_key, signature)`
- **Rejection**:
  - `"Signature cannot be empty"`
  - `"Unknown signature algorithm: X (expected 'secp256k1' or 'ed25519')"`
  - `"Signature verification failed for creator [...] using algorithm 'X'"`

### Step 4: Parent Reference Integrity
- **Input**: `content.predecessors: HashSet<BlockIdentity>`
- **Scope**: **Pure byte-level structural check only.** Parent existence is not verified
  here — that is a semantic concern delegated to the adapter (closure axiom).
- **Validation** per predecessor:
  - `content_hash`: statically `[u8; 32]` — no runtime check needed
  - `creator` key length: must be 33 bytes (Secp256k1 compressed) or 65 bytes
    (Secp256k1 uncompressed)
  - `signature`: **intentionally not checked** — wire-format predecessors carry only
    a content hash and creator key; their signatures are legitimately absent
- **Rejection**:
  - `"Parent creator has invalid key length: N bytes (expected 33 or 65)"`

## Error Handling

All errors are `anyhow::Result<Block>`, providing:
- Detailed error messages for each validation failure
- Clear actionable guidance (e.g., "expected 33 or 65, got N")
- Opportunity for logging, metrics, and debugging

### Error Classes

| Error | Cause | Action |
|-------|-------|--------|
| Translation failure | Malformed protobuf / missing fields | Drop message, log error |
| Wire hash wrong length | `block_hash` field not 32 bytes | Drop block, log anomaly |
| Content hash mismatch | Tampered `block_hash` or corrupted payload | Drop block, log anomaly |
| Unknown signature algorithm | `sig_algorithm` not `secp256k1` or `ed25519` | Drop block, check peer version |
| Empty signature | `sig` field absent or zero-length | Drop block, potential peer error |
| Signature verification failed | Invalid signature or wrong creator key | Drop block, potential Byzantine peer |
| Parent key wrong length | Predecessor creator key not 33 or 65 bytes | Drop block, protocol violation |

## Testing Strategy

**Test Location**: `crates/cordial-f1r3node-adapter/tests/test_grpc_ingest.rs`  
**Test Count**: 23 tests (13 unit tests + 10 integration tests)

### Unit Tests (13 cases)

**Valid block mapping** (3 tests):
- `valid_genesis_block_maps_to_block` — no predecessors, hash and creator verified
- `valid_block_with_predecessors_maps_deterministically` — predecessor chain, idempotent output
- `mapper_is_stateless_and_idempotent` — different instances and repeated calls, same result

**Content hash validation** (1 test):
- `block_with_corrupted_content_hash_rejected` — flipped byte in `block_hash` field caught

**Signature validation** (4 tests):
- `block_with_invalid_signature_rejected` — corrupted `sig` bytes
- `block_with_wrong_creator_key_rejected` — mismatched sender key
- `block_with_short_signature_rejected` — truncated signature
- `block_with_short_creator_key_rejected` — truncated creator key

**Parent integrity** (2 tests):
- `block_with_empty_signature_in_parent_rejected` — injected parent reference, verified no panic
- `block_with_malformed_parent_key_rejected` — parent hash too short (3 bytes), verified no panic

**Mock adapter integration** (3 tests):
- `valid_block_triggers_on_block_callback` — callback fires exactly once, hash preserved
- `multiple_valid_blocks_trigger_multiple_callbacks` — two creators, two callbacks in order
- `adapter_fails_to_receive_invalid_block` — corrupted block never reaches adapter

### Integration Tests (10 cases)

**Full pipeline validation** (1 test):
- `full_pipeline_valid_block_from_protobuf_to_adapter` — `BlockMessage` → mapper → adapter end-to-end

**Algorithm rejection** (1 test):
- `pipeline_rejects_non_broadcast_block_messages` — invalid `sig_algorithm` string rejected

**Corruption detection** (1 test):
- `pipeline_rejects_corrupted_blocks_before_adapter` — corrupted signature never reaches adapter

**Multiple block sequencing** (1 test):
- `pipeline_sequence_multiple_valid_blocks` — 5 blocks from distinct creators, all accepted in order

**Predecessor handling** (2 tests):
- `pipeline_with_block_predecessors` — genesis → child, predecessor relationship preserved
- `complex_predecessor_chain` — genesis → 3 chained children, all accepted, structure intact

**Adapter failure paths** (1 test):
- `adapter_can_reject_valid_blocks` — adapter may reject structurally-valid blocks

**Mapper properties** (2 tests):
- `mapper_determinism_across_instances` — two mapper instances, identical output
- `mapper_idempotence_same_instance` — same instance, same input three times, identical output

**Error clarity** (1 test):
- `error_messages_are_descriptive` — corrupted hash and corrupted signature each produce
  descriptive errors containing `"hash mismatch"` / `"Signature"` respectively

### Test Execution

Run all gRPC ingestion tests:
```bash
cargo test -p cordial-f1r3node-adapter --test test_grpc_ingest
```

Run a specific test:
```bash
cargo test -p cordial-f1r3node-adapter --test test_grpc_ingest valid_genesis_block_maps_to_block
```

Expected output:
```
running 23 tests
test valid_genesis_block_maps_to_block ... ok
test full_pipeline_valid_block_from_protobuf_to_adapter ... ok
...
test result: ok. 23 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
```

## Cryptographic Assumptions

- **Hash function**: Blake2b-256 (f1r3node alignment; deterministic, collision-resistant)
- **Signature schemes** (selected per-message via `sig_algorithm`):
  - `"secp256k1"` (default): Secp256k1 ECDSA, DER-encoded signatures
    - Public key: 33 bytes compressed or 65 bytes uncompressed
    - Signature: variable-length DER (typically 71–72 bytes)
  - `"ed25519"`: EdDSA
    - Public key: 32 bytes
    - Signature: 64 bytes

**Implications**:
- A block's identity is immutable once signed
- Blocks cannot be forged without the creator's private key
- Content cannot be modified without invalidating both the hash and the signature
- Algorithm agility is fully wire-driven: no hard-coded scheme at the mapper level

## Future Extensions

### Type Parameter Usage

The generic parameters `<V, P, Id>` enable future extensibility:

```rust
// Future: custom validator type for weighted voting
pub struct GrpcBlockMapper<V: Validator, P, Id> {
    validators: PhantomData<V>,
    // ...
}

impl<V: Validator> GrpcBlockMapper<V, (), ()> {
    pub fn from_protobuf_with_weights(&self, msg: &BlockMessage, weights: &[V]) -> Result<Block> {
        // Validate block weight thresholds here
    }
}
```

### Blocklace Integration

The mapper works naturally with `Blocklace` for closure axiom enforcement in the adapter:

```rust
impl BlocklaceAdapter<BlockIdentity> for MyAdapter {
    fn on_block(&mut self, block: Block) -> Result<()> {
        // Mapper guaranteed structural validity; adapter checks semantic rules
        let missing: Vec<_> = block.content.predecessors.iter()
            .filter(|pid| !self.blocklace.blocks.contains_key(pid))
            .collect();

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

impl From<CustomPayload> for AppPayload { /* ... */ }
```

## Related Modules

- **`cordial_f1r3node_adapter::block_translation`**: `BlockMessage` protobuf type and `message_to_block()` translator
- **`cordial_miners_core::crypto`**: Blake2b-256 hashing; `Secp256k1Scheme` and `Ed25519Scheme` verifiers
- **`cordial_miners_core::Block`**: The consensus-layer block type
- **`cordial_miners_core::Blocklace`**: In-memory block store (closure enforcement in adapter)
- **`cordial_miners_core::ConsensusEngine`**: Trait for consensus callbacks

## Summary

The gRPC ingestion layer provides a **translation boundary** between untrusted network input and trusted consensus state:

| Aspect | Status |
|--------|--------|
| **Correctness** | All structural invariants verified before passing to consensus |
| **Determinism** | Same input always produces same output |
| **Performance** | O(n) in payload + predecessors size, no allocation beyond protobuf parsing |
| **Safety** | Pure mapping phase cannot corrupt consensus state |
| **Algorithm agility** | Signature scheme selected per-message from wire format |
| **Debuggability** | Detailed, actionable error messages for every rejection |
| **Extensibility** | Generic type parameters enable future customization |

Blocks that pass this layer have passed structural validation (hash integrity, signature verification, parent key byte lengths) and are ready for semantic validation by the consensus engine.