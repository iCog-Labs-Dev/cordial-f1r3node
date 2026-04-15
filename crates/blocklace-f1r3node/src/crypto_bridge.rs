//! Cryptographic alignment (Phase 3.4).
//!
//! f1r3node uses Blake2b-256 for block hashing and Secp256k1 for primary
//! validator signatures. The core blocklace uses SHA-256 + ED25519. This
//! module provides pluggable `Hasher` and `Signer`/`Verifier` traits plus
//! Blake2b/Secp256k1 implementations so the blocklace can interoperate with
//! f1r3node's wire format.
//!
//! Not yet implemented.
