//! Real RSpace-backed `RuntimeManager` adapter.
//!
//! Delegates `blocklace::execution::RuntimeManager` to f1r3node's real
//! `casper::rust::util::rholang::runtime_manager::RuntimeManager` so blocks
//! produced by the Cordial Miners consensus can execute Rholang deploys
//! against an actual RSpace tuplespace.
//!
//! ## Design: A-lite (caller supplies the RuntimeManager)
//!
//! Constructing a real RSpace requires LMDB storage paths, Rholang
//! interpreter setup, history repository initialization, and bond/genesis
//! bootstrapping — roughly the same setup f1r3node's node binary runs. We
//! don't duplicate that here. Instead the caller (typically f1r3node's
//! node binary, or an integration test harness) constructs a
//! `f1r3node_runtime_manager` and passes a reference when they want to
//! execute a block. Our adapter handles the translation from our
//! `ExecutionRequest` / `ExecutionResult` to f1r3node's types.
//!
//! Currently a stub — this crate is scaffolded but the translation glue
//! is not yet written. First milestone is getting the f1r3node path
//! dependencies to build cleanly in this workspace.
