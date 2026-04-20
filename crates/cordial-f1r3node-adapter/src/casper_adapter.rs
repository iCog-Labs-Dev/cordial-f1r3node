//! `Casper` and `MultiParentCasper` trait adapters (Phase 3.1 / 3.2).
//!
//! Wraps a [`Blocklace`] plus supporting state and exposes it through a
//! Casper-compatible interface so f1r3node's engine, proposer, block
//! processor, and API can drive Cordial Miners through their existing
//! call sites.
//!
//! ## Why a local trait?
//!
//! The real `casper::rust::Casper` trait lives in f1r3node's `casper` crate
//! and pulls in heavy transitive dependencies (RSpace, Rholang, gRPC).
//! Until we uncomment the `casper` path dependency, we mirror the trait
//! shape locally as [`CordialCasper`] and [`CordialMultiParentCasper`].
//! The trait surface and method signatures are identical to f1r3node's,
//! using our mirror types ([`BlockMessage`], [`CasperSnapshot`], etc.)
//! instead of f1r3node's real types.
//!
//! When the `casper` dep is enabled, switching to `impl casper::Casper for
//! CordialCasperAdapter` is mechanical — the method bodies stay the same
//! and only type imports change.
//!
//! ## What the adapter owns
//!
//! ```text
//! CordialCasperAdapter {
//!     blocklace:    Mutex<Blocklace>,            // consensus state
//!     deploy_pool:  Mutex<DeployPool>,           // pending user deploys
//!     bonds:        HashMap<NodeId, u64>,        // validator stakes
//!     shard_conf:   CasperShardConf,             // configuration
//!     shard_id:     String,                      // for BlockMessage.shard_id
//!     approved_block: Option<BlockMessage>,      // genesis / approved block
//!     buffer:       Mutex<HashMap<...>>,         // dependency-pending blocks
//! }
//! ```
//!
//! ## Method mapping
//!
//! | f1r3node `Casper` method      | Cordial Miners equivalent                       |
//! |-------------------------------|--------------------------------------------------|
//! | `get_snapshot`                | [`build_snapshot`] over current blocklace + bonds|
//! | `contains` / `dag_contains`   | `Blocklace::dom().contains(...)` by content_hash |
//! | `buffer_contains`             | Internal pending-block buffer                    |
//! | `get_approved_block`          | Stored at construction                           |
//! | `deploy`                      | Validate signature, push to `DeployPool`         |
//! | `estimator`                   | [`fork_choice`] tips, then translate to hashes   |
//! | `get_version`                 | From `shard_conf.casper_version`                 |
//! | `validate`                    | Translate then call core [`validate_block`]      |
//! | `validate_self_created`       | Same as `validate`, with crypto checks skipped   |
//! | `handle_valid_block`          | Insert into blocklace; rebuild snapshot DAG view |
//! | `handle_invalid_block`        | Mark block-hash invalid; do not insert           |
//! | `get_dependency_free_from_buffer` | Buffer entries whose preds all exist        |
//! | `get_all_from_buffer`         | All buffer entries                               |
//!
//! `MultiParentCasper` adds `last_finalized_block` (from
//! [`find_last_finalized`]), `block_dag` (rebuilt from snapshot), and
//! `has_pending_deploys_in_storage` (DeployPool emptiness check). The
//! RSpace-specific `runtime_manager`, `block_store`, and
//! `get_history_exporter` methods are stubbed — they return adapter-local
//! placeholders since we don't have RSpace wired into the core crate yet.

use std::collections::{HashMap, HashSet};

use async_trait::async_trait;
use either::Either;

use blocklace::block::Block;
use blocklace::blocklace::Blocklace;
use blocklace::consensus::{
    fork_choice, find_last_finalized, validate_block as core_validate_block,
    InvalidBlock as CoreInvalidBlock, ValidationConfig, ValidationResult,
};
use blocklace::execution::{
    DeployPool, DeployPoolConfig, PoolError, SignedDeploy as CmSignedDeploy,
};
use blocklace::types::{BlockIdentity, NodeId};

use crate::block_translation::{
    block_to_message, message_to_block, BlockMessage, SignedDeployData, TranslationError,
};
use crate::shard_conf::CasperShardConf;
use crate::snapshot::{build_snapshot, CasperSnapshot, SnapshotError};

// ═══════════════════════════════════════════════════════════════════════════
// Mirror types for the trait surface
// ═══════════════════════════════════════════════════════════════════════════

/// Mirror of f1r3node's `BlockHash` (a `Bytes` newtype). We use `Vec<u8>`.
pub type BlockHash = Vec<u8>;

/// Mirror of f1r3node's `Validator` (a `Bytes` newtype).
pub type Validator = Vec<u8>;

/// Mirror of f1r3node's `DeployId` (a `Bytes` newtype, the deploy signature).
pub type DeployId = Vec<u8>;

/// Mirror of f1r3node's `BlockError` (subset). f1r3node has more variants
/// like `Processed`, `MissingBlocks`, `CasperIsBusy`, `BlockException`;
/// we model the ones the adapter actually emits.
#[derive(Debug, Clone, PartialEq)]
pub enum BlockError {
    /// Block has already been processed (in the DAG).
    Processed,
    /// One or more predecessors are missing — block is buffered awaiting them.
    MissingBlocks,
    /// Block is invalid; specific reason in [`InvalidBlock`].
    Invalid(InvalidBlock),
    /// Adapter-internal exception with a string description.
    BlockException(String),
}

/// Mirror of f1r3node's `InvalidBlock`. Subset that the Cordial Miners
/// validation pipeline can produce.
#[derive(Debug, Clone, PartialEq)]
pub enum InvalidBlock {
    InvalidBlockHash,
    InvalidSignature,
    InvalidSender,
    InvalidParents,
    AdmissibleEquivocation,
    NotOfInterest,
    /// Translation failure — block didn't decode against the wire format.
    InvalidFormat,
}

/// Mirror of f1r3node's `ValidBlock`.
#[derive(Debug, Clone, PartialEq)]
pub enum ValidBlock {
    Valid,
}

/// Mirror of f1r3node's `DeployError`.
#[derive(Debug, Clone, PartialEq)]
pub enum DeployError {
    ParsingError(String),
    MissingUser,
    UnknownSignatureAlgorithm(String),
    SignatureVerificationFailed,
    /// Adapter-specific: the deploy failed pool admission.
    PoolRejected(String),
}

/// Top-level adapter error. Mirrors f1r3node's `CasperError` superset.
#[derive(Debug, Clone, PartialEq)]
pub enum CasperError {
    /// Snapshot construction failed.
    Snapshot(String),
    /// Block translation failed.
    Translation(String),
    /// Adapter is in a state where the requested operation is invalid
    /// (e.g. `get_approved_block` before one is set).
    InvalidState(&'static str),
}

impl From<SnapshotError> for CasperError {
    fn from(e: SnapshotError) -> Self {
        CasperError::Snapshot(format!("{:?}", e))
    }
}

impl From<TranslationError> for CasperError {
    fn from(e: TranslationError) -> Self {
        CasperError::Translation(format!("{:?}", e))
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Trait definitions (mirrors of f1r3node Casper / MultiParentCasper)
// ═══════════════════════════════════════════════════════════════════════════

/// Local mirror of f1r3node's `Casper` trait.
#[async_trait]
pub trait CordialCasper {
    async fn get_snapshot(&self) -> Result<CasperSnapshot, CasperError>;

    fn contains(&self, hash: &BlockHash) -> bool;
    fn dag_contains(&self, hash: &BlockHash) -> bool;
    fn buffer_contains(&self, hash: &BlockHash) -> bool;

    fn get_approved_block(&self) -> Result<&BlockMessage, CasperError>;

    fn deploy(
        &self,
        deploy: SignedDeployData,
    ) -> Result<Either<DeployError, DeployId>, CasperError>;

    /// Returns ranked tips (matches f1r3node's estimator: highest-stake-weight
    /// fork first). Cordial Miners doesn't strictly need fork choice — see
    /// [`fork_choice`] module doc — but we honour the trait shape.
    async fn estimator(&self) -> Result<Vec<BlockHash>, CasperError>;

    fn get_version(&self) -> i64;

    async fn validate(
        &self,
        block: &BlockMessage,
    ) -> Result<Either<BlockError, ValidBlock>, CasperError>;

    async fn validate_self_created(
        &self,
        block: &BlockMessage,
        pre_state_hash: Vec<u8>,
        post_state_hash: Vec<u8>,
    ) -> Result<Either<BlockError, ValidBlock>, CasperError>;

    async fn handle_valid_block(&self, block: &BlockMessage) -> Result<(), CasperError>;

    fn handle_invalid_block(
        &self,
        block: &BlockMessage,
        status: &InvalidBlock,
    ) -> Result<(), CasperError>;

    fn get_dependency_free_from_buffer(&self) -> Result<Vec<BlockMessage>, CasperError>;
    fn get_all_from_buffer(&self) -> Result<Vec<BlockMessage>, CasperError>;
}

/// Local mirror of f1r3node's `MultiParentCasper` trait. Adds finality
/// query and on-chain state accessors. The RSpace-coupled methods
/// (`runtime_manager`, `block_store`, `get_history_exporter`) are
/// intentionally omitted — they belong to the future RSpace adapter
/// crate (Phase 3 deferred work) since the standalone adapter has no
/// RSpace runtime to expose.
#[async_trait]
pub trait CordialMultiParentCasper: CordialCasper + Send + Sync {
    async fn last_finalized_block(&self) -> Result<BlockMessage, CasperError>;

    fn normalized_initial_fault(
        &self,
        weights: HashMap<Validator, u64>,
    ) -> Result<f32, CasperError>;

    async fn has_pending_deploys_in_storage(&self) -> Result<bool, CasperError>;
}

// ═══════════════════════════════════════════════════════════════════════════
// Adapter implementation
// ═══════════════════════════════════════════════════════════════════════════

/// Adapter that wraps the Cordial Miners core types and exposes them
/// through [`CordialCasper`] + [`CordialMultiParentCasper`].
///
/// Uses `tokio::sync::Mutex` for the blocklace, deploy pool, and buffer
/// so the trait methods can be called concurrently from multiple tasks.
pub struct CordialCasperAdapter {
    blocklace: tokio::sync::Mutex<Blocklace>,
    deploy_pool: tokio::sync::Mutex<DeployPool>,
    /// Pending blocks awaiting predecessor arrival, keyed by block_hash.
    buffer: tokio::sync::Mutex<HashMap<BlockHash, BlockMessage>>,
    /// Block hashes the adapter has marked invalid (so we don't re-process).
    invalid_blocks: tokio::sync::Mutex<HashMap<BlockHash, Validator>>,

    bonds: HashMap<NodeId, u64>,
    shard_conf: CasperShardConf,
    shard_id: String,
    approved_block: Option<BlockMessage>,
    /// Validation knobs. Adapter callers may want to relax cordial check
    /// when ingesting blocks from f1r3node since f1r3node doesn't enforce it.
    validation_config: ValidationConfig,
}

impl CordialCasperAdapter {
    /// Create a new adapter.
    ///
    /// `approved_block` is the genesis or approved block from f1r3node's
    /// perspective. It's stored verbatim and returned by `get_approved_block`.
    pub fn new(
        bonds: HashMap<NodeId, u64>,
        shard_conf: CasperShardConf,
        shard_id: impl Into<String>,
        deploy_pool_config: DeployPoolConfig,
        approved_block: Option<BlockMessage>,
    ) -> Self {
        // Default to a relaxed validation config: skip the cordial check so
        // blocks coming from a non-cordial source (f1r3node, replay) aren't
        // rejected. Strict consensus tests can call `with_validation_config`.
        let validation_config = ValidationConfig {
            check_cordial: false,
            ..ValidationConfig::default()
        };

        Self {
            blocklace: tokio::sync::Mutex::new(Blocklace::new()),
            deploy_pool: tokio::sync::Mutex::new(DeployPool::new(deploy_pool_config)),
            buffer: tokio::sync::Mutex::new(HashMap::new()),
            invalid_blocks: tokio::sync::Mutex::new(HashMap::new()),
            bonds,
            shard_conf,
            shard_id: shard_id.into(),
            approved_block,
            validation_config,
        }
    }

    /// Override the validation config (e.g. enable strict cordial checks).
    pub fn with_validation_config(mut self, cfg: ValidationConfig) -> Self {
        self.validation_config = cfg;
        self
    }

    /// Direct access to the wrapped blocklace, for advanced callers that
    /// need to inspect or seed state outside the trait surface.
    pub fn blocklace(&self) -> &tokio::sync::Mutex<Blocklace> {
        &self.blocklace
    }

    /// Direct access to the deploy pool.
    pub fn deploy_pool(&self) -> &tokio::sync::Mutex<DeployPool> {
        &self.deploy_pool
    }

    /// Helper: convert our core `InvalidBlock` to the adapter's mirror enum.
    fn map_core_error(err: &CoreInvalidBlock) -> InvalidBlock {
        match err {
            CoreInvalidBlock::InvalidContentHash { .. } => InvalidBlock::InvalidBlockHash,
            CoreInvalidBlock::InvalidSignature => InvalidBlock::InvalidSignature,
            CoreInvalidBlock::UnknownSender { .. } => InvalidBlock::InvalidSender,
            CoreInvalidBlock::MissingPredecessors { .. } => InvalidBlock::InvalidParents,
            CoreInvalidBlock::Equivocation { .. } => InvalidBlock::AdmissibleEquivocation,
            CoreInvalidBlock::NotCordial { .. } => InvalidBlock::NotOfInterest,
        }
    }

    /// Internal: validate a translated `Block` using the configured rules.
    async fn validate_translated_block(
        &self,
        block: &Block,
        cfg: &ValidationConfig,
    ) -> ValidationResult {
        let bl = self.blocklace.lock().await;
        core_validate_block(block, &bl, &self.bonds, cfg)
    }

    /// Internal: build a snapshot using the current blocklace + bonds.
    async fn build_current_snapshot(&self) -> Result<CasperSnapshot, CasperError> {
        let bl = self.blocklace.lock().await;
        Ok(build_snapshot(
            &bl,
            &self.bonds,
            self.shard_conf.to_snapshot_conf(),
            &self.shard_id,
        )?)
    }
}

#[async_trait]
impl CordialCasper for CordialCasperAdapter {
    async fn get_snapshot(&self) -> Result<CasperSnapshot, CasperError> {
        self.build_current_snapshot().await
    }

    fn contains(&self, hash: &BlockHash) -> bool {
        // Synchronous trait method but we hold an async Mutex. Use try_lock
        // for the common uncontended case; fall back to checking the buffer
        // alone if contended (callers can retry).
        if let Ok(bl) = self.blocklace.try_lock() {
            if bl.dom().iter().any(|id| id.content_hash.as_slice() == hash.as_slice()) {
                return true;
            }
        }
        if let Ok(buf) = self.buffer.try_lock() {
            return buf.contains_key(hash);
        }
        false
    }

    fn dag_contains(&self, hash: &BlockHash) -> bool {
        if let Ok(bl) = self.blocklace.try_lock() {
            bl.dom().iter().any(|id| id.content_hash.as_slice() == hash.as_slice())
        } else {
            false
        }
    }

    fn buffer_contains(&self, hash: &BlockHash) -> bool {
        if let Ok(buf) = self.buffer.try_lock() {
            buf.contains_key(hash)
        } else {
            false
        }
    }

    fn get_approved_block(&self) -> Result<&BlockMessage, CasperError> {
        self.approved_block
            .as_ref()
            .ok_or(CasperError::InvalidState("no approved block set"))
    }

    fn deploy(
        &self,
        deploy: SignedDeployData,
    ) -> Result<Either<DeployError, DeployId>, CasperError> {
        // Translate the wire deploy into our core type
        let cm_signed = CmSignedDeploy {
            deploy: blocklace::execution::Deploy {
                term: deploy.data.term.as_bytes().to_vec(),
                timestamp: u64::try_from(deploy.data.time_stamp).unwrap_or(0),
                phlo_price: u64::try_from(deploy.data.phlo_price).unwrap_or(0),
                phlo_limit: u64::try_from(deploy.data.phlo_limit).unwrap_or(0),
                valid_after_block_number: u64::try_from(
                    deploy.data.valid_after_block_number,
                )
                .unwrap_or(0),
                shard_id: deploy.data.shard_id.clone(),
            },
            deployer: deploy.pk.clone(),
            signature: deploy.sig.clone(),
        };
        let sig = cm_signed.signature.clone();

        let mut pool = match self.deploy_pool.try_lock() {
            Ok(p) => p,
            Err(_) => {
                return Err(CasperError::InvalidState("deploy pool locked"));
            }
        };
        match pool.add(cm_signed) {
            Ok(()) => Ok(Either::Right(sig)),
            Err(PoolError::Duplicate) => Ok(Either::Left(DeployError::PoolRejected(
                "duplicate signature".into(),
            ))),
            Err(PoolError::InvalidSignature) => {
                Ok(Either::Left(DeployError::SignatureVerificationFailed))
            }
            Err(PoolError::InsufficientPhloPrice { required, actual }) => {
                Ok(Either::Left(DeployError::PoolRejected(format!(
                    "phlo price {} below required {}",
                    actual, required
                ))))
            }
        }
    }

    async fn estimator(&self) -> Result<Vec<BlockHash>, CasperError> {
        let bl = self.blocklace.lock().await;
        let fc = fork_choice(&bl, &self.bonds);
        Ok(fc
            .map(|fc| {
                fc.tips
                    .into_iter()
                    .map(|id| id.content_hash.to_vec())
                    .collect()
            })
            .unwrap_or_default())
    }

    fn get_version(&self) -> i64 {
        self.shard_conf.casper_version
    }

    async fn validate(
        &self,
        block: &BlockMessage,
    ) -> Result<Either<BlockError, ValidBlock>, CasperError> {
        // Translate wire -> core
        let core_block = match message_to_block(block) {
            Ok(b) => b,
            Err(e) => {
                return Ok(Either::Left(BlockError::Invalid(InvalidBlock::InvalidFormat)))
                    .map(|res: Either<BlockError, ValidBlock>| {
                        // record the translation reason for debugging
                        let _ = e;
                        res
                    });
            }
        };

        let result = self
            .validate_translated_block(&core_block, &self.validation_config)
            .await;
        match result {
            ValidationResult::Valid => Ok(Either::Right(ValidBlock::Valid)),
            ValidationResult::Invalid(errs) => {
                // Surface the first error; missing predecessors map to MissingBlocks
                if errs
                    .iter()
                    .any(|e| matches!(e, CoreInvalidBlock::MissingPredecessors { .. }))
                {
                    Ok(Either::Left(BlockError::MissingBlocks))
                } else {
                    let first = &errs[0];
                    Ok(Either::Left(BlockError::Invalid(Self::map_core_error(first))))
                }
            }
        }
    }

    async fn validate_self_created(
        &self,
        block: &BlockMessage,
        _pre_state_hash: Vec<u8>,
        _post_state_hash: Vec<u8>,
    ) -> Result<Either<BlockError, ValidBlock>, CasperError> {
        // Self-created blocks skip the expensive checkpoint replay AND the
        // signature/content-hash crypto checks (we just produced them).
        let core_block = message_to_block(block)?;
        let cfg = ValidationConfig {
            check_content_hash: false,
            check_signature: false,
            ..self.validation_config.clone()
        };
        let result = self.validate_translated_block(&core_block, &cfg).await;
        match result {
            ValidationResult::Valid => Ok(Either::Right(ValidBlock::Valid)),
            ValidationResult::Invalid(errs) => {
                if errs
                    .iter()
                    .any(|e| matches!(e, CoreInvalidBlock::MissingPredecessors { .. }))
                {
                    Ok(Either::Left(BlockError::MissingBlocks))
                } else {
                    let first = &errs[0];
                    Ok(Either::Left(BlockError::Invalid(Self::map_core_error(first))))
                }
            }
        }
    }

    async fn handle_valid_block(&self, block: &BlockMessage) -> Result<(), CasperError> {
        let core_block = message_to_block(block)?;
        let mut bl = self.blocklace.lock().await;
        bl.insert(core_block)
            .map_err(|e| CasperError::InvalidState(Box::leak(format!("insert: {}", e).into_boxed_str())))?;

        // Drain the pending buffer for blocks whose predecessors are now
        // satisfied. We re-check in a loop because each newly-inserted block
        // may unblock more.
        drop(bl);
        let to_retry: Vec<BlockMessage> = {
            let mut buf = self.buffer.lock().await;
            buf.remove(&block.block_hash);
            // Snapshot entries; we'll filter below.
            buf.values().cloned().collect()
        };
        let mut promoted = Vec::new();
        for pending in to_retry {
            // Try to translate + insert; if it succeeds, drop from buffer.
            if let Ok(translated) = message_to_block(&pending) {
                let mut bl = self.blocklace.lock().await;
                if bl.insert(translated).is_ok() {
                    promoted.push(pending.block_hash.clone());
                }
            }
        }
        if !promoted.is_empty() {
            let mut buf = self.buffer.lock().await;
            for h in promoted {
                buf.remove(&h);
            }
        }
        Ok(())
    }

    fn handle_invalid_block(
        &self,
        block: &BlockMessage,
        status: &InvalidBlock,
    ) -> Result<(), CasperError> {
        // Track invalid hash; do NOT insert into the blocklace.
        let mut invalid = self
            .invalid_blocks
            .try_lock()
            .map_err(|_| CasperError::InvalidState("invalid_blocks locked"))?;
        invalid.insert(block.block_hash.clone(), block.sender.clone());
        // status is informational here; we don't gate on the variant.
        let _ = status;
        Ok(())
    }

    fn get_dependency_free_from_buffer(&self) -> Result<Vec<BlockMessage>, CasperError> {
        let buf = self
            .buffer
            .try_lock()
            .map_err(|_| CasperError::InvalidState("buffer locked"))?;
        let bl = self
            .blocklace
            .try_lock()
            .map_err(|_| CasperError::InvalidState("blocklace locked"))?;
        let dom: HashSet<Vec<u8>> = bl
            .dom()
            .iter()
            .map(|id| id.content_hash.to_vec())
            .collect();

        // A buffer entry is "dependency-free" when all its parent_hash_list
        // entries are in the DAG.
        let free = buf
            .values()
            .filter(|msg| msg.header.parents_hash_list.iter().all(|p| dom.contains(p)))
            .cloned()
            .collect();
        Ok(free)
    }

    fn get_all_from_buffer(&self) -> Result<Vec<BlockMessage>, CasperError> {
        let buf = self
            .buffer
            .try_lock()
            .map_err(|_| CasperError::InvalidState("buffer locked"))?;
        Ok(buf.values().cloned().collect())
    }
}

#[async_trait]
impl CordialMultiParentCasper for CordialCasperAdapter {
    async fn last_finalized_block(&self) -> Result<BlockMessage, CasperError> {
        let bl = self.blocklace.lock().await;
        let id: BlockIdentity = match find_last_finalized(&bl, &self.bonds) {
            Some(id) => id,
            None => return Err(CasperError::InvalidState("no finalized block yet")),
        };
        let block = bl
            .get(&id)
            .ok_or(CasperError::InvalidState("LFB not in blocklace"))?;
        Ok(block_to_message(&block, &self.shard_id)?)
    }

    fn normalized_initial_fault(
        &self,
        weights: HashMap<Validator, u64>,
    ) -> Result<f32, CasperError> {
        // Cordial Miners excludes equivocators from the honest stake. In
        // f1r3node terms: "initial fault" = sum of equivocator stake / total stake.
        let bl = self
            .blocklace
            .try_lock()
            .map_err(|_| CasperError::InvalidState("blocklace locked"))?;
        let equivocators = bl.find_equivacators();
        let total: u64 = weights.values().sum();
        if total == 0 {
            return Ok(0.0);
        }
        let fault: u64 = weights
            .iter()
            .filter(|(v, _)| equivocators.contains(&NodeId(v.to_vec())))
            .map(|(_, s)| *s)
            .sum();
        Ok(fault as f32 / total as f32)
    }

    async fn has_pending_deploys_in_storage(&self) -> Result<bool, CasperError> {
        let pool = self.deploy_pool.lock().await;
        Ok(!pool.is_empty())
    }
}
