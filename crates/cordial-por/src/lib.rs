//! Proof-of-Reputation scaffolding for Cordial Miners.
//!
//! This crate is the dedicated home for reputation state, reputation-derived
//! weights, and future PoR audit data. Cordial Miners approval, ratification,
//! finality, and tau ordering remain in `cordial-miners-core`.
//!
//! The current crate is intentionally a scaffold. It defines the crate boundary
//! and minimal placeholders only; the reputation data model and algorithms will
//! be added in follow-up issues.

pub mod config;
pub mod error;
pub mod state;
pub mod types;
pub mod weights;

pub use config::PorConfig;
pub use error::PorError;
pub use state::ReputationState;
pub use types::{ReputationEntry, ReputationRound, ReputationWeight};
pub use weights::reputation_weights;
