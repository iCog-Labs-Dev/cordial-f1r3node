//! Outbound block-creation pipeline for Cordial Miners.
//!
//! The proposer is the mirror of the inbound gRPC ingest path: it selects tips
//! from the local blocklace, pulls deploys from the mempool, executes them
//! through an [`ExecutionEngine`] (RSpace in production, [`MockRuntime`] in
//! tests), signs the block, and broadcasts it.
//!
//! ## Pipeline (strictly sequential)
//!
//! 1. [`TipSelector::select_tips`] — parent block identities from the DAG
//! 2. [`DeployPool::select_for_block`] — pending deploys from the mempool
//! 3. [`ExecutionEngine::execute`] — run deploys, obtain `post_state_hash`
//! 4. [`BlockSigner::sign_block`] — assemble and cryptographically sign
//! 5. [`BlockBroadcaster::broadcast`] — publish to the network
//!
//! Multi-parent **state merge** is owned by f1r3node's `RuntimeManager` when
//! wired to real RSpace. This module uses the highest `block_number` tip as
//! the execution parent, which is sufficient for [`MockRuntime`] chaining.

use std::collections::{HashMap, HashSet};

use cordial_miners_core::Block;
use cordial_miners_core::blocklace::Blocklace;
use cordial_miners_core::consensus::select_predecessors;
use cordial_miners_core::crypto::{SignatureScheme, hash_content};
use cordial_miners_core::execution::{
    Bond, BlockState, CordialBlockPayload, DeployPool, DeployPoolConfig,
    ExecutionRequest, ExecutionResult, RuntimeError, RuntimeManager, SystemDeployRequest,
    compute_deploys_in_scope,
};
use cordial_miners_core::types::{BlockContent, BlockIdentity, NodeId};


#[derive(Debug, Clone, PartialEq)]
pub enum ProposeError {
    NoTips,
    Execution(RuntimeError),
    Sign(String),
    Broadcast(String),
    PayloadDecode(String),
}

impl std::fmt::Display for ProposeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoTips => write!(f, "no honest tips available for proposal"),
            Self::Execution(e) => write!(f, "execution failed: {e:?}"),
            Self::Sign(msg) => write!(f, "signing failed: {msg}"),
            Self::Broadcast(msg) => write!(f, "broadcast failed: {msg}"),
            Self::PayloadDecode(msg) => write!(f, "payload decode failed: {msg}"),
        }
    }
}

impl std::error::Error for ProposeError {}

pub trait TipSelector {
    fn select_tips(
        &self,
        blocklace: &Blocklace,
        bonds: &HashMap<NodeId, u64>,
    ) -> HashSet<BlockIdentity>;
}

pub trait ExecutionEngine {
    fn execute(&mut self, request: ExecutionRequest) -> Result<ExecutionResult, RuntimeError>;
}

pub trait BlockSigner {
    fn sign_block(
        &self,
        content: &BlockContent,
        creator: &NodeId,
    ) -> Result<BlockIdentity, String>;
}

pub trait BlockBroadcaster {
    fn broadcast(&self, block: &Block) -> Result<(), String>;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct DisseminationTipSelector;

impl TipSelector for DisseminationTipSelector {
    fn select_tips(
        &self,
        blocklace: &Blocklace,
        bonds: &HashMap<NodeId, u64>,
    ) -> HashSet<BlockIdentity> {
        select_predecessors(blocklace, bonds)
    }
}

pub struct RuntimeExecutionEngine<R> {
    runtime: R,
}

impl<R> RuntimeExecutionEngine<R> {
    pub fn new(runtime: R) -> Self {
        Self { runtime }
    }

    pub fn into_inner(self) -> R {
        self.runtime
    }
}

impl<R: RuntimeManager> ExecutionEngine for RuntimeExecutionEngine<R> {
    fn execute(&mut self, request: ExecutionRequest) -> Result<ExecutionResult, RuntimeError> {
        self.runtime.execute_block(request)
    }
}

pub struct Secp256k1BlockSigner {
    private_key: Vec<u8>,
}

impl Secp256k1BlockSigner {
    pub fn new(private_key: Vec<u8>) -> Self {
        Self { private_key }
    }
}

impl BlockSigner for Secp256k1BlockSigner {
    fn sign_block(
        &self,
        content: &BlockContent,
        creator: &NodeId,
    ) -> Result<BlockIdentity, String> {
        use cordial_miners_core::crypto::Secp256k1Scheme;

        let content_hash = hash_content(content);
        let signature = Secp256k1Scheme.sign(&content_hash, &self.private_key)?;
        Ok(BlockIdentity {
            content_hash,
            creator: creator.clone(),
            signature,
        })
    }
}

pub struct FnBroadcaster<F>
where
    F: Fn(&Block) -> Result<(), String>,
{
    f: F,
}

impl<F> FnBroadcaster<F>
where
    F: Fn(&Block) -> Result<(), String>,
{
    pub fn new(f: F) -> Self {
        Self { f }
    }
}

impl<F> BlockBroadcaster for FnBroadcaster<F>
where
    F: Fn(&Block) -> Result<(), String>,
{
    fn broadcast(&self, block: &Block) -> Result<(), String> {
        (self.f)(block)
    }
}

pub struct RecordingBroadcaster {
    pub blocks: std::sync::Arc<std::sync::Mutex<Vec<Block>>>,
}

impl RecordingBroadcaster {
    pub fn new() -> Self {
        Self {
            blocks: std::sync::Arc::new(std::sync::Mutex::new(Vec::new())),
        }
    }
}

impl Default for RecordingBroadcaster {
    fn default() -> Self {
        Self::new()
    }
}

impl BlockBroadcaster for RecordingBroadcaster {
    fn broadcast(&self, block: &Block) -> Result<(), String> {
        self.blocks
            .lock()
            .map_err(|e| format!("lock poisoned: {e}"))?
            .push(block.clone());
        Ok(())
    }
}


fn derive_chain_head(
    blocklace: &Blocklace,
    predecessors: &HashSet<BlockIdentity>,
) -> Result<(Vec<u8>, u64, Vec<Bond>), ProposeError> {
    let mut best_number: Option<u64> = None;
    let mut pre_state_hash = Vec::new();
    let mut bonds = Vec::new();

    for pred_id in predecessors {
        let block = blocklace
            .get(pred_id)
            .ok_or_else(|| ProposeError::PayloadDecode(format!("missing predecessor {pred_id:?}")))?;
        let payload = CordialBlockPayload::from_bytes(&block.content.payload)
            .map_err(ProposeError::PayloadDecode)?;
        let n = payload.state.block_number;
        if best_number.is_none_or(|best| n > best) {
            best_number = Some(n);
            pre_state_hash = payload.state.post_state_hash.clone();
            bonds = payload.state.bonds.clone();
        }
    }

    let block_number = best_number.map(|n| n + 1).unwrap_or(0);
    Ok((pre_state_hash, block_number, bonds))
}

fn bonds_map_to_vec(bonds: &HashMap<NodeId, u64>) -> Vec<Bond> {
    bonds
        .iter()
        .map(|(validator, stake)| Bond {
            validator: validator.clone(),
            stake: *stake,
        })
        .collect()
}


pub struct CordialProposer<TS, EE, BS, BC> {
    tip_selector: TS,
    execution: EE,
    signer: BS,
    broadcaster: BC,
    creator: NodeId,
    bonds: HashMap<NodeId, u64>,
    deploy_pool_config: DeployPoolConfig,
    include_close_block: bool,
}

impl<TS, EE, BS, BC> CordialProposer<TS, EE, BS, BC> {
    pub fn new(
        tip_selector: TS,
        execution: EE,
        signer: BS,
        broadcaster: BC,
        creator: NodeId,
        bonds: HashMap<NodeId, u64>,
        deploy_pool_config: DeployPoolConfig,
    ) -> Self {
        Self {
            tip_selector,
            execution,
            signer,
            broadcaster,
            creator,
            bonds,
            deploy_pool_config,
            include_close_block: true,
        }
    }

    pub fn with_close_block(mut self, include: bool) -> Self {
        self.include_close_block = include;
        self
    }
}

impl<TS, EE, BS, BC> CordialProposer<TS, EE, BS, BC>
where
    TS: TipSelector,
    EE: ExecutionEngine,
    BS: BlockSigner,
    BC: BlockBroadcaster,
{
    pub fn propose(
        &mut self,
        blocklace: &Blocklace,
        deploy_pool: &DeployPool,
    ) -> Result<Block, ProposeError> {
        let predecessors = self.tip_selector.select_tips(blocklace, &self.bonds);

        let (pre_state_hash, block_number, bonds) = if predecessors.is_empty() {
            if blocklace.dom().is_empty() {
                (vec![], 0u64, bonds_map_to_vec(&self.bonds))
            } else {
                return Err(ProposeError::NoTips);
            }
        } else {
            derive_chain_head(blocklace, &predecessors)?
        };

        let deploys_in_scope = compute_deploys_in_scope(
            blocklace,
            &predecessors,
            block_number,
            self.deploy_pool_config.deploy_lifespan,
        );

        let selected = deploy_pool.select_for_block(block_number, 0, &deploys_in_scope);

        let mut system_deploys = Vec::new();
        if self.include_close_block {
            system_deploys.push(SystemDeployRequest::CloseBlock);
        }

        let request = ExecutionRequest {
            pre_state_hash: pre_state_hash.clone(),
            deploys: selected.deploys,
            system_deploys,
            bonds: bonds.clone(),
            block_number,
        };

        let result = self
            .execution
            .execute(request)
            .map_err(ProposeError::Execution)?;

        let payload = CordialBlockPayload {
            state: BlockState {
                pre_state_hash,
                post_state_hash: result.post_state_hash,
                bonds: result.new_bonds,
                block_number,
            },
            deploys: result.processed_deploys,
            rejected_deploys: result.rejected_deploys,
            system_deploys: result.system_deploys,
        };

        let content = BlockContent {
            payload: payload.to_bytes(),
            predecessors: predecessors.clone(),
        };

        let identity = self
            .signer
            .sign_block(&content, &self.creator)
            .map_err(ProposeError::Sign)?;

        let block = Block { identity, content };

        self.broadcaster
            .broadcast(&block)
            .map_err(ProposeError::Broadcast)?;

        Ok(block)
    }
}
