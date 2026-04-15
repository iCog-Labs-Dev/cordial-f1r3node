use serde::{Serialize, Deserialize};
use crate::types::NodeId;

/// Typed block payload for Cordial Miners consensus.
///
/// Maps to f1r3node's `Body` struct but adapted for the blocklace model.
/// This is serialized into `BlockContent.payload: Vec<u8>` via bincode,
/// keeping the blocklace protocol-agnostic while carrying execution data.
///
/// f1r3node equivalent: `Body { state, deploys, rejected_deploys, system_deploys }`
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CordialBlockPayload {
    /// State before and after executing deploys in this block.
    pub state: BlockState,

    /// Successfully processed user deploys.
    pub deploys: Vec<ProcessedDeploy>,

    /// Deploys that were rejected (invalid signature, expired, etc.).
    pub rejected_deploys: Vec<RejectedDeploy>,

    /// System-level deploys (slash equivocators, close block).
    pub system_deploys: Vec<ProcessedSystemDeploy>,
}

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
        bincode::serialize(self).expect("payload serialization failed")
    }

    /// Deserialize a payload from `BlockContent.payload` bytes.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, String> {
        bincode::deserialize(bytes).map_err(|e| format!("payload deserialization failed: {}", e))
    }
}

impl CordialBlockPayload {
    /// Create an empty genesis payload with no deploys.
    pub fn genesis(bonds: Vec<Bond>) -> Self {
        Self {
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
