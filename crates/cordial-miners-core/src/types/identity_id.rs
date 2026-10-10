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

impl BlockIdentity {
    /// Whether two authenticated identities name the same consensus block.
    ///
    /// Signature bytes are proof material and may differ between otherwise
    /// equivalent identities, so consensus relations compare only the content
    /// hash and creator.
    pub fn same_consensus_identity(&self, other: &Self) -> bool {
        self.content_hash == other.content_hash && self.creator == other.creator
    }

    /// Return the signature-independent identity used by consensus output.
    ///
    /// A signature authenticates a block, but is not a stable semantic name:
    /// a signer can produce more than one valid signature for the same hash.
    /// Transport predecessor references also carry only the content hash and
    /// creator. Consensus comparisons therefore project identities to those
    /// two fields while the full identity remains available as evidence.
    pub fn consensus_identity(&self) -> Self {
        Self {
            content_hash: self.content_hash,
            creator: self.creator.clone(),
            signature: Vec::new(),
        }
    }
}
