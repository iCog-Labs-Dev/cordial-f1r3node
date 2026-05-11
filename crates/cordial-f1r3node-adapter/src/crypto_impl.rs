// This CryptoImpl's is teaching the adapter HOW to check if a block's signature for encryption is real.

// The core (cordial-miners-core) already defined the RULE: so in here We must be able to verify a block signature.
// That rule that mentioned in cordial-miners-core is the CryptoVerifier trait.

// This file FOLLOWS that rule using F1R3FLY's actual crypto tools.


// We need to bring tools from core crate (project) like importing
use cordial_miners_core::crypto::CryptoVerifier; // The trait that defines the rule for verifying signatures

use cordial_miners_core::crypto::{
    hash_content,     // turns block content into a 32-byte fingerprint
    Ed25519Scheme,    // Algorithm checker for Ed25519 signatures
    Secp256k1Scheme,  // Algorithm checker for Secp256k1 signatures
    SignatureScheme,  // A  shared interface both checkers follow
};

use cordial_miners_core::types::{BlockContent, NodeId}; // The data types we need to work with

//-------------------------------------------------------------------
// An enum to remember which Algorithm to use
//--------------------------------------------------------------------
// Using enum is like choosing a between provided Algorithms; we store this inside our adapter so it knows which checker to call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CryptoAlgorithm {
    Ed25519,
    Secp256k1,  // the defualt algotihm for F1R3FLY node 
}


//-------------------------------------------------------------------
// The Adapter Struct
//-------------------------------------------------------------------
// This struct (a box in rust)will implement the CryptoVerifier trait, and it will use the chosen algorithm to verify signatures.
#[derive(Debug)]
pub struct F1r3flyCryptoAdapter {
    algorithm: CryptoAlgorithm, // which algorithm to use for verification choosen from the enum above
}

// This implementation in here are the functions that belong to F1r3flyCryptoAdapter struct".
impl F1r3flyCryptoAdapter {
    // Constructor 1: make the adapter that use Secp256k1 by default
    pub fn secp256k1() -> Self {
        Self { algorithm: CryptoAlgorithm::Secp256k1 }
    }
    // Constructor 2: make the adapter that use Ed25519
    pub fn ed25519() -> Self {
        Self { algorithm: CryptoAlgorithm::Ed25519 }
    }
    // Constructor 3: Create adapter from algorithm string ("secp256k1", "ed25519") which is sent by network.
    // Returns Ok(adapter) or Err if the algorithm is unknown.
    pub fn from_algorithm_str(s: &str) -> Result<Self, String> {
        // .to_lowercase() makes the input case-insensitive because of diffrent forms of writing.
        match s.to_lowercase().as_str() {
            // empty string is treated as secp256k1 by default, matching f1r3node's behavior.
            ""          => Ok(Self::secp256k1()),
            "secp256k1" => Ok(Self::secp256k1()),
            "ed25519"   => Ok(Self::ed25519()),
            // anything else is an error
            other       => Err(format!("Unknown algorithm: '{}' — expected 'secp256k1' or 'ed25519'", other)),
        }
    }
    // A simple getter so tests can check which algorithm the adapter is using.
    pub fn algorithm(&self) -> CryptoAlgorithm {
        self.algorithm
    }
}


//--------------------------------------------------------------------
// The Actual Verification Logic
//--------------------------------------------------------------------
// This is where we implement the CryptoVerifier trait for our adapter, which means 
// we have to write the code that checks if a block's signature is valid according to the chosen algorithm.
// The compiler will error if we don't provide verify_block — it's required.
impl CryptoVerifier for F1r3flyCryptoAdapter {
    // The type of error when verification fails. We use String for Human Readable,
    // more structured error handling could be added later if needed.
    type Error = String;

    // Verify block is function blocklace calls on every new block.
    // Parameters: content (block data), signature (proof byte), creator (public key).
    // Returns: Ok(()) if valid, Err(msg) if invalid.
    fn verify_block(
        &self,
        content: &BlockContent,
        signature: &[u8],
        creator: &NodeId,
    ) -> Result<(), Self::Error> {
        // First: recompute the block hash from content with Blake2b-256.
        // The creator signed that hash, so we verify the signature against it.
        // Since we don’t trust any stored hash in the block; recomputing prevents tampering.

        let hash: [u8; 32] = hash_content(content); // Get the 32-byte hash of the block content.

        // Make sure to reject empty signatures right away, as they are invalid.
        if signature.is_empty() {
            return Err("Signature is empty".to_string());
        }

        // Verify the signature with the chosen algorithm.
        // `creator.0` unwraps NodeId to raw public-key bytes.
        // Call the selected scheme’s (algorithms) `verify()` functions and use its result for further processing.
        let is_valid = match self.algorithm {
            CryptoAlgorithm::Secp256k1 => {
                // Verifies a Secp256k1 ECDSA signature over the hash using the public key.
                Secp256k1Scheme.verify(&hash, &creator.0, signature)
            }
            CryptoAlgorithm::Ed25519 =>{
                // Verifies an Ed25519 EdDSA signature over the hash using the public key.
                Ed25519Scheme.verify(&hash, &creator.0, signature)
            }
            
        };
        // Return Ok if valid in the above is true, Err with message if is_valid is false.
        if is_valid {
            Ok(())
        } else {
            Err(format!(
                "Block signature verification failed (public_key={:?}, algorithm={:?})",
                creator.0,
                self.algorithm
            ))
        }

    }
}
    
