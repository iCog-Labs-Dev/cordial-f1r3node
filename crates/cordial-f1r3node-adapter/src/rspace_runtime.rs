//! Real RSpace `RuntimeManager` implementation.
//!
//! Implements the `blocklace::execution::RuntimeManager` trait by delegating
//! to f1r3node's RSpace tuplespace and Rholang interpreter. Replaces the
//! `MockRuntime` when running the blocklace inside f1r3node.
//!
//! This is the "real" counterpart to the mock shipped in
//! `blocklace::execution::runtime`. Keeping it in this adapter crate means
//! the core library stays free of RSpace and Rholang dependencies.
//!
//! Not yet implemented.
