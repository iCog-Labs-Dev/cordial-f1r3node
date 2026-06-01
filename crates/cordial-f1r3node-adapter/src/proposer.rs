use std::collections::{BTreeSet, HashMap, HashSet};

use cordial_miners_core::Block;
use cordial_miners_core::blocklace::Blocklace;
use cordial_miners_core::consensus::{
    CordialEquivocationEvidence, EvidencePool, select_predecessors,
};
use cordial_miners_core::crypto::{SignatureScheme, hash_content};
use cordial_miners_core::execution::{
    BlockState, Bond, CordialBlockPayload, DeployPool, DeployPoolConfig, ExecutionRequest,
    ExecutionResult, RuntimeError, RuntimeManager, SystemDeployRequest, compute_deploys_in_scope,
};
use cordial_miners_core::types::{BlockContent, BlockIdentity, NodeId};
use models::casper::{ProcessedSystemDeployProto, system_deploy_data_proto::SystemDeploy};
use prost::Message;

use crate::slashing::SlashDeployFormatter;

#[derive(Debug, Clone, PartialEq)]
pub enum ProposeError {
    NoTips,
    Execution(RuntimeError),
    Sign(String),
    Broadcast(String),
    PayloadDecode(String),
    SlashFormat(String),
}

impl std::fmt::Display for ProposeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoTips => write!(f, "no honest tips available for proposal"),
            Self::Execution(e) => write!(f, "execution failed: {e:?}"),
            Self::Sign(msg) => write!(f, "signing failed: {msg}"),
            Self::Broadcast(msg) => write!(f, "broadcast failed: {msg}"),
            Self::PayloadDecode(msg) => write!(f, "payload decode failed: {msg}"),
            Self::SlashFormat(msg) => write!(f, "slash deploy formatting failed: {msg}"),
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

/// Exposes equivocation evidence retained by the pure Cordial core.
pub trait EvidenceSource {
    fn pending_evidence(&mut self) -> Vec<CordialEquivocationEvidence>;
}

pub trait ExecutionEngine {
    fn execute(&mut self, request: ExecutionRequest) -> Result<ExecutionResult, RuntimeError>;
}

pub trait BlockSigner {
    fn sign_block(&self, content: &BlockContent, creator: &NodeId)
    -> Result<BlockIdentity, String>;
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

#[derive(Debug, Clone, Copy, Default)]
pub struct NoEvidenceSource;

impl EvidenceSource for NoEvidenceSource {
    fn pending_evidence(&mut self) -> Vec<CordialEquivocationEvidence> {
        Vec::new()
    }
}

/// Evidence source backed by a core [`EvidencePool`].
///
/// The adapter supplies the validator set it wants to query; the pool remains a
/// pure core data structure and never sees f1r3node protobuf or RSpace types.
pub struct EvidencePoolSource<'a, P> {
    pool: &'a P,
    validators: Vec<NodeId>,
}

impl<'a, P> EvidencePoolSource<'a, P> {
    pub fn new<I>(pool: &'a P, validators: I) -> Self
    where
        I: IntoIterator<Item = NodeId>,
    {
        let validators = validators.into_iter().collect::<BTreeSet<_>>();
        Self {
            pool,
            validators: validators.into_iter().collect(),
        }
    }
}

impl<P> EvidenceSource for EvidencePoolSource<'_, P>
where
    P: EvidencePool<NodeId, Block, BlockIdentity>,
{
    fn pending_evidence(&mut self) -> Vec<CordialEquivocationEvidence> {
        self.validators
            .iter()
            .flat_map(|validator| self.pool.evidence_for(validator))
            .collect()
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct NoopSlashFormatter;

impl SlashDeployFormatter<NodeId, Block, BlockIdentity> for NoopSlashFormatter {
    fn to_slash_system_deploys(
        &self,
        _evidence: &[CordialEquivocationEvidence],
    ) -> anyhow::Result<Vec<Vec<u8>>> {
        Ok(Vec::new())
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
    let mut best_id: Option<BlockIdentity> = None;
    let mut pre_state_hash = Vec::new();
    let mut bonds = Vec::new();

    for pred_id in predecessors {
        let block = blocklace.get(pred_id).ok_or_else(|| {
            ProposeError::PayloadDecode(format!("missing predecessor {pred_id:?}"))
        })?;
        let payload = CordialBlockPayload::from_bytes(&block.content.payload)
            .map_err(ProposeError::PayloadDecode)?;
        let n = payload.state.block_number;
        let should_update = match best_number {
            None => true,
            Some(best) => {
                if n > best {
                    true
                } else if n < best {
                    false
                } else {
                    match best_id.as_ref() {
                        Some(current) => {
                            compare_identity(pred_id, current) == std::cmp::Ordering::Greater
                        }
                        None => true,
                    }
                }
            }
        };

        if should_update {
            best_number = Some(n);
            best_id = Some(pred_id.clone());
            pre_state_hash = payload.state.post_state_hash.clone();
            bonds = payload.state.bonds.clone();
        }
    }

    let block_number = best_number.map(|n| n + 1).unwrap_or(0);
    Ok((pre_state_hash, block_number, bonds))
}

fn bonds_map_to_vec(bonds: &HashMap<NodeId, u64>) -> Vec<Bond> {
    let mut out: Vec<Bond> = bonds
        .iter()
        .map(|(validator, stake)| Bond {
            validator: validator.clone(),
            stake: *stake,
        })
        .collect();
    out.sort_by(|a, b| a.validator.0.cmp(&b.validator.0));
    out
}

fn compare_identity(a: &BlockIdentity, b: &BlockIdentity) -> std::cmp::Ordering {
    a.content_hash
        .cmp(&b.content_hash)
        .then_with(|| a.creator.0.cmp(&b.creator.0))
        .then_with(|| a.signature.cmp(&b.signature))
}

fn slash_system_deploys<ES, SF>(
    evidence_source: &mut ES,
    slash_formatter: &SF,
) -> Result<Vec<SystemDeployRequest>, ProposeError>
where
    ES: EvidenceSource,
    SF: SlashDeployFormatter<NodeId, Block, BlockIdentity>,
{
    let evidence = evidence_source.pending_evidence();
    if evidence.is_empty() {
        return Ok(Vec::new());
    }

    let formatted = slash_formatter
        .to_slash_system_deploys(&evidence)
        .map_err(|err| ProposeError::SlashFormat(err.to_string()))?;

    if formatted.len() != evidence.len() {
        return Err(ProposeError::SlashFormat(format!(
            "formatter returned {} slash deploys for {} evidence records",
            formatted.len(),
            evidence.len()
        )));
    }

    evidence
        .iter()
        .zip(formatted.iter())
        .map(|(record, bytes)| decode_slash_system_deploy(record, bytes))
        .collect()
}

fn decode_slash_system_deploy(
    record: &CordialEquivocationEvidence,
    bytes: &[u8],
) -> Result<SystemDeployRequest, ProposeError> {
    let proto = ProcessedSystemDeployProto::decode(bytes)
        .map_err(|err| ProposeError::SlashFormat(err.to_string()))?;
    let system_deploy = proto
        .system_deploy
        .and_then(|data| data.system_deploy)
        .ok_or_else(|| ProposeError::SlashFormat("missing slash system deploy".to_string()))?;

    let SystemDeploy::SlashSystemDeploy(slash) = system_deploy else {
        return Err(ProposeError::SlashFormat(
            "formatted system deploy is not a slash deploy".to_string(),
        ));
    };

    let invalid_block_hash = slash.invalid_block_hash.to_vec();
    SystemDeployRequest::validate_invalid_block_hash(&invalid_block_hash)
        .map_err(ProposeError::SlashFormat)?;

    Ok(SystemDeployRequest::Slash {
        validator: record.validator.clone(),
        invalid_block_hash,
    })
}

pub struct CordialProposer<TS, EE, BS, BC, ES = NoEvidenceSource, SF = NoopSlashFormatter> {
    tip_selector: TS,
    execution: EE,
    signer: BS,
    broadcaster: BC,
    evidence_source: ES,
    slash_formatter: SF,
    creator: NodeId,
    bonds: HashMap<NodeId, u64>,
    deploy_pool_config: DeployPoolConfig,
    include_close_block: bool,
}

impl<TS, EE, BS, BC> CordialProposer<TS, EE, BS, BC, NoEvidenceSource, NoopSlashFormatter> {
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
            evidence_source: NoEvidenceSource,
            slash_formatter: NoopSlashFormatter,
            creator,
            bonds,
            deploy_pool_config,
            include_close_block: true,
        }
    }
}

impl<TS, EE, BS, BC, ES, SF> CordialProposer<TS, EE, BS, BC, ES, SF> {
    pub fn with_slashing<NES, NSF>(
        self,
        evidence_source: NES,
        slash_formatter: NSF,
    ) -> CordialProposer<TS, EE, BS, BC, NES, NSF> {
        CordialProposer {
            tip_selector: self.tip_selector,
            execution: self.execution,
            signer: self.signer,
            broadcaster: self.broadcaster,
            evidence_source,
            slash_formatter,
            creator: self.creator,
            bonds: self.bonds,
            deploy_pool_config: self.deploy_pool_config,
            include_close_block: self.include_close_block,
        }
    }

    pub fn with_close_block(mut self, include: bool) -> Self {
        self.include_close_block = include;
        self
    }
}

impl<TS, EE, BS, BC, ES, SF> CordialProposer<TS, EE, BS, BC, ES, SF>
where
    TS: TipSelector,
    EE: ExecutionEngine,
    BS: BlockSigner,
    BC: BlockBroadcaster,
    ES: EvidenceSource,
    SF: SlashDeployFormatter<NodeId, Block, BlockIdentity>,
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

        let mut system_deploys =
            slash_system_deploys(&mut self.evidence_source, &self.slash_formatter)?;
        if self.include_close_block {
            system_deploys.push(SystemDeployRequest::CloseBlock);
        }

        let deploys_in_scope = compute_deploys_in_scope(
            blocklace,
            &predecessors,
            block_number,
            self.deploy_pool_config.deploy_lifespan,
        );

        let selected = deploy_pool.select_for_block(block_number, 0, &deploys_in_scope);

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
