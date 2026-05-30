//! Cordial Miners proposer pipeline.
//!
//! The proposer is the outbound side of consensus: it selects parents from the
//! blocklace, checks whether the core has retained equivocation evidence,
//! formats any slash evidence into f1r3node system deploy bytes, prepends those
//! system deploys ahead of user deploys, executes the batch, signs the block,
//! and broadcasts it.

use std::collections::{BTreeSet, HashSet};

use anyhow::Result;
use cordial_miners_core::block::Block;
use cordial_miners_core::consensus::{CordialEquivocationEvidence, EvidencePool};
use cordial_miners_core::execution::{CordialBlockPayload, SignedDeploy};
use cordial_miners_core::types::{BlockContent, BlockIdentity, NodeId};

use crate::slashing::SlashDeployFormatter;

/// Selects the parent/tip set to reference from the next Cordial block.
pub trait TipSelector {
    fn select_tips(&mut self) -> Result<HashSet<BlockIdentity>>;
}

/// Exposes equivocation evidence retained by the pure Cordial core.
pub trait EvidenceSource {
    fn pending_evidence(&mut self) -> Vec<CordialEquivocationEvidence>;
}

/// Pulls user deploys from the host node's deploy pool.
pub trait DeploySource {
    fn pending_deploys(&mut self) -> Result<Vec<SignedDeploy>>;
}

/// Executes an ordered proposer batch and returns the block payload.
pub trait ExecutionEngine {
    fn execute(
        &mut self,
        parents: &HashSet<BlockIdentity>,
        batch: Vec<ExecutionBatchItem>,
    ) -> Result<CordialBlockPayload>;
}

/// Signs assembled block content into a Cordial block.
pub trait BlockSigner {
    fn sign(&self, content: BlockContent) -> Result<Block>;
}

/// Broadcasts a newly signed block to peers.
pub trait BlockBroadcaster {
    fn broadcast(&mut self, block: &Block) -> Result<()>;
}

/// One item in the exact order sent to execution.
#[derive(Debug, Clone, PartialEq)]
pub enum ExecutionBatchItem {
    /// f1r3node slash system deploy bytes produced by [`SlashDeployFormatter`].
    SlashSystemDeploy(Vec<u8>),
    /// A normal user deploy from the mempool.
    UserDeploy(SignedDeploy),
}

/// Summary returned after a successful proposal.
#[derive(Debug, Clone, PartialEq)]
pub struct ProposedBlock {
    pub block: Block,
    pub slash_deploy_count: usize,
    pub user_deploy_count: usize,
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

/// Cordial Miners block proposer with host-specific dependencies injected.
pub struct CordialProposer<T, E, D, F, X, S, B> {
    tip_selector: T,
    evidence_source: E,
    deploy_source: D,
    slash_formatter: F,
    execution_engine: X,
    signer: S,
    broadcaster: B,
}

impl<T, E, D, F, X, S, B> CordialProposer<T, E, D, F, X, S, B> {
    pub fn new(
        tip_selector: T,
        evidence_source: E,
        deploy_source: D,
        slash_formatter: F,
        execution_engine: X,
        signer: S,
        broadcaster: B,
    ) -> Self {
        Self {
            tip_selector,
            evidence_source,
            deploy_source,
            slash_formatter,
            execution_engine,
            signer,
            broadcaster,
        }
    }
}

impl<T, E, D, F, X, S, B> CordialProposer<T, E, D, F, X, S, B>
where
    T: TipSelector,
    E: EvidenceSource,
    D: DeploySource,
    F: SlashDeployFormatter<NodeId, Block, BlockIdentity>,
    X: ExecutionEngine,
    S: BlockSigner,
    B: BlockBroadcaster,
{
    /// Create, sign, and broadcast one Cordial block.
    ///
    /// Slashing evidence is intentionally queried and formatted before the
    /// deploy source is touched. That makes slash system deploys the first
    /// items in the execution batch and prevents user deploys from taking
    /// priority over consensus safety penalties.
    pub fn propose(&mut self) -> Result<ProposedBlock> {
        let parents = self.tip_selector.select_tips()?;
        let evidence = self.evidence_source.pending_evidence();
        let slash_system_deploys = self.slash_formatter.to_slash_system_deploys(&evidence)?;
        let user_deploys = self.deploy_source.pending_deploys()?;

        let slash_deploy_count = slash_system_deploys.len();
        let user_deploy_count = user_deploys.len();
        let batch = execution_batch(slash_system_deploys, user_deploys);
        let payload = self.execution_engine.execute(&parents, batch)?;
        let content = BlockContent {
            payload: payload.to_bytes(),
            predecessors: parents,
        };
        let block = self.signer.sign(content)?;

        self.broadcaster.broadcast(&block)?;

        Ok(ProposedBlock {
            block,
            slash_deploy_count,
            user_deploy_count,
        })
    }
}

/// Build the exact ordered batch sent to execution.
///
/// Slash system deploys are always first. User deploy order is preserved after
/// the system deploy prefix.
pub fn execution_batch(
    slash_system_deploys: Vec<Vec<u8>>,
    user_deploys: Vec<SignedDeploy>,
) -> Vec<ExecutionBatchItem> {
    slash_system_deploys
        .into_iter()
        .map(ExecutionBatchItem::SlashSystemDeploy)
        .chain(user_deploys.into_iter().map(ExecutionBatchItem::UserDeploy))
        .collect()
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use cordial_miners_core::consensus::{CordialEvidencePool, EquivocationEvidence, EvidencePool};
    use cordial_miners_core::execution::{
        BlockState, Bond, CordialBlockPayload, Deploy, ProcessedSystemDeploy, SignedDeploy,
    };

    use super::*;

    type CallLog = Rc<RefCell<Vec<&'static str>>>;

    fn node(byte: u8) -> NodeId {
        NodeId(vec![byte; 33])
    }

    fn identity(creator: NodeId, tag: u8) -> BlockIdentity {
        let mut content_hash = [0u8; 32];
        content_hash[0] = tag;
        BlockIdentity {
            content_hash,
            creator,
            signature: vec![tag; 64],
        }
    }

    fn block(creator: NodeId, tag: u8) -> Block {
        Block {
            identity: identity(creator, tag),
            content: BlockContent {
                payload: vec![tag],
                predecessors: HashSet::new(),
            },
        }
    }

    fn signed_deploy(tag: u8) -> SignedDeploy {
        SignedDeploy {
            deploy: Deploy {
                term: vec![tag],
                timestamp: tag as u64,
                phlo_price: 1,
                phlo_limit: 1_000,
                valid_after_block_number: 0,
                shard_id: "root".to_string(),
            },
            deployer: vec![tag; 33],
            signature: vec![tag; 64],
        }
    }

    fn payload(system_deploys: Vec<ProcessedSystemDeploy>) -> CordialBlockPayload {
        CordialBlockPayload {
            state: BlockState {
                pre_state_hash: vec![0x01],
                post_state_hash: vec![0x02],
                bonds: vec![Bond {
                    validator: node(1),
                    stake: 1,
                }],
                block_number: 1,
            },
            deploys: vec![],
            rejected_deploys: vec![],
            system_deploys,
        }
    }

    struct MockTipSelector {
        log: CallLog,
        tips: HashSet<BlockIdentity>,
    }

    impl TipSelector for MockTipSelector {
        fn select_tips(&mut self) -> Result<HashSet<BlockIdentity>> {
            self.log.borrow_mut().push("select_tips");
            Ok(self.tips.clone())
        }
    }

    struct MockEvidenceSource {
        log: CallLog,
        evidence: Vec<CordialEquivocationEvidence>,
    }

    impl EvidenceSource for MockEvidenceSource {
        fn pending_evidence(&mut self) -> Vec<CordialEquivocationEvidence> {
            self.log.borrow_mut().push("query_evidence");
            self.evidence.clone()
        }
    }

    struct MockDeploySource {
        log: CallLog,
        deploys: Vec<SignedDeploy>,
    }

    impl DeploySource for MockDeploySource {
        fn pending_deploys(&mut self) -> Result<Vec<SignedDeploy>> {
            self.log.borrow_mut().push("pull_user_deploys");
            Ok(self.deploys.clone())
        }
    }

    struct MockSlashFormatter {
        log: CallLog,
        formatted: Vec<Vec<u8>>,
        observed_evidence: Rc<RefCell<Vec<CordialEquivocationEvidence>>>,
    }

    impl SlashDeployFormatter<NodeId, Block, BlockIdentity> for MockSlashFormatter {
        fn to_slash_system_deploys(
            &self,
            evidence: &[EquivocationEvidence<NodeId, Block, BlockIdentity>],
        ) -> Result<Vec<Vec<u8>>> {
            self.log.borrow_mut().push("format_slash_deploys");
            *self.observed_evidence.borrow_mut() = evidence.to_vec();
            Ok(self.formatted.clone())
        }
    }

    struct MockExecutionEngine {
        log: CallLog,
        captured_batch: Rc<RefCell<Option<Vec<ExecutionBatchItem>>>>,
        output: CordialBlockPayload,
    }

    impl ExecutionEngine for MockExecutionEngine {
        fn execute(
            &mut self,
            _parents: &HashSet<BlockIdentity>,
            batch: Vec<ExecutionBatchItem>,
        ) -> Result<CordialBlockPayload> {
            self.log.borrow_mut().push("execute_batch");
            *self.captured_batch.borrow_mut() = Some(batch);
            Ok(self.output.clone())
        }
    }

    struct MockSigner {
        log: CallLog,
        creator: NodeId,
    }

    impl BlockSigner for MockSigner {
        fn sign(&self, content: BlockContent) -> Result<Block> {
            self.log.borrow_mut().push("sign_block");
            Ok(Block {
                identity: identity(self.creator.clone(), 0x99),
                content,
            })
        }
    }

    struct MockBroadcaster {
        log: CallLog,
        broadcasted: Rc<RefCell<Vec<Block>>>,
    }

    impl BlockBroadcaster for MockBroadcaster {
        fn broadcast(&mut self, block: &Block) -> Result<()> {
            self.log.borrow_mut().push("broadcast_block");
            self.broadcasted.borrow_mut().push(block.clone());
            Ok(())
        }
    }

    #[test]
    fn proposer_prepends_slash_deploys_before_user_deploys() {
        let log = Rc::new(RefCell::new(Vec::new()));
        let validator = node(7);
        let evidence = vec![EquivocationEvidence::new(
            validator.clone(),
            2,
            vec![block(validator.clone(), 0x01), block(validator, 0x02)],
        )];
        let slash_bytes = vec![0xfa, 0xce];
        let user_deploy = signed_deploy(0x42);
        let captured_batch = Rc::new(RefCell::new(None));
        let observed_evidence = Rc::new(RefCell::new(Vec::new()));
        let broadcasted = Rc::new(RefCell::new(Vec::new()));

        let mut proposer = CordialProposer::new(
            MockTipSelector {
                log: Rc::clone(&log),
                tips: HashSet::from([identity(node(1), 0x10)]),
            },
            MockEvidenceSource {
                log: Rc::clone(&log),
                evidence: evidence.clone(),
            },
            MockDeploySource {
                log: Rc::clone(&log),
                deploys: vec![user_deploy.clone()],
            },
            MockSlashFormatter {
                log: Rc::clone(&log),
                formatted: vec![slash_bytes.clone()],
                observed_evidence: Rc::clone(&observed_evidence),
            },
            MockExecutionEngine {
                log: Rc::clone(&log),
                captured_batch: Rc::clone(&captured_batch),
                output: payload(vec![]),
            },
            MockSigner {
                log: Rc::clone(&log),
                creator: node(8),
            },
            MockBroadcaster {
                log: Rc::clone(&log),
                broadcasted: Rc::clone(&broadcasted),
            },
        );

        let proposed = proposer.propose().unwrap();

        assert_eq!(proposed.slash_deploy_count, 1);
        assert_eq!(proposed.user_deploy_count, 1);
        assert_eq!(broadcasted.borrow().len(), 1);
        assert_eq!(*observed_evidence.borrow(), evidence);

        let batch = captured_batch.borrow().clone().unwrap();
        assert_eq!(
            batch,
            vec![
                ExecutionBatchItem::SlashSystemDeploy(slash_bytes),
                ExecutionBatchItem::UserDeploy(user_deploy),
            ]
        );
        assert_eq!(
            *log.borrow(),
            vec![
                "select_tips",
                "query_evidence",
                "format_slash_deploys",
                "pull_user_deploys",
                "execute_batch",
                "sign_block",
                "broadcast_block",
            ]
        );
    }

    #[test]
    fn core_evidence_pool_and_adapter_formatter_remain_isolated_by_traits() {
        let log = Rc::new(RefCell::new(Vec::new()));
        let validator = node(3);
        let left = block(validator.clone(), 0x11);
        let right = block(validator.clone(), 0x22);
        let mut pool = CordialEvidencePool::new();
        assert!(pool.record_equivocation(validator.clone(), 4, vec![left.clone(), right.clone()]));

        let slash_bytes = vec![0xab, 0xcd, 0xef];
        let captured_batch = Rc::new(RefCell::new(None));
        let observed_evidence = Rc::new(RefCell::new(Vec::new()));
        let broadcasted = Rc::new(RefCell::new(Vec::new()));

        let mut proposer = CordialProposer::new(
            MockTipSelector {
                log: Rc::clone(&log),
                tips: HashSet::new(),
            },
            EvidencePoolSource::new(&pool, vec![validator.clone(), validator.clone()]),
            MockDeploySource {
                log: Rc::clone(&log),
                deploys: vec![],
            },
            MockSlashFormatter {
                log: Rc::clone(&log),
                formatted: vec![slash_bytes.clone()],
                observed_evidence: Rc::clone(&observed_evidence),
            },
            MockExecutionEngine {
                log: Rc::clone(&log),
                captured_batch: Rc::clone(&captured_batch),
                output: payload(vec![ProcessedSystemDeploy::Slash {
                    validator: validator.clone(),
                    succeeded: true,
                }]),
            },
            MockSigner {
                log: Rc::clone(&log),
                creator: node(9),
            },
            MockBroadcaster {
                log: Rc::clone(&log),
                broadcasted: Rc::clone(&broadcasted),
            },
        );

        proposer.propose().unwrap();

        let observed = observed_evidence.borrow();
        assert_eq!(observed.len(), 1);
        assert_eq!(observed[0].validator, validator);
        assert_eq!(observed[0].round, 4);
        assert_eq!(observed[0].blocks, vec![left, right]);

        let batch = captured_batch.borrow().clone().unwrap();
        assert_eq!(
            batch.first(),
            Some(&ExecutionBatchItem::SlashSystemDeploy(slash_bytes))
        );
        assert_eq!(
            *log.borrow(),
            vec![
                "select_tips",
                "format_slash_deploys",
                "pull_user_deploys",
                "execute_batch",
                "sign_block",
                "broadcast_block",
            ]
        );
    }
}
