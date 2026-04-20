//! Block format translation between the blocklace and f1r3node (Phase 3.5).
//!
//! The blocklace uses a single unified `predecessors: HashSet<BlockIdentity>`
//! per block. f1r3node splits the same information into two fields on
//! [`BlockMessage`]:
//!
//! - [`Header::parents_hash_list`] — explicit parents, bounded by
//!   `max_number_of_parents`
//! - [`BlockMessage::justifications`] — validator → latest block hash map
//!   used for fork choice
//!
//! In Cordial Miners both roles are served by the predecessor set. Our
//! translation:
//!
//! - **blocklace → f1r3node**: pack all predecessors into `parents_hash_list`
//!   and derive `justifications` as `(predecessor.creator, predecessor_hash)`
//!   pairs. Some justifications may be redundant with parents — that's
//!   expected and matches how f1r3node treats them during fork choice.
//!
//! - **f1r3node → blocklace**: take the union of `parents_hash_list` and
//!   `justifications` hashes as the predecessor set. Any `BlockMessage`
//!   that passed f1r3node's validation has its chain axiom satisfied as long
//!   as the justifications match the senders.
//!
//! ## Mirror structs
//!
//! To keep this crate standalone-buildable we mirror f1r3node's types rather
//! than depending on the `models` crate. The structs below match the shapes
//! in `f1r3node/models/src/rust/casper/protocol/casper_message.rs`. When we
//! uncomment the `models` path dependency later, we'll swap these mirrors
//! for `use models::...` imports without changing the translation API.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use blocklace::block::Block;
use blocklace::crypto::hash_content;
use blocklace::execution::{
    Bond as CmBond, CordialBlockPayload, Deploy as CmDeploy, ProcessedDeploy as CmProcessedDeploy,
    ProcessedSystemDeploy as CmSystemDeploy, RejectedDeploy as CmRejectedDeploy,
    RejectReason as CmRejectReason, SignedDeploy as CmSignedDeploy,
};
use blocklace::types::{BlockContent, BlockIdentity, NodeId};

// ═══════════════════════════════════════════════════════════════════════════
// Mirror of f1r3node wire types
// ═══════════════════════════════════════════════════════════════════════════
//
// Field names and types match f1r3node/models/src/rust/casper/protocol/
// casper_message.rs. When we switch to depending on the real `models` crate
// we'll delete these mirrors.

/// f1r3node uses `prost::bytes::Bytes` (aliased as `ByteString`). We use
/// `Vec<u8>` for portability; conversion is cheap either way.
pub type ByteString = Vec<u8>;

/// Mirror of f1r3node's `BlockMessage`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BlockMessage {
    pub block_hash: ByteString,
    pub header: Header,
    pub body: Body,
    pub justifications: Vec<Justification>,
    pub sender: ByteString,
    pub seq_num: i32,
    pub sig: ByteString,
    pub sig_algorithm: String,
    pub shard_id: String,
    pub extra_bytes: ByteString,
}

/// Mirror of f1r3node's `Header`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Header {
    pub parents_hash_list: Vec<ByteString>,
    pub timestamp: i64,
    pub version: i64,
    pub extra_bytes: ByteString,
}

/// Mirror of f1r3node's `Body`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Body {
    pub state: F1r3flyState,
    pub deploys: Vec<ProcessedDeploy>,
    pub rejected_deploys: Vec<RejectedDeploy>,
    pub system_deploys: Vec<ProcessedSystemDeploy>,
    pub extra_bytes: ByteString,
}

/// Mirror of f1r3node's `F1r3flyState`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct F1r3flyState {
    pub pre_state_hash: ByteString,
    pub post_state_hash: ByteString,
    pub bonds: Vec<Bond>,
    pub block_number: i64,
}

/// Mirror of f1r3node's `Bond`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Bond {
    pub validator: ByteString,
    pub stake: i64,
}

/// Mirror of f1r3node's `Justification`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Justification {
    pub validator: ByteString,
    pub latest_block_hash: ByteString,
}

/// Mirror of f1r3node's `DeployData`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DeployData {
    pub term: String,
    pub time_stamp: i64,
    pub phlo_price: i64,
    pub phlo_limit: i64,
    pub valid_after_block_number: i64,
    pub shard_id: String,
    pub expiration_timestamp: Option<i64>,
}

/// Mirror of f1r3node's `Signed<DeployData>`.
///
/// The real type is generic (`Signed<A>`) but for translation we only deal
/// with `Signed<DeployData>`. `sig_algorithm` is a string here rather than
/// `Box<dyn SignaturesAlg>` to keep the mirror plain-data.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SignedDeployData {
    pub data: DeployData,
    pub pk: ByteString,
    pub sig: ByteString,
    pub sig_algorithm: String,
}

/// Mirror of f1r3node's `ProcessedDeploy`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProcessedDeploy {
    pub deploy: SignedDeployData,
    /// f1r3node uses `PCost { cost: i64 }`. We inline the cost as i64.
    pub cost: i64,
    /// f1r3node uses `Vec<Event>` — we represent as opaque bytes since the
    /// blocklace doesn't model execution events.
    pub deploy_log: Vec<u8>,
    pub is_failed: bool,
    pub system_deploy_error: Option<String>,
}

/// Mirror of f1r3node's `RejectedDeploy { sig: ByteString }`. The reason
/// for rejection is not encoded at the wire level — we retain the blocklace's
/// richer reason separately and drop it when translating to f1r3node.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RejectedDeploy {
    pub sig: ByteString,
}

/// Mirror of f1r3node's `ProcessedSystemDeploy` (simplified).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ProcessedSystemDeploy {
    Slash { validator: ByteString, succeeded: bool },
    CloseBlock { succeeded: bool },
}

// ═══════════════════════════════════════════════════════════════════════════
// Translation API
// ═══════════════════════════════════════════════════════════════════════════

/// Errors from translating between the two formats.
#[derive(Debug, Clone, PartialEq)]
pub enum TranslationError {
    /// The block's payload bytes failed to deserialize as `CordialBlockPayload`.
    PayloadDecodeFailed(String),

    /// A numeric field overflowed when casting between i64 (f1r3node) and u64 (blocklace).
    NumericOverflow(&'static str),

    /// A predecessor id could not be reconstructed from the wire hash (length mismatch).
    InvalidPredecessorHash { expected_len: usize, got: usize },
}

// ──────────────────────────────────────────────────────────────────────
// blocklace → f1r3node
// ──────────────────────────────────────────────────────────────────────

/// Translate a blocklace [`Block`] into an f1r3node [`BlockMessage`].
///
/// The block's `payload` bytes are decoded as [`CordialBlockPayload`] to
/// populate the `Body`. Predecessors are packed into both `parents_hash_list`
/// and `justifications` (validator → latest block hash pairs). The f1r3node
/// `block_hash` is set to the blocklace's 32-byte content hash.
///
/// Note: the blocklace's `signature` (ED25519, 64 bytes) is carried verbatim
/// in the `sig` field. Real wire compatibility requires the Phase 3.4 crypto
/// bridge to produce Blake2b + Secp256k1 signatures instead — this function
/// just translates the data shape.
pub fn block_to_message(block: &Block, shard_id: &str) -> Result<BlockMessage, TranslationError> {
    let payload = CordialBlockPayload::from_bytes(&block.content.payload)
        .map_err(TranslationError::PayloadDecodeFailed)?;

    // Predecessors → parents_hash_list (sorted for deterministic ordering)
    let mut parents: Vec<ByteString> = block
        .content
        .predecessors
        .iter()
        .map(|id| id.content_hash.to_vec())
        .collect();
    parents.sort();

    // Predecessors → justifications: one entry per (creator, hash) pair.
    // If a creator has multiple blocks in the predecessor set (which means the
    // creator equivocated), all of them appear. f1r3node's validator may then
    // flag the equivocation.
    let mut justifications: Vec<Justification> = block
        .content
        .predecessors
        .iter()
        .map(|id| Justification {
            validator: id.creator.0.clone(),
            latest_block_hash: id.content_hash.to_vec(),
        })
        .collect();
    justifications.sort_by(|a, b| {
        a.validator
            .cmp(&b.validator)
            .then_with(|| a.latest_block_hash.cmp(&b.latest_block_hash))
    });

    let header = Header {
        parents_hash_list: parents,
        timestamp: 0, // blocklace doesn't carry timestamps at the block level
        version: 1,
        extra_bytes: vec![],
    };

    let body = Body {
        state: F1r3flyState {
            pre_state_hash: payload.state.pre_state_hash,
            post_state_hash: payload.state.post_state_hash,
            bonds: payload
                .state
                .bonds
                .into_iter()
                .map(bond_to_f1r3node)
                .collect(),
            block_number: u64_to_i64(payload.state.block_number, "block_number")?,
        },
        deploys: payload
            .deploys
            .into_iter()
            .map(processed_deploy_to_f1r3node)
            .collect::<Result<Vec<_>, _>>()?,
        rejected_deploys: payload
            .rejected_deploys
            .into_iter()
            .map(rejected_deploy_to_f1r3node)
            .collect(),
        system_deploys: payload
            .system_deploys
            .into_iter()
            .map(system_deploy_to_f1r3node)
            .collect(),
        extra_bytes: vec![],
    };

    Ok(BlockMessage {
        block_hash: block.identity.content_hash.to_vec(),
        header,
        body,
        justifications,
        sender: block.identity.creator.0.clone(),
        seq_num: 0, // blocklace doesn't maintain a sequence number; caller may set later
        sig: block.identity.signature.clone(),
        sig_algorithm: "ed25519".to_string(),
        shard_id: shard_id.to_string(),
        extra_bytes: vec![],
    })
}

// ──────────────────────────────────────────────────────────────────────
// f1r3node → blocklace
// ──────────────────────────────────────────────────────────────────────

/// Translate an f1r3node [`BlockMessage`] into a blocklace [`Block`].
///
/// Predecessors are the union of `header.parents_hash_list` and
/// `justifications.latest_block_hash` — any hash appearing in either list
/// becomes a predecessor. Each predecessor identity needs a creator; we
/// look it up via the `justifications` mapping and fall back to the block's
/// `sender` if a parent hash is not in the justifications list.
///
/// The `Body` is packed back into a [`CordialBlockPayload`] and serialized
/// into `BlockContent.payload`.
pub fn message_to_block(msg: &BlockMessage) -> Result<Block, TranslationError> {
    // Build a map hash -> creator from justifications
    let mut hash_to_creator: std::collections::HashMap<&[u8], &[u8]> =
        std::collections::HashMap::new();
    for j in &msg.justifications {
        hash_to_creator.insert(&j.latest_block_hash, &j.validator);
    }

    // Union of parents and justification hashes, deduped
    let mut pred_hashes: HashSet<Vec<u8>> = HashSet::new();
    for p in &msg.header.parents_hash_list {
        pred_hashes.insert(p.clone());
    }
    for j in &msg.justifications {
        pred_hashes.insert(j.latest_block_hash.clone());
    }

    let mut predecessors = HashSet::new();
    for hash in pred_hashes {
        let content_hash: [u8; 32] = hash.as_slice().try_into().map_err(|_| {
            TranslationError::InvalidPredecessorHash {
                expected_len: 32,
                got: hash.len(),
            }
        })?;
        let creator_bytes = hash_to_creator
            .get(hash.as_slice())
            .copied()
            .unwrap_or(msg.sender.as_slice());
        predecessors.insert(BlockIdentity {
            content_hash,
            creator: NodeId(creator_bytes.to_vec()),
            // The f1r3node wire format doesn't carry the per-predecessor
            // signature. Leave empty; downstream verification can re-fetch
            // the referenced block and read its signature from there.
            signature: vec![],
        });
    }

    // Rebuild the payload from Body
    let payload = CordialBlockPayload {
        state: blocklace::execution::BlockState {
            pre_state_hash: msg.body.state.pre_state_hash.clone(),
            post_state_hash: msg.body.state.post_state_hash.clone(),
            bonds: msg
                .body
                .state
                .bonds
                .iter()
                .map(bond_from_f1r3node)
                .collect::<Result<Vec<_>, _>>()?,
            block_number: i64_to_u64(msg.body.state.block_number, "block_number")?,
        },
        deploys: msg
            .body
            .deploys
            .iter()
            .map(processed_deploy_from_f1r3node)
            .collect::<Result<Vec<_>, _>>()?,
        rejected_deploys: msg
            .body
            .rejected_deploys
            .iter()
            .map(rejected_deploy_from_f1r3node)
            .collect(),
        system_deploys: msg
            .body
            .system_deploys
            .iter()
            .map(system_deploy_from_f1r3node)
            .collect(),
    };

    let content = BlockContent {
        payload: payload.to_bytes(),
        predecessors,
    };

    // The block's content_hash is recomputed so the blocklace structure
    // stays internally consistent. f1r3node's `block_hash` is advisory and
    // discarded — the hash we compute is over the translated content.
    let content_hash = hash_content(&content);

    Ok(Block {
        identity: BlockIdentity {
            content_hash,
            creator: NodeId(msg.sender.clone()),
            signature: msg.sig.clone(),
        },
        content,
    })
}

// ═══════════════════════════════════════════════════════════════════════════
// Helpers: payload-level translation
// ═══════════════════════════════════════════════════════════════════════════

fn bond_to_f1r3node(b: CmBond) -> Bond {
    Bond {
        validator: b.validator.0,
        stake: b.stake as i64,
    }
}

fn bond_from_f1r3node(b: &Bond) -> Result<CmBond, TranslationError> {
    Ok(CmBond {
        validator: NodeId(b.validator.clone()),
        stake: i64_to_u64(b.stake, "bond.stake")?,
    })
}

fn deploy_to_f1r3node(d: CmDeploy) -> Result<DeployData, TranslationError> {
    Ok(DeployData {
        term: String::from_utf8_lossy(&d.term).into_owned(),
        time_stamp: u64_to_i64(d.timestamp, "deploy.timestamp")?,
        phlo_price: u64_to_i64(d.phlo_price, "deploy.phlo_price")?,
        phlo_limit: u64_to_i64(d.phlo_limit, "deploy.phlo_limit")?,
        valid_after_block_number: u64_to_i64(
            d.valid_after_block_number,
            "deploy.valid_after_block_number",
        )?,
        shard_id: d.shard_id,
        expiration_timestamp: None, // blocklace Deploy doesn't carry this yet
    })
}

fn deploy_from_f1r3node(d: &DeployData) -> Result<CmDeploy, TranslationError> {
    Ok(CmDeploy {
        term: d.term.as_bytes().to_vec(),
        timestamp: i64_to_u64(d.time_stamp, "deploy.time_stamp")?,
        phlo_price: i64_to_u64(d.phlo_price, "deploy.phlo_price")?,
        phlo_limit: i64_to_u64(d.phlo_limit, "deploy.phlo_limit")?,
        valid_after_block_number: i64_to_u64(
            d.valid_after_block_number,
            "deploy.valid_after_block_number",
        )?,
        shard_id: d.shard_id.clone(),
    })
}

fn signed_deploy_to_f1r3node(sd: CmSignedDeploy) -> Result<SignedDeployData, TranslationError> {
    Ok(SignedDeployData {
        data: deploy_to_f1r3node(sd.deploy)?,
        pk: sd.deployer,
        sig: sd.signature,
        sig_algorithm: "ed25519".to_string(),
    })
}

fn signed_deploy_from_f1r3node(sd: &SignedDeployData) -> Result<CmSignedDeploy, TranslationError> {
    Ok(CmSignedDeploy {
        deploy: deploy_from_f1r3node(&sd.data)?,
        deployer: sd.pk.clone(),
        signature: sd.sig.clone(),
    })
}

fn processed_deploy_to_f1r3node(
    pd: CmProcessedDeploy,
) -> Result<ProcessedDeploy, TranslationError> {
    Ok(ProcessedDeploy {
        deploy: signed_deploy_to_f1r3node(pd.deploy)?,
        cost: u64_to_i64(pd.cost, "processed_deploy.cost")?,
        deploy_log: vec![], // blocklace doesn't model execution events
        is_failed: pd.is_failed,
        system_deploy_error: None,
    })
}

fn processed_deploy_from_f1r3node(
    pd: &ProcessedDeploy,
) -> Result<CmProcessedDeploy, TranslationError> {
    Ok(CmProcessedDeploy {
        deploy: signed_deploy_from_f1r3node(&pd.deploy)?,
        cost: i64_to_u64(pd.cost, "processed_deploy.cost")?,
        is_failed: pd.is_failed,
    })
}

fn rejected_deploy_to_f1r3node(rd: CmRejectedDeploy) -> RejectedDeploy {
    // The reason is dropped — f1r3node's wire RejectedDeploy only carries the
    // signature. Consumers who need the reason must track it separately.
    RejectedDeploy {
        sig: rd.deploy.signature,
    }
}

fn rejected_deploy_from_f1r3node(rd: &RejectedDeploy) -> CmRejectedDeploy {
    // We don't know the reason from the wire — default to InvalidSignature.
    // We also don't have the full deploy; reconstruct a minimal SignedDeploy
    // with the signature and empty fields. The caller is expected to correlate
    // with the deploy pool if richer data is needed.
    CmRejectedDeploy {
        deploy: CmSignedDeploy {
            deploy: CmDeploy {
                term: vec![],
                timestamp: 0,
                phlo_price: 0,
                phlo_limit: 0,
                valid_after_block_number: 0,
                shard_id: String::new(),
            },
            deployer: vec![],
            signature: rd.sig.clone(),
        },
        reason: CmRejectReason::InvalidSignature,
    }
}

fn system_deploy_to_f1r3node(sd: CmSystemDeploy) -> ProcessedSystemDeploy {
    match sd {
        CmSystemDeploy::Slash { validator, succeeded } => ProcessedSystemDeploy::Slash {
            validator: validator.0,
            succeeded,
        },
        CmSystemDeploy::CloseBlock { succeeded } => {
            ProcessedSystemDeploy::CloseBlock { succeeded }
        }
    }
}

fn system_deploy_from_f1r3node(sd: &ProcessedSystemDeploy) -> CmSystemDeploy {
    match sd {
        ProcessedSystemDeploy::Slash { validator, succeeded } => CmSystemDeploy::Slash {
            validator: NodeId(validator.clone()),
            succeeded: *succeeded,
        },
        ProcessedSystemDeploy::CloseBlock { succeeded } => CmSystemDeploy::CloseBlock {
            succeeded: *succeeded,
        },
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Numeric conversion helpers
// ═══════════════════════════════════════════════════════════════════════════

fn u64_to_i64(v: u64, field: &'static str) -> Result<i64, TranslationError> {
    i64::try_from(v).map_err(|_| TranslationError::NumericOverflow(field))
}

fn i64_to_u64(v: i64, field: &'static str) -> Result<u64, TranslationError> {
    u64::try_from(v).map_err(|_| TranslationError::NumericOverflow(field))
}
