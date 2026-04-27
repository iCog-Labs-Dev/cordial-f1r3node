use super::node_id::NodeId;

/// The cryptographic identity of a block: hash(C) signed by its creator.
///
/// From the paper (§2.2):
///   i = signedhash((v, P), k_p)
///
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct BlockIdentity {
    /// SHA-256 (or similar) of the serialized BlockContent.
    pub content_hash: [u8; 32],

    /// The node that signed this hash.
    /// Recoverable from the signature; stored explicitly for convenience.
    /// From the paper: node(i) = p
    pub creator: NodeId,

    /// The signature bytes: sign(content_hash, creator_private_key).
    pub signature: Vec<u8>,
}
