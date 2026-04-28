/// A node identity — in practice, a public key.
/// From the paper: each node p ∈ Π is identified by its public key.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct NodeId(pub Vec<u8>);
