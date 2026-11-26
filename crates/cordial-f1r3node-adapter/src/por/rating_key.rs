//! Load one validator key for PoR's existing rating-signing path.
//!
//! The derived public key must belong to the node's authorized bonds. The
//! private key is never included in Debug output.

use std::{collections::HashMap, fs, path::Path};

use anyhow::{Context, Result, bail, ensure};
use cordial_miners_core::NodeId;
use k256::ecdsa::SigningKey;

pub struct BondedRatingKey {
    rater: NodeId,
    private_key: [u8; 32],
}

impl BondedRatingKey {
    pub fn rater(&self) -> &NodeId {
        &self.rater
    }

    pub fn private_key_bytes(&self) -> &[u8; 32] {
        &self.private_key
    }
}

/// Load a 32-byte secp256k1 private key encoded as 64 hexadecimal characters.
///
/// The public identity must match exactly one positively bonded validator.
/// Both SEC1 public-key encodings are checked because bonds files may use
/// compressed or uncompressed keys.
pub fn load_bonded_rating_key(
    path: &Path,
    bonds: &HashMap<NodeId, u64>,
) -> Result<BondedRatingKey> {
    let encoded = fs::read_to_string(path)
        .with_context(|| format!("reading validator rating key {}", path.display()))?;
    let encoded = encoded.trim();
    ensure!(
        encoded.len() == 64 && encoded.bytes().all(|byte| byte.is_ascii_hexdigit()),
        "validator rating key must contain exactly 64 hexadecimal characters"
    );
    let decoded = hex::decode(encoded).context("invalid validator rating key encoding")?;
    let private_key: [u8; 32] = decoded
        .try_into()
        .expect("64 hexadecimal characters decode to 32 bytes");
    let signing_key = SigningKey::from_slice(&private_key)
        .map_err(|_| anyhow::anyhow!("invalid secp256k1 validator rating key"))?;

    let verifying_key = signing_key.verifying_key();
    let compressed = NodeId(verifying_key.to_encoded_point(true).as_bytes().to_vec());
    let uncompressed = NodeId(verifying_key.to_encoded_point(false).as_bytes().to_vec());
    let compressed_bonded = bonds.get(&compressed).is_some_and(|weight| *weight > 0);
    let uncompressed_bonded = bonds.get(&uncompressed).is_some_and(|weight| *weight > 0);
    let rater = match (compressed_bonded, uncompressed_bonded) {
        (true, false) => compressed,
        (false, true) => uncompressed,
        (true, true) => bail!("validator rating key matches two bonded key encodings"),
        (false, false) => bail!("validator rating key is not in the authorized bonds"),
    };

    Ok(BondedRatingKey { rater, private_key })
}
