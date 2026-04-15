//! `Casper` and `MultiParentCasper` trait adapters (Phase 3.1 / 3.2).
//!
//! Wraps a `Blocklace` plus supporting state and implements f1r3node's
//! `Casper` trait so the rest of f1r3node (engine, proposer, block processor,
//! API) can drive consensus through the Cordial Miners protocol.
//!
//! Depends on block translation ([`super::block_translation`]) and snapshot
//! construction ([`super::snapshot`]).
//!
//! Not yet implemented.
