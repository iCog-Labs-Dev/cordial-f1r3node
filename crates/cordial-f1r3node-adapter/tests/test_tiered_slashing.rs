use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use cordial_f1r3node_adapter::crypto_bridge::{F1r3flyCryptoAdapter, SigAlgorithm};
use cordial_f1r3node_adapter::proposer::{
    BlockSigner, CordialProposer, DisseminationTipSelector, EvidencePoolSource, ExecutionEngine,
    ProposeError, RecordingBroadcaster, Secp256k1BlockSigner,
};
use cordial_f1r3node_adapter::slashing::{
    F1r3SlashDeployFormatter, FormattedSlashEvidence, SlashDeployFormatter,
};
use cordial_f1r3node_adapter::tiered_slashing::{
    ExecutionBatchItem, PreparedExecutionRequest, TieredSlashPolicy,
};
use cordial_miners_core::consensus::{
    CordialEquivocationEvidence, CordialEvidencePool, EvidencePool, InvalidBlock, ValidationConfig,
    record_rejected_equivocation, validate_block, validated_insert,
};
use cordial_miners_core::crypto::CryptoVerifier;
use cordial_miners_core::execution::{
    BlockState, Bond, CordialBlockPayload, Deploy, DeployPool, DeployPoolConfig,
    DeploySignatureAlgorithm, ExecutionRequest, ExecutionResult, MockRuntime,
    ProcessedSystemDeploy, RuntimeError, RuntimeManager, SignedDeploy, SystemDeployRequest,
};
use cordial_miners_core::{Block, BlockContent, BlockIdentity, Blocklace, NodeId};
use cordial_por::PorConfig;

fn node(id: u8) -> NodeId {
    NodeId(vec![id; 33])
}

fn weights(entries: &[(u8, u64)]) -> HashMap<NodeId, u64> {
    entries
        .iter()
        .map(|(id, weight)| (node(*id), *weight))
        .collect()
}

fn evidence(pool: &mut CordialEvidencePool, validator: u8, round: u64, tag: u8) {
    let blocks = [tag, tag + 1].map(|tag| Block {
        identity: BlockIdentity {
            creator: node(validator),
            content_hash: [tag; 32],
            signature: vec![tag; 64],
        },
        content: BlockContent {
            payload: vec![tag],
            predecessors: HashSet::new(),
        },
    });
    assert!(pool.record_equivocation(node(validator), round, blocks));
}

fn policy(round: u64, entries: &[(u8, u64)]) -> TieredSlashPolicy {
    TieredSlashPolicy::new(PorConfig::default())
        .unwrap()
        .with_round_weights(round, weights(entries))
        .unwrap()
}

fn deploy(tag: u8) -> SignedDeploy {
    SignedDeploy {
        deploy: Deploy {
            term: format!("@0!({tag})").into_bytes(),
            timestamp: u64::from(tag),
            phlo_price: 1,
            phlo_limit: 1000,
            valid_after_block_number: 0,
            shard_id: "root".into(),
            expiration_timestamp: None,
        },
        deployer: vec![tag; 33],
        signature: vec![tag; 64],
        signature_algorithm: DeploySignatureAlgorithm::Secp256k1,
    }
}

#[derive(Clone, Default)]
struct Capture {
    prepared: Arc<Mutex<Option<PreparedExecutionRequest>>>,
    legacy: Arc<Mutex<Option<ExecutionRequest>>>,
}

impl ExecutionEngine for Capture {
    fn execute(&mut self, request: ExecutionRequest) -> Result<ExecutionResult, RuntimeError> {
        *self.legacy.lock().unwrap() = Some(request.clone());
        Ok(ExecutionResult {
            post_state_hash: vec![0xaa; 32],
            processed_deploys: vec![],
            rejected_deploys: vec![],
            system_deploys: vec![],
            new_bonds: request.bonds,
        })
    }

    fn execute_prepared(
        &mut self,
        request: PreparedExecutionRequest,
    ) -> Result<ExecutionResult, RuntimeError> {
        *self.prepared.lock().unwrap() = Some(request.clone());
        self.execute(request.into_legacy_request())
    }
}

fn propose<E: ExecutionEngine>(
    pool: &CordialEvidencePool,
    validators: &[u8],
    policy: TieredSlashPolicy,
    deploys: &[SignedDeploy],
    capture: E,
) -> Result<Block, ProposeError> {
    let mut deploy_pool = DeployPool::new(DeployPoolConfig::default());
    for deploy in deploys {
        deploy_pool.add(deploy.clone()).unwrap();
    }
    let mut proposer = CordialProposer::new(
        DisseminationTipSelector,
        capture,
        Secp256k1BlockSigner::new(vec![1; 32]),
        RecordingBroadcaster::new(),
        node(9),
        weights(&[(1, 90), (2, 10)]),
        DeployPoolConfig::default(),
    )
    .with_slashing(
        EvidencePoolSource::new(pool, validators.iter().copied().map(node)),
        F1r3SlashDeployFormatter::new(vec![0xbb; 33]),
    )
    .with_slash_policy(policy);
    proposer.propose_at_time(&Blocklace::new(), &deploy_pool, 100)
}

#[test]
fn round_snapshots_determine_the_tier_including_the_exact_threshold() {
    for (offender_weight, expected) in [(20, 2500), (30, 2500), (31, 10000)] {
        let policy = policy(4, &[(1, offender_weight), (2, 100 - offender_weight)]);
        assert_eq!(policy.penalty_ratio_bps(4, &[node(1)]).unwrap(), expected);
        assert_eq!(
            policy.penalty_ratio_bps(4, &[node(1), node(1)]).unwrap(),
            expected
        );
    }
}

#[test]
fn custom_scale_is_converted_to_basis_points() {
    let policy = TieredSlashPolicy::new(PorConfig::new(1000, 200))
        .unwrap()
        .with_round_weights(4, weights(&[(1, 20), (2, 80)]))
        .unwrap();
    assert_eq!(policy.penalty_ratio_bps(4, &[node(1)]).unwrap(), 2500);
}

#[test]
fn invalid_snapshots_and_offenders_fail_explicitly() {
    assert!(TieredSlashPolicy::new(PorConfig::new(0, 0)).is_err());
    for entries in [vec![], vec![(1, 0)], vec![(1, u64::MAX), (2, 1)]] {
        assert!(
            TieredSlashPolicy::new(PorConfig::default())
                .unwrap()
                .with_round_weights(4, weights(&entries))
                .is_err()
        );
    }
    let policy = policy(4, &[(1, 0), (2, 100)]);
    assert!(policy.penalty_ratio_bps(5, &[node(2)]).is_err());
    assert!(policy.penalty_ratio_bps(4, &[node(3)]).is_err());
    assert!(policy.penalty_ratio_bps(4, &[node(1)]).is_err());
    assert!(policy.penalty_ratio_bps(4, &[]).is_err());
}

#[test]
fn nonrepresentable_basis_point_fractions_are_not_silently_rounded() {
    let mut config = PorConfig::new(3, 0);
    config.correlation_threshold = 1;
    config.base_slash_penalty = 1;
    let policy = TieredSlashPolicy::new(config)
        .unwrap()
        .with_round_weights(4, weights(&[(1, 1), (2, 99)]))
        .unwrap();
    assert!(policy.penalty_ratio_bps(4, &[node(1)]).is_err());
}

#[test]
fn proposer_retains_tiers_by_round_and_prepares_slashes_before_users() {
    let mut pool = CordialEvidencePool::new();
    evidence(&mut pool, 1, 4, 1);
    evidence(&mut pool, 1, 4, 3);
    evidence(&mut pool, 1, 8, 5);
    evidence(&mut pool, 2, 8, 7);
    let policy = policy(4, &[(1, 25), (2, 25), (3, 50)])
        .with_round_weights(8, weights(&[(1, 25), (2, 25), (3, 50)]))
        .unwrap();
    let capture = Capture::default();
    let users = vec![deploy(11), deploy(12)];
    propose(&pool, &[2, 1, 1], policy, &users, capture.clone()).unwrap();

    let guard = capture.prepared.lock().unwrap();
    let prepared = guard.as_ref().unwrap();
    let batch = prepared.batch();
    assert_eq!(batch.len(), 7);
    for (item, expected_round, expected_ratio) in [
        (&batch[0], 4, 2500),
        (&batch[1], 4, 2500),
        (&batch[2], 8, 10000),
        (&batch[3], 8, 10000),
    ] {
        let ExecutionBatchItem::SlashSystemDeploy {
            round, evidence, ..
        } = item
        else {
            panic!("expected a slash request before user deploys");
        };
        assert_eq!(*round, expected_round);
        assert_eq!(evidence.penalty_ratio_bps, expected_ratio);
        assert_eq!(evidence.conflicting_block_bytes.len(), 2);
    }
    for (item, selected) in batch[4..6].iter().zip(&prepared.legacy_request().deploys) {
        assert!(
            matches!(item, ExecutionBatchItem::UserDeploy(d) if d.signature == selected.signature)
        );
    }
    assert!(matches!(&batch[6], ExecutionBatchItem::CloseBlock));
    let legacy = capture.legacy.lock().unwrap();
    let legacy = legacy.as_ref().unwrap();
    assert_eq!(legacy.deploys.len(), 2);
    assert_eq!(
        legacy
            .deploys
            .iter()
            .map(|d| &d.signature)
            .collect::<HashSet<_>>(),
        users.iter().map(|d| &d.signature).collect::<HashSet<_>>()
    );
    assert_eq!(legacy.system_deploys.len(), 5);
    assert!(matches!(
        legacy.system_deploys[0],
        SystemDeployRequest::Slash { .. }
    ));
    assert!(matches!(
        legacy.system_deploys[4],
        SystemDeployRequest::CloseBlock
    ));
}

#[test]
fn proposer_uses_each_evidence_rounds_own_weight_snapshot() {
    let mut pool = CordialEvidencePool::new();
    evidence(&mut pool, 1, 4, 1);
    evidence(&mut pool, 1, 8, 3);
    let policy = policy(4, &[(1, 20), (2, 80)])
        .with_round_weights(8, weights(&[(1, 40), (2, 60)]))
        .unwrap();
    let capture = Capture::default();
    propose(&pool, &[1], policy, &[], capture.clone()).unwrap();

    let guard = capture.prepared.lock().unwrap();
    let batch = guard.as_ref().unwrap().batch();
    assert_eq!(batch.len(), 3);
    for (item, expected_round, expected_ratio) in [(&batch[0], 4, 2500), (&batch[1], 8, 10000)] {
        let ExecutionBatchItem::SlashSystemDeploy {
            validator,
            round,
            evidence,
        } = item
        else {
            panic!("expected a slash request for each evidence round");
        };
        assert_eq!(validator, &node(1));
        assert_eq!(*round, expected_round);
        assert_eq!(evidence.penalty_ratio_bps, expected_ratio);
    }
    assert!(matches!(&batch[2], ExecutionBatchItem::CloseBlock));
}

#[test]
fn tiered_preparation_can_omit_close_block_and_preserve_selected_user_order() {
    let mut pool = CordialEvidencePool::new();
    evidence(&mut pool, 1, 4, 1);
    let mut deploy_pool = DeployPool::new(DeployPoolConfig::default());
    for user in [deploy(11), deploy(12)] {
        deploy_pool.add(user).unwrap();
    }
    let selected_signatures = deploy_pool
        .select_for_block(0, 100, &HashSet::new())
        .deploys
        .into_iter()
        .map(|deploy| deploy.signature)
        .collect::<Vec<_>>();
    assert_eq!(selected_signatures.len(), 2);

    let capture = Capture::default();
    let mut proposer = CordialProposer::new(
        DisseminationTipSelector,
        capture.clone(),
        Secp256k1BlockSigner::new(vec![1; 32]),
        RecordingBroadcaster::new(),
        node(9),
        weights(&[(1, 90), (2, 10)]),
        DeployPoolConfig::default(),
    )
    .with_close_block(false)
    .with_slashing(
        EvidencePoolSource::new(&pool, [node(1)]),
        F1r3SlashDeployFormatter::new(vec![0xbb; 33]),
    )
    .with_slash_policy(policy(4, &[(1, 20), (2, 80)]));
    proposer
        .propose_at_time(&Blocklace::new(), &deploy_pool, 100)
        .unwrap();

    let guard = capture.prepared.lock().unwrap();
    let prepared = guard.as_ref().unwrap();
    let batch = prepared.batch();
    assert_eq!(batch.len(), 3);
    assert!(matches!(
        &batch[0],
        ExecutionBatchItem::SlashSystemDeploy { evidence, .. }
            if evidence.penalty_ratio_bps == 2500
    ));
    assert!(
        batch
            .iter()
            .all(|item| !matches!(item, ExecutionBatchItem::CloseBlock))
    );
    let prepared_signatures = batch[1..]
        .iter()
        .map(|item| match item {
            ExecutionBatchItem::UserDeploy(deploy) => deploy.signature.clone(),
            _ => panic!("expected only user deploys after the slash request"),
        })
        .collect::<Vec<_>>();
    assert_eq!(prepared_signatures, selected_signatures);

    let legacy = capture.legacy.lock().unwrap();
    for request in [prepared.legacy_request(), legacy.as_ref().unwrap()] {
        assert_eq!(request.system_deploys.len(), 1);
        assert!(matches!(
            request.system_deploys[0],
            SystemDeployRequest::Slash { .. }
        ));
        assert!(
            request
                .system_deploys
                .iter()
                .all(|item| !matches!(item, SystemDeployRequest::CloseBlock))
        );
        assert_eq!(
            request
                .deploys
                .iter()
                .map(|deploy| deploy.signature.clone())
                .collect::<Vec<_>>(),
            selected_signatures
        );
    }
}

#[test]
fn correlation_includes_all_round_offenders_even_when_only_one_proof_is_selected() {
    let mut pool = CordialEvidencePool::new();
    evidence(&mut pool, 1, 4, 1);
    evidence(&mut pool, 2, 4, 3);
    let capture = Capture::default();
    propose(
        &pool,
        &[1],
        policy(4, &[(1, 20), (2, 20), (3, 60)]),
        &[],
        capture.clone(),
    )
    .unwrap();
    let guard = capture.prepared.lock().unwrap();
    let batch = guard.as_ref().unwrap().batch();
    assert!(matches!(
        &batch[0],
        ExecutionBatchItem::SlashSystemDeploy { evidence, .. }
            if evidence.penalty_ratio_bps == 10000
    ));
}

#[test]
fn missing_round_weights_abort_before_execution() {
    let mut pool = CordialEvidencePool::new();
    evidence(&mut pool, 1, 4, 1);
    let capture = Capture::default();
    let result = propose(
        &pool,
        &[1],
        policy(5, &[(1, 20), (2, 80)]),
        &[],
        capture.clone(),
    );
    assert!(matches!(result, Err(ProposeError::SlashFormat(_))));
    assert!(capture.prepared.lock().unwrap().is_none());
    assert!(capture.legacy.lock().unwrap().is_none());
}

#[test]
fn no_evidence_preserves_user_order_without_needing_a_round_snapshot() {
    let capture = Capture::default();
    let users = vec![deploy(11), deploy(12)];
    propose(
        &CordialEvidencePool::new(),
        &[],
        TieredSlashPolicy::new(PorConfig::default()).unwrap(),
        &users,
        capture.clone(),
    )
    .unwrap();
    let guard = capture.prepared.lock().unwrap();
    let prepared = guard.as_ref().unwrap();
    let batch = prepared.batch();
    assert_eq!(batch.len(), 3);
    let selected = &prepared.legacy_request().deploys;
    assert_eq!(selected.len(), users.len());
    assert_eq!(
        selected
            .iter()
            .map(|d| &d.signature)
            .collect::<HashSet<_>>(),
        users.iter().map(|d| &d.signature).collect::<HashSet<_>>()
    );
    for (item, selected) in batch[..2].iter().zip(selected) {
        assert!(
            matches!(item, ExecutionBatchItem::UserDeploy(d) if d.signature == selected.signature)
        );
    }
    assert!(matches!(&batch[2], ExecutionBatchItem::CloseBlock));
}

#[test]
fn a_round_snapshot_cannot_be_silently_replaced() {
    assert!(
        policy(4, &[(1, 20), (2, 80)])
            .with_round_weights(4, weights(&[(1, 80), (2, 20)]))
            .is_err()
    );
}

struct LegacyCapture(Capture);

impl ExecutionEngine for LegacyCapture {
    fn execute(&mut self, request: ExecutionRequest) -> Result<ExecutionResult, RuntimeError> {
        self.0.execute(request)
    }
}

#[test]
fn an_existing_execution_engine_accepts_tiered_preparation_without_an_override() {
    let mut pool = CordialEvidencePool::new();
    evidence(&mut pool, 1, 4, 1);
    let capture = Capture::default();
    propose(
        &pool,
        &[1],
        policy(4, &[(1, 20), (2, 80)]),
        &[deploy(11)],
        LegacyCapture(capture.clone()),
    )
    .unwrap();
    assert!(capture.prepared.lock().unwrap().is_none());
    let guard = capture.legacy.lock().unwrap();
    let request = guard.as_ref().unwrap();
    assert_eq!(request.deploys.len(), 1);
    assert!(matches!(
        request.system_deploys[0],
        SystemDeployRequest::Slash { .. }
    ));
    assert!(matches!(
        request.system_deploys[1],
        SystemDeployRequest::CloseBlock
    ));
}

#[derive(Clone, Copy)]
enum Corruption {
    Ratio,
    Count,
    Hash,
}

struct CorruptFormatter(Corruption);

impl SlashDeployFormatter<NodeId, Block, BlockIdentity> for CorruptFormatter {
    fn to_slash_system_deploys(
        &self,
        evidence: &[CordialEquivocationEvidence],
    ) -> anyhow::Result<Vec<Vec<u8>>> {
        F1r3SlashDeployFormatter::new(vec![0xbb; 33]).to_slash_system_deploys(evidence)
    }

    fn to_tiered_slash_deploys(
        &self,
        evidence: &[CordialEquivocationEvidence],
        ratio: u32,
    ) -> anyhow::Result<Vec<FormattedSlashEvidence>> {
        let formatter = F1r3SlashDeployFormatter::new(vec![0xbb; 33]);
        let mut formatted = formatter.to_tiered_slash_deploys(evidence, ratio)?;
        match self.0 {
            Corruption::Ratio => formatted[0].penalty_ratio_bps = 10000,
            Corruption::Count => formatted.clear(),
            Corruption::Hash => {
                let mut unrelated = evidence.to_vec();
                for block in &mut unrelated[0].blocks {
                    block.identity.content_hash = [0xfe; 32];
                }
                formatted[0].slash_system_deploy =
                    formatter.to_slash_system_deploys(&unrelated)?[0].clone();
            }
        }
        Ok(formatted)
    }
}

#[test]
fn a_formatter_cannot_substitute_the_tier_count_or_evidence_hash() {
    let mut pool = CordialEvidencePool::new();
    evidence(&mut pool, 1, 4, 1);
    for corruption in [Corruption::Ratio, Corruption::Count, Corruption::Hash] {
        let capture = Capture::default();
        let mut proposer = CordialProposer::new(
            DisseminationTipSelector,
            capture.clone(),
            Secp256k1BlockSigner::new(vec![1; 32]),
            RecordingBroadcaster::new(),
            node(9),
            weights(&[(1, 90), (2, 10)]),
            DeployPoolConfig::default(),
        )
        .with_slashing(
            EvidencePoolSource::new(&pool, [node(1)]),
            CorruptFormatter(corruption),
        )
        .with_slash_policy(policy(4, &[(1, 20), (2, 80)]));
        assert!(matches!(
            proposer.propose_at_time(
                &Blocklace::new(),
                &DeployPool::new(DeployPoolConfig::default()),
                100
            ),
            Err(ProposeError::SlashFormat(_))
        ));
        assert!(capture.prepared.lock().unwrap().is_none());
        assert!(capture.legacy.lock().unwrap().is_none());
    }
}

struct CapturingMockRuntime {
    capture: Capture,
    runtime: MockRuntime,
}

impl ExecutionEngine for CapturingMockRuntime {
    fn execute(&mut self, request: ExecutionRequest) -> Result<ExecutionResult, RuntimeError> {
        *self.capture.legacy.lock().unwrap() = Some(request.clone());
        self.runtime.execute_block(request)
    }

    fn execute_prepared(
        &mut self,
        request: PreparedExecutionRequest,
    ) -> Result<ExecutionResult, RuntimeError> {
        *self.capture.prepared.lock().unwrap() = Some(request.clone());
        self.execute(request.into_legacy_request())
    }
}

#[test]
fn rejected_equivocation_reaches_a_signed_broadcast_and_peer_validation() {
    let offender_key = vec![1; 32];
    let proposer_key = vec![2; 32];
    let identity = |key: &[u8]| {
        let key = k256::ecdsa::SigningKey::from_slice(key).unwrap();
        NodeId(
            key.verifying_key()
                .to_encoded_point(true)
                .as_bytes()
                .to_vec(),
        )
    };
    let offender = identity(&offender_key);
    let creator = identity(&proposer_key);
    let verifier = F1r3flyCryptoAdapter::new(SigAlgorithm::Secp256k1);

    for (offender_weight, expected_ratio) in [(20, 2500), (40, 10000)] {
        let bonds = HashMap::from([
            (offender.clone(), offender_weight),
            (creator.clone(), 100 - offender_weight),
        ]);
        let genesis = |tag| {
            let payload = CordialBlockPayload {
                state: BlockState {
                    pre_state_hash: vec![],
                    post_state_hash: vec![tag; 32],
                    bonds: bonds
                        .iter()
                        .map(|(validator, weight)| Bond {
                            validator: validator.clone(),
                            stake: *weight,
                        })
                        .collect(),
                    block_number: 0,
                },
                deploys: vec![],
                rejected_deploys: vec![],
                system_deploys: vec![],
            };
            let content = BlockContent {
                payload: payload.to_bytes(),
                predecessors: HashSet::new(),
            };
            let identity = Secp256k1BlockSigner::new(offender_key.clone())
                .sign_block(&content, &offender)
                .unwrap();
            Block { identity, content }
        };
        let left = genesis(1);
        let right = genesis(2);
        let mut peer = Blocklace::new();
        peer.insert(left.clone(), &verifier).unwrap();
        let config = ValidationConfig::default();
        let rejected = validate_block(&right, &peer, &bonds, &config);
        assert!(
            rejected
                .errors()
                .iter()
                .any(|error| matches!(error, InvalidBlock::Equivocation { .. }))
        );
        let mut pool = CordialEvidencePool::new();
        assert!(record_rejected_equivocation(
            &right,
            rejected.errors(),
            &peer,
            &mut pool,
        ));

        let mut deploy_pool = DeployPool::new(DeployPoolConfig::default());
        for user in [deploy(11), deploy(12)] {
            deploy_pool.add(user).unwrap();
        }
        let capture = Capture::default();
        let broadcaster = RecordingBroadcaster::new();
        let broadcasted = Arc::clone(&broadcaster.blocks);
        let policy = TieredSlashPolicy::new(PorConfig::default())
            .unwrap()
            .with_round_weights(0, bonds.clone())
            .unwrap();
        let mut proposer = CordialProposer::new(
            DisseminationTipSelector,
            CapturingMockRuntime {
                capture: capture.clone(),
                runtime: MockRuntime::permissive(),
            },
            Secp256k1BlockSigner::new(proposer_key.clone()),
            broadcaster,
            creator.clone(),
            bonds.clone(),
            DeployPoolConfig::default(),
        )
        .with_slashing(
            EvidencePoolSource::new(&pool, [offender.clone()]),
            F1r3SlashDeployFormatter::new(creator.0.clone()),
        )
        .with_slash_policy(policy);

        let block = proposer.propose_at_time(&peer, &deploy_pool, 100).unwrap();
        let guard = capture.prepared.lock().unwrap();
        let prepared = guard.as_ref().unwrap();
        let batch = prepared.batch();
        assert_eq!(batch.len(), 4);
        let ExecutionBatchItem::SlashSystemDeploy {
            validator,
            round,
            evidence,
        } = &batch[0]
        else {
            panic!("expected slash before user deploys");
        };
        assert_eq!(validator, &offender);
        assert_eq!(*round, 0);
        assert_eq!(evidence.penalty_ratio_bps, expected_ratio);
        for bytes in &evidence.conflicting_block_bytes {
            let proof: Block = bincode::deserialize(bytes).unwrap();
            verifier
                .verify_block(
                    &proof.content,
                    &proof.identity.signature,
                    &proof.identity.creator,
                )
                .unwrap();
        }
        assert!(matches!(&batch[1], ExecutionBatchItem::UserDeploy(_)));
        assert!(matches!(&batch[2], ExecutionBatchItem::UserDeploy(_)));
        assert!(matches!(&batch[3], ExecutionBatchItem::CloseBlock));

        let payload = CordialBlockPayload::from_bytes(&block.content.payload).unwrap();
        assert_eq!(payload.state.pre_state_hash, vec![1; 32]);
        assert_eq!(payload.state.block_number, 1);
        assert_eq!(payload.deploys.len(), 2);
        assert!(payload.rejected_deploys.is_empty());
        assert!(matches!(
            &payload.system_deploys[0],
            ProcessedSystemDeploy::Slash { validator, succeeded: true } if validator == &offender
        ));
        assert!(matches!(
            &payload.system_deploys[1],
            ProcessedSystemDeploy::CloseBlock { succeeded: true }
        ));
        // The existing mock applies legacy full removal for both metadata tiers.
        // This test verifies compatibility, not percentage enforcement by a host.
        assert_eq!(
            proposer.bonds(),
            &HashMap::from([(creator.clone(), 100 - offender_weight)])
        );
        assert_eq!(*broadcasted.lock().unwrap(), vec![block.clone()]);
        verifier
            .verify_block(
                &block.content,
                &block.identity.signature,
                &block.identity.creator,
            )
            .unwrap();
        assert!(validated_insert(block.clone(), &mut peer, &bonds, &config).is_valid());
        assert!(peer.get(&block.identity).is_some());
    }
}
