//! # blocklace-f1r3node
//!
//! Integration adapter between the standalone [`blocklace`] crate and
//! f1r3node's consensus layer.
//!
//! This crate lives in its own workspace member so that the core `blocklace`
//! library stays free of f1r3node's RSpace, Rholang, and gRPC dependencies.
//! Consumers who only want the consensus protocol can depend on `blocklace`
//! alone; consumers integrating with f1r3node pull in this crate as well.
//!
//! ## Phase 3 subtask map
//!
//! | Module               | Subtask | Description                                |
//! |----------------------|---------|--------------------------------------------|
//! | [`block_translation`]| 3.5     | `Block` ↔ `BlockMessage` conversions       |
//! | [`casper_adapter`]   | 3.1/3.2 | `Casper` / `MultiParentCasper` impl        |
//! | [`snapshot`]         | 3.3     | `CasperSnapshot` construction              |
//! | [`shard_conf`]       | 3.6     | `CasperShardConf` equivalent               |
//! | [`crypto_bridge`]    | 3.4     | Blake2b + Secp256k1 alignment              |
//! | [`rspace_runtime`]   | 2.3*    | Real `RuntimeManager` impl against RSpace  |

pub mod block_translation;
pub mod casper_adapter;
pub mod crypto_bridge;
pub mod rspace_runtime;
pub mod shard_conf;
pub mod snapshot;
