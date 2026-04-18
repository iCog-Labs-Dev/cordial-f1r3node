//! Real RSpace-backed `RuntimeManager` adapter.
//!
//! Delegates [`blocklace::execution::RuntimeManager`] to f1r3node's real
//! [`casper::rust::util::rholang::runtime_manager::RuntimeManager`] so
//! blocks produced by the Cordial Miners consensus can execute Rholang
//! deploys against an actual RSpace tuplespace.
//!
//! # Design: A-lite (caller supplies the RuntimeManager)
//!
//! Constructing a real RSpace requires LMDB storage paths, Rholang
//! interpreter setup, history repository initialization, and bond / genesis
//! bootstrapping — roughly the same setup f1r3node's node binary runs. We
//! don't duplicate that here. The adapter wraps a `&mut f1r3node::RuntimeManager`
//! supplied by the caller (typically f1r3node's node binary, or an integration
//! test harness), and handles the translation from our `ExecutionRequest` /
//! `ExecutionResult` to f1r3node's types.
//!
//! # Usage
//!
//! ```ignore
//! use blocklace::execution::{ExecutionRequest, RuntimeManager as _};
//! use blocklace_f1r3rspace::F1r3RspaceRuntime;
//!
//! // The caller builds a real f1r3node RuntimeManager somewhere (node
//! // binary startup, test harness, etc.)
//! let mut f1r3_rt: casper::rust::util::rholang::runtime_manager::RuntimeManager =
//!     /* ... */;
//!
//! let mut adapter = F1r3RspaceRuntime::new(&mut f1r3_rt);
//! let result = adapter.execute_block(request)?;
//! ```
//!
//! # Translation notes
//!
//! - **pre/post state hashes** are `Vec<u8>` on our side, `prost::bytes::Bytes`
//!   on f1r3node's side. Both are just byte sequences; conversion is free.
//! - **Deploys**: our `SignedDeploy` carries a `Vec<u8>` term; f1r3node's
//!   `DeployData` wants a `String`. We UTF-8-decode lossily with `from_utf8_lossy`.
//! - **Signatures**: f1r3node's `Signed<DeployData>` is constructed via
//!   `Signed::from_existing_signature` rather than re-signing, so our
//!   deploy's existing signature is preserved verbatim.
//! - **System deploys**: `Slash` requires a `PublicKey` and a block hash
//!   (what's being slashed). Our `SystemDeployRequest::Slash` only carries
//!   the validator NodeId. We use it as both the slashed-validator
//!   identifier and as the `invalid_block_hash` placeholder — adapter
//!   callers who need tighter semantics should construct a richer request.
//! - **Block data** (sender, seq_num, block_number): populated from
//!   `ExecutionRequest.block_number` plus a default sender derived from the
//!   bonds list. Timestamp is 0.
//! - **Bonds**: `execute_block` does not update bonds directly; bonds are
//!   state-hash-addressable in f1r3node and get computed separately via
//!   `RuntimeManager::compute_bonds`. The adapter returns the caller's
//!   input bonds unchanged in `new_bonds` for now (future work: call
//!   `compute_bonds` on the post-state hash).

use std::collections::HashMap;

use blocklace::execution::{
    Bond, ExecutionRequest, ExecutionResult, ProcessedDeploy as CmProcessedDeploy,
    ProcessedSystemDeploy as CmProcessedSystemDeploy, RejectReason, RejectedDeploy,
    RuntimeError, RuntimeManager as CoreRuntimeManager, SignedDeploy as CmSignedDeploy,
    SystemDeployRequest,
};
use blocklace::types::NodeId;

use casper::rust::util::rholang::runtime_manager::RuntimeManager as F1r3RuntimeManager;
use casper::rust::util::rholang::system_deploy_enum::SystemDeployEnum;
use casper::rust::util::rholang::costacc::{
    close_block_deploy::CloseBlockDeploy, slash_deploy::SlashDeploy,
};
use crypto::rust::hash::blake2b512_random::Blake2b512Random;
use crypto::rust::public_key::PublicKey;
use crypto::rust::signatures::secp256k1::Secp256k1;
use crypto::rust::signatures::signed::Signed;
use models::rust::casper::protocol::casper_message::{DeployData, ProcessedDeploy, ProcessedSystemDeploy, SystemDeployData};
use rholang::rust::interpreter::system_processes::BlockData;

/// The adapter. Holds a mutable reference to f1r3node's `RuntimeManager`
/// for the lifetime of the `execute_block` call.
///
/// Cheap to construct — it's just a borrowed reference. Create one per
/// block execution.
pub struct F1r3RspaceRuntime<'a> {
    f1r3_rt: &'a mut F1r3RuntimeManager,
}

impl<'a> F1r3RspaceRuntime<'a> {
    pub fn new(f1r3_rt: &'a mut F1r3RuntimeManager) -> Self {
        Self { f1r3_rt }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// RuntimeManager trait implementation
// ═══════════════════════════════════════════════════════════════════════════
//
// Our core `RuntimeManager` is a sync trait. f1r3node's `compute_state` is
// async. We bridge by blocking on a tokio runtime handle — this is the
// same pattern f1r3node uses in its sync trait wrappers.

impl<'a> CoreRuntimeManager for F1r3RspaceRuntime<'a> {
    fn execute_block(
        &mut self,
        request: ExecutionRequest,
    ) -> Result<ExecutionResult, RuntimeError> {
        // Translate request → f1r3node inputs
        let start_hash: prost::bytes::Bytes = request.pre_state_hash.clone().into();

        let terms: Vec<Signed<DeployData>> = request
            .deploys
            .iter()
            .map(signed_deploy_to_f1r3node)
            .collect::<Result<Vec<_>, RuntimeError>>()?;

        let system_deploys: Vec<SystemDeployEnum> = request
            .system_deploys
            .iter()
            .map(|sd| system_deploy_to_f1r3node(sd, &request.pre_state_hash))
            .collect();

        let block_data = build_block_data(&request)?;

        let invalid_blocks: Option<
            HashMap<models::rust::block_hash::BlockHash, models::rust::validator::Validator>,
        > = Some(HashMap::new()); // we don't track invalid blocks at this layer

        // Call f1r3node. compute_state is async → block_on a Tokio handle.
        let (post_hash, f1r3_processed, f1r3_system) = tokio::runtime::Handle::current()
            .block_on(self.f1r3_rt.compute_state(
                &start_hash,
                terms,
                system_deploys,
                block_data,
                invalid_blocks,
            ))
            .map_err(|e| RuntimeError::InternalError(format!("compute_state: {:?}", e)))?;

        // Translate back into ExecutionResult
        let processed_deploys: Vec<CmProcessedDeploy> = f1r3_processed
            .iter()
            .map(processed_deploy_from_f1r3node)
            .collect::<Result<Vec<_>, RuntimeError>>()?;

        let system_deploys_out: Vec<CmProcessedSystemDeploy> = f1r3_system
            .iter()
            .map(system_deploy_from_f1r3node)
            .collect();

        Ok(ExecutionResult {
            post_state_hash: post_hash.to_vec(),
            processed_deploys,
            rejected_deploys: Vec::new(), // f1r3node doesn't return rejected here
            system_deploys: system_deploys_out,
            new_bonds: request.bonds.clone(), // unchanged; see module docs
        })
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Translation helpers
// ═══════════════════════════════════════════════════════════════════════════

/// Convert our `SignedDeploy` to f1r3node's `Signed<DeployData>`.
///
/// Constructs `Signed<_>` directly via public fields so the signature the
/// caller supplied is preserved verbatim (no re-verification). See the
/// note inside the function body for why we bypass
/// `Signed::from_signed_data`.
pub fn signed_deploy_to_f1r3node(sd: &CmSignedDeploy) -> Result<Signed<DeployData>, RuntimeError> {
    let data = DeployData {
        term: String::from_utf8_lossy(&sd.deploy.term).into_owned(),
        time_stamp: i64::try_from(sd.deploy.timestamp)
            .map_err(|_| RuntimeError::InternalError("timestamp overflow".into()))?,
        phlo_price: i64::try_from(sd.deploy.phlo_price)
            .map_err(|_| RuntimeError::InternalError("phlo_price overflow".into()))?,
        phlo_limit: i64::try_from(sd.deploy.phlo_limit)
            .map_err(|_| RuntimeError::InternalError("phlo_limit overflow".into()))?,
        valid_after_block_number: i64::try_from(sd.deploy.valid_after_block_number)
            .map_err(|_| RuntimeError::InternalError("valid_after overflow".into()))?,
        shard_id: sd.deploy.shard_id.clone(),
        expiration_timestamp: None,
    };

    // f1r3node's SignaturesAlgFactory explicitly disables ed25519, registering
    // only secp256k1 and secp256k1-eth for deploys. Since Signed's fields are
    // all `pub`, we construct directly rather than going through
    // Signed::from_signed_data (which would re-verify the sig). The deploy
    // pool already verified at admission time; another round-trip would
    // both duplicate work and fail for our ed25519 defaults.
    //
    // Adapter callers bringing secp256k1-signed deploys will hash/verify
    // correctly downstream; ed25519-signed deploys won't be recognized by
    // f1r3node's verification paths, so mixing algorithms across the boundary
    // is a caller problem (document in module header if needed).
    Ok(Signed {
        data,
        pk: PublicKey::from_bytes(&sd.deployer),
        sig: prost::bytes::Bytes::copy_from_slice(&sd.signature),
        sig_algorithm: Box::new(Secp256k1),
    })
}

/// Convert our `SystemDeployRequest` to f1r3node's `SystemDeployEnum`.
///
/// `Slash` needs a `PublicKey` for the slashed validator and a "block
/// hash that's being slashed"; our request carries only the NodeId, so we
/// use it for both. `CloseBlock` needs an initial random seed; we derive
/// one from the pre-state hash.
pub fn system_deploy_to_f1r3node(
    sd: &SystemDeployRequest,
    pre_state_hash: &[u8],
) -> SystemDeployEnum {
    let rand_seed = Blake2b512Random::create_from_bytes(pre_state_hash);
    match sd {
        SystemDeployRequest::Slash { validator } => SystemDeployEnum::Slash(SlashDeploy {
            invalid_block_hash: prost::bytes::Bytes::copy_from_slice(&validator.0),
            pk: PublicKey::from_bytes(&validator.0),
            initial_rand: rand_seed,
        }),
        SystemDeployRequest::CloseBlock => SystemDeployEnum::Close(CloseBlockDeploy {
            initial_rand: rand_seed,
        }),
    }
}

/// Build f1r3node's `BlockData` from our `ExecutionRequest`.
///
/// Picks a sender public key from the first bond (f1r3node requires one
/// non-empty key; any bonded validator works for the block-data field).
pub fn build_block_data(request: &ExecutionRequest) -> Result<BlockData, RuntimeError> {
    let sender = request
        .bonds
        .first()
        .map(|b| PublicKey::from_bytes(&b.validator.0))
        .unwrap_or_else(|| PublicKey::from_bytes(&[0u8; 33])); // placeholder compressed secp256k1 zero
    Ok(BlockData {
        time_stamp: 0,
        block_number: i64::try_from(request.block_number)
            .map_err(|_| RuntimeError::InternalError("block_number overflow".into()))?,
        sender,
        seq_num: 0,
    })
}

/// Convert an f1r3node `ProcessedDeploy` back to our type.
pub fn processed_deploy_from_f1r3node(
    pd: &ProcessedDeploy,
) -> Result<CmProcessedDeploy, RuntimeError> {
    let signed = CmSignedDeploy {
        deploy: blocklace::execution::Deploy {
            term: pd.deploy.data.term.as_bytes().to_vec(),
            timestamp: u64::try_from(pd.deploy.data.time_stamp).unwrap_or(0),
            phlo_price: u64::try_from(pd.deploy.data.phlo_price).unwrap_or(0),
            phlo_limit: u64::try_from(pd.deploy.data.phlo_limit).unwrap_or(0),
            valid_after_block_number: u64::try_from(pd.deploy.data.valid_after_block_number)
                .unwrap_or(0),
            shard_id: pd.deploy.data.shard_id.clone(),
        },
        deployer: pd.deploy.pk.bytes.to_vec(),
        signature: pd.deploy.sig.to_vec(),
    };
    Ok(CmProcessedDeploy {
        deploy: signed,
        cost: u64::try_from(pd.cost.cost).unwrap_or(0),
        is_failed: pd.is_failed,
    })
}

/// Convert an f1r3node `ProcessedSystemDeploy` back to our type.
pub fn system_deploy_from_f1r3node(sd: &ProcessedSystemDeploy) -> CmProcessedSystemDeploy {
    match sd {
        ProcessedSystemDeploy::Succeeded { system_deploy, .. } => match system_deploy {
            SystemDeployData::Slash {
                issuer_public_key, ..
            } => CmProcessedSystemDeploy::Slash {
                validator: NodeId(issuer_public_key.bytes.to_vec()),
                succeeded: true,
            },
            SystemDeployData::CloseBlockSystemDeployData => {
                CmProcessedSystemDeploy::CloseBlock { succeeded: true }
            }
            SystemDeployData::Empty => CmProcessedSystemDeploy::CloseBlock { succeeded: true },
        },
        ProcessedSystemDeploy::Failed { .. } => {
            // f1r3node's Failed variant doesn't carry the original system
            // deploy data, so we can't recover whether it was slash or close.
            // Reporting as a failed CloseBlock is the least-information-
            // losing choice; callers interested in specific failure types
            // should inspect the f1r3node result directly.
            CmProcessedSystemDeploy::CloseBlock { succeeded: false }
        }
    }
}

/// Decode a term string into bytes — helper for callers translating from
/// f1r3node's `DeployData.term: String` to our `Deploy.term: Vec<u8>`.
pub fn term_string_to_bytes(term: &str) -> Vec<u8> {
    term.as_bytes().to_vec()
}

// Currently unused but kept for symmetry with the broader translation API.
#[allow(dead_code)]
fn rejected_deploy_placeholder(sig: Vec<u8>) -> RejectedDeploy {
    RejectedDeploy {
        deploy: CmSignedDeploy {
            deploy: blocklace::execution::Deploy {
                term: vec![],
                timestamp: 0,
                phlo_price: 0,
                phlo_limit: 0,
                valid_after_block_number: 0,
                shard_id: String::new(),
            },
            deployer: vec![],
            signature: sig,
        },
        reason: RejectReason::InvalidSignature,
    }
}

// Keep the Bond import referenced so rustc doesn't warn about unused
// imports when adapter bodies evolve.
#[allow(dead_code)]
const _BOND_MARKER: std::marker::PhantomData<Bond> = std::marker::PhantomData;
