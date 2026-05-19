# 14 — Crypto Adapter: `F1r3flyCryptoAdapter`

**File**: `crates/cordial-f1r3node-adapter/src/crypto_impl.rs`  
**Previous doc**: [13 — gRPC Ingestion Layer](./13-grpc-ingestion.md)

---

## What problem does this solve?

When a block arrives at the node from the network, the node must answer one question:

> "Was this block actually signed by the node who claims to have created it?"

Without this check, anyone could send a fake block pretending to be a validator. The `F1r3flyCryptoAdapter` is the code that answers this question.

---

## How a signature works (Pseudo Steps)

1. A validator has two keys: a **private key** (secret) and a **public key** (shared).
2. When they create a block, they sign it with their private key which produces `signature` bytes.
3. When another node receives the block, it uses the **public key** to check: "do these signature bytes prove the private key was used?"
4. If yes → block is accepted. 
    If no → block is rejected.

The math makes it impossible to produce a valid signature without the private key.

---

## Where this fits in the system

```
Block arrives from network
        ↓
GrpcBlockMapper        ← checks the wire format (is it well-formed?)
        ↓
blocklace.insert()     ← calls CryptoVerifier::verify_block()
        ↓
F1r3flyCryptoAdapter   ← THIS MODULE (is the signature mathematically valid?) or Implements actual crypto logic
        ↓
Block stored in blocklace
```

---

## What algorithms are supported?

| Algorithm | Public key size | Signature size | Default? |
|-----------|----------------|----------------|----------|
| Secp256k1 | 33 or 65 bytes | ~71–72 bytes (DER) | ✅ Yes |
| Ed25519   | 32 bytes       | 64 bytes (fixed)   | No |

The algorithm is chosen per-block based on the `sig_algorithm` field in the network message.

---

## How to use it

```rust
use cordial_f1r3node_adapter::crypto_impl::F1r3flyCryptoAdapter;

// For a secp256k1 block (the default case):
let verifier = F1r3flyCryptoAdapter::secp256k1();
blocklace.insert(block, &verifier)?;

// For an ed25519 block:
let verifier = F1r3flyCryptoAdapter::ed25519();
blocklace.insert(block, &verifier)?;

// Or: parse from the wire string:
let verifier = F1r3flyCryptoAdapter::from_algorithm_str(&block_msg.sig_algorithm)?;
blocklace.insert(block, &verifier)?;
```

---

## Why we recompute the hash inside [crypto_impl.rs](../../crates/cordial-f1r3node-adapter/src/crypto_impl.rs)
Inside `verify_block`, we call `hash_content(content)` ourselves instead of using the `content_hash` stored in the block's identity.

**Reason**: an attacker could put any value in `content_hash`. By recomputing it from the raw `content`, we guarantee we're checking the hash that the original signer actually produced.

---

## Tests

#### You can see the source code for [`test_crypto_adapter.rs`](../../crates/cordial-f1r3node-adapter/tests/test_crypto_adapter.rs)

```bash
cargo test -p cordial-f1r3node-adapter --test test_crypto_bridge
```

| Test | What it checks |
|------|---------------|
| `secp256k1_valid_signature_returns_ok` | Real key + correct signature → `Ok(())` |
| `secp256k1_corrupted_signature_returns_err` | One last byte flipped → `Err` |
| `secp256k1_forged_signature_returns_err` | Signed by wrong key → `Err` |
| `secp256k1_empty_signature_returns_err` | Empty bytes → `Err` |
| `secp256k1_tampered_content_returns_err` | Content changed after signing → `Err` |
| `ed25519_valid_signature_returns_ok` | Real Ed25519 key + correct signature → `Ok(())` |
| `ed25519_corrupted_signature_returns_err` | One byte flipped → `Err` |
| `from_str_secp256k1_gives_secp256k1_adapter` | String parsing works |
| `from_str_ed25519_gives_ed25519_adapter` | String parsing works |
| `from_str_empty_defaults_to_secp256k1` | Empty string = Secp256k1 |
| `from_str_unknown_returns_err` | Unknown algorithm → `Err` |

---

## Related files

- `cordial-miners-core/src/crypto.rs` — defines `CryptoVerifier`, `Secp256k1Scheme`, `Ed25519Scheme`
- `cordial-miners-core/src/blocklace.rs` — calls `verifier.verify_block()` inside `insert()`
- `cordial-f1r3node-adapter/src/crypto_bridge.rs` — separate wire-format crypto (different use)
- `cordial-f1r3node-adapter/src/grpc_ingest.rs` — upstream validation at the network boundary