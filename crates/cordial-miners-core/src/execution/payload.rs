use crate::types::NodeId;
use bincode::Options;
use serde::{Deserialize, Serialize};

/// Typed block payload for Cordial Miners consensus.
///
/// Maps to f1r3node's `Body` struct but adapted for the blocklace model.
/// This is serialized into `BlockContent.payload: Vec<u8>` via bincode,
/// keeping the blocklace protocol-agnostic while carrying execution data.
///
/// f1r3node equivalent: `Body { state, deploys, rejected_deploys, system_deploys }`
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CordialBlockPayload {
    /// Signed protocol messages authorizing PoR penalties in a specific context.
    /// These are claims; receivers must verify their proofs and authorization.
    pub por_authorizations: Vec<PorEquivocationAuthorization>,

    /// State before and after executing deploys in this block.
    pub state: BlockState,

    /// Successfully processed user deploys.
    pub deploys: Vec<ProcessedDeploy>,

    /// Deploys that were rejected (invalid signature, expired, etc.).
    pub rejected_deploys: Vec<RejectedDeploy>,

    /// System-level deploys (slash equivocators, close block).
    pub system_deploys: Vec<ProcessedSystemDeploy>,
}

/// Context-tagged PoR authorization carried inside the signed block payload.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub struct PorEquivocationAuthorization {
    pub shard_id: Vec<u8>,
    /// Wave for v1 messages; reserved zero for v2 checkpoint intents. The signed
    /// message domain selects the mode, so wave zero is never a mode discriminator.
    pub source_finalized_wave: u64,
    pub round: u64,
    pub message: Vec<u8>,
}

const PAYLOAD_V2: &[u8] = b"cordial:execution-payload:v2\0";
const POR_EXTRA_V1: &[u8] = b"cordial:por-authorizations:v1\0";

/// Block state tracking pre/post execution hashes and validator bonds.
///
/// f1r3node equivalent: `F1r3flyState { pre_state_hash, post_state_hash, bonds, block_number }`
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BlockState {
    /// Hash of the state tuplespace before executing this block's deploys.
    pub pre_state_hash: Vec<u8>,

    /// Hash of the state tuplespace after executing this block's deploys.
    pub post_state_hash: Vec<u8>,

    /// Current validator bonds (stake amounts) after this block.
    pub bonds: Vec<Bond>,

    /// Sequential block number.
    pub block_number: u64,
}

/// A validator's stake bond.
///
/// f1r3node equivalent: `Bond { validator: ByteString, stake: i64 }`
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Bond {
    /// Validator identity (public key bytes).
    pub validator: NodeId,

    /// Stake amount. Using u64 (non-negative) since negative stake is not meaningful.
    pub stake: u64,
}

/// A user deploy -- code to be executed on the tuplespace.
///
/// f1r3node equivalent: `DeployData { term, time_stamp, phlo_price, phlo_limit, ... }`
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Deploy {
    /// The code to execute (e.g., Rholang source).
    pub term: Vec<u8>,

    /// Deployment timestamp (milliseconds since epoch).
    pub timestamp: u64,

    /// Price per unit of computation (phlogiston).
    pub phlo_price: u64,

    /// Maximum computation units to consume.
    pub phlo_limit: u64,

    /// Deploy is only valid after this block number.
    pub valid_after_block_number: u64,

    /// Shard identifier this deploy targets.
    pub shard_id: String,

    /// Optional timestamp after which the deploy is no longer valid.
    ///
    /// This is part of the signed host `DeployData` and must survive adapter
    /// translation unchanged for signature verification to remain valid.
    pub expiration_timestamp: Option<u64>,
}

/// Signature scheme attached to a deploy at its trust boundary.
///
/// Unknown names are retained instead of being guessed so an adapter can
/// reject unsupported algorithms with an accurate diagnostic.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum DeploySignatureAlgorithm {
    Secp256k1,
    Secp256k1Eth,
    Ed25519,
    Other(String),
    #[default]
    Unspecified,
}

impl DeploySignatureAlgorithm {
    pub fn from_name(name: &str) -> Self {
        match name.to_ascii_lowercase().as_str() {
            "secp256k1" => Self::Secp256k1,
            "secp256k1-eth" | "secp256k1:eth" => Self::Secp256k1Eth,
            "ed25519" => Self::Ed25519,
            "" => Self::Unspecified,
            _ => Self::Other(name.to_owned()),
        }
    }

    pub fn as_name(&self) -> &str {
        match self {
            Self::Secp256k1 => "secp256k1",
            // This is the name returned by f1r3node's Secp256k1Eth object.
            Self::Secp256k1Eth => "secp256k1:eth",
            Self::Ed25519 => "ed25519",
            Self::Other(name) => name,
            Self::Unspecified => "",
        }
    }
}

/// A signed deploy -- deploy data with the deployer's signature.
///
/// f1r3node equivalent: `Signed<DeployData> { data, pk, sig, sig_algorithm }`
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SignedDeploy {
    /// The deploy data.
    pub deploy: Deploy,

    /// Deployer's public key.
    pub deployer: Vec<u8>,

    /// Signature over the serialized deploy data.
    pub signature: Vec<u8>,

    /// Algorithm that produced `signature`.
    ///
    /// This field intentionally has no Serde default: legacy serialized
    /// deploys that omitted the algorithm are rejected instead of being
    /// silently treated as a particular signature scheme.
    pub signature_algorithm: DeploySignatureAlgorithm,
}

/// A deploy that has been executed, with its cost and execution log.
///
/// f1r3node equivalent: `ProcessedDeploy { deploy, cost, deploy_log, is_failed, ... }`
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ProcessedDeploy {
    /// The signed deploy that was executed.
    pub deploy: SignedDeploy,

    /// Computation cost consumed during execution.
    pub cost: u64,

    /// Whether the deploy execution failed.
    pub is_failed: bool,
}

/// A deploy that was rejected before execution.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RejectedDeploy {
    /// The signed deploy that was rejected.
    pub deploy: SignedDeploy,

    /// Reason for rejection.
    pub reason: RejectReason,
}

/// Why a deploy was rejected.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum RejectReason {
    /// Deploy signature did not verify.
    InvalidSignature,
    /// Deploy has expired (past block lifespan or timestamp expiration).
    Expired,
    /// Deploy is a duplicate (already in ancestry).
    Duplicate,
    /// Deploy phlo price is below the minimum.
    InsufficientPhloPrice,
    /// Deploy is not yet valid (valid_after_block_number not reached).
    NotYetValid,
}

/// A system-level deploy (slash, close block).
///
/// f1r3node equivalent: `ProcessedSystemDeploy::Succeeded { .. } | Failed { .. }`
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ProcessedSystemDeploy {
    /// Slash an equivocating validator.
    Slash {
        /// The equivocating validator.
        validator: NodeId,
        /// Whether the slash succeeded.
        succeeded: bool,
    },

    /// Close block (finalize state transitions).
    CloseBlock {
        /// Whether the close succeeded.
        succeeded: bool,
    },
}

// ── Serialization helpers ──

impl CordialBlockPayload {
    /// Serialize this payload into bytes for storage in `BlockContent.payload`.
    pub fn to_bytes(&self) -> Vec<u8> {
        // Preserve existing block bytes and identities when no extension is present.
        if self.por_authorizations.is_empty() {
            bincode::serialize(&(
                &self.state,
                &self.deploys,
                &self.rejected_deploys,
                &self.system_deploys,
            ))
            .expect("payload serialization failed")
        } else {
            let mut bytes = PAYLOAD_V2.to_vec();
            bytes.extend(bincode::serialize(self).expect("payload serialization failed"));
            bytes
        }
    }

    /// Deserialize a payload from `BlockContent.payload` bytes.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, String> {
        let options = bincode::DefaultOptions::new()
            .with_fixint_encoding()
            .with_limit(bytes.len() as u64)
            .reject_trailing_bytes();
        if let Some(body) = bytes.strip_prefix(PAYLOAD_V2) {
            options
                .deserialize(body)
                .map_err(|e| format!("payload deserialization failed: {e}"))
        } else {
            let (state, deploys, rejected_deploys, system_deploys) = options
                .deserialize(bytes)
                .map_err(|e| format!("payload deserialization failed: {e}"))?;
            Ok(Self {
                state,
                deploys,
                rejected_deploys,
                system_deploys,
                por_authorizations: vec![],
            })
        }
    }

    /// Read only the explicit versioned carrier. Legacy/opaque payloads cannot
    /// authorize PoR penalties; malformed versioned payloads fail closed.
    pub fn por_authorizations_from_bytes(
        bytes: &[u8],
    ) -> Result<Vec<PorEquivocationAuthorization>, String> {
        if bytes.starts_with(PAYLOAD_V2) {
            Ok(Self::from_bytes(bytes)?.por_authorizations)
        } else {
            Ok(vec![])
        }
    }

    /// Preserve the signed carrier in the host Body.extra_bytes transport slot.
    pub fn encode_por_extra(authorizations: &[PorEquivocationAuthorization]) -> Vec<u8> {
        if authorizations.is_empty() {
            return vec![];
        }
        let mut bytes = POR_EXTRA_V1.to_vec();
        bytes.extend(
            bincode::serialize(authorizations).expect("authorization serialization failed"),
        );
        bytes
    }

    pub fn decode_por_extra(bytes: &[u8]) -> Result<Vec<PorEquivocationAuthorization>, String> {
        let Some(body) = bytes.strip_prefix(POR_EXTRA_V1) else {
            return Ok(vec![]);
        };
        bincode::DefaultOptions::new()
            .with_fixint_encoding()
            .with_limit(body.len() as u64)
            .reject_trailing_bytes()
            .deserialize(body)
            .map_err(|e| format!("invalid PoR authorization carrier: {e}"))
    }
}

impl CordialBlockPayload {
    /// Create an empty genesis payload with no deploys.
    pub fn genesis(bonds: Vec<Bond>) -> Self {
        Self {
            por_authorizations: vec![],
            state: BlockState {
                pre_state_hash: vec![],
                post_state_hash: vec![],
                bonds,
                block_number: 0,
            },
            deploys: vec![],
            rejected_deploys: vec![],
            system_deploys: vec![],
        }
    }

    /// Extract the bonds map as `HashMap<NodeId, u64>` for consensus functions.
    pub fn bonds_map(&self) -> std::collections::HashMap<NodeId, u64> {
        self.state
            .bonds
            .iter()
            .map(|b| (b.validator.clone(), b.stake))
            .collect()
    }
}
