/// Shared benchmark fixtures, excluded unless explicitly enabled.
#[cfg(feature = "bench-support")]
#[doc(hidden)]
pub mod bench_support;

pub mod block;
pub mod blocklace;
pub mod consensus;
pub mod cordiality;
pub mod crypto;
pub mod dag;
pub mod execution;
pub mod finality;
pub mod network;
pub mod simulation;
pub mod trace;
pub mod types;
pub mod wave;

pub use block::Block;
pub use blocklace::Blocklace;
pub use types::{BlockContent, BlockIdentity, NodeId};
