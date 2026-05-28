use std::collections::{HashMap, HashSet};

use cordial_miners_core::Block;
use cordial_miners_core::blocklace::Blocklace;
use cordial_miners_core::consensus::select_predecessors;
use cordial_miners_core::crypto::{CryptoVerifier, hash_content};
use cordial_miners_core::execution::{
    compute_deploys_in_scope, Bond, CordialBlockPayload, Deploy, DeployPool, DeployPoolConfig,
    ExecutionRequest, MockRuntime, RuntimeManager, SignedDeploy,
};
use cordial_miners_core::types::{BlockContent, BlockIdentity, NodeId};

use cordial_f1r3node_adapter::crypto_bridge::{F1r3flyCryptoAdapter, SigAlgorithm};
use cordial_f1r3node_adapter::proposer::{
    CordialProposer, DisseminationTipSelector, RecordingBroadcaster, RuntimeExecutionEngine,
    Secp256k1BlockSigner,
};

struct MockVerifier;

impl CryptoVerifier for MockVerifier {
    type Error = String;

    fn verify_block(
        &self,
        _content: &BlockContent,
        _sig: &[u8],
        _creator: &NodeId,
    ) -> Result<(), Self::Error> {
        Ok(())
    }
}

fn test_signing_key(seed: u8) -> Vec<u8> {
    let mut key = vec![0u8; 32];
    key[0] = seed;
    for (i, item) in key.iter_mut().enumerate().skip(1) {
        *item = ((seed as u16).wrapping_mul(i as u16 + 1)) as u8;
    }
    key
}

fn test_public_key(signing_key: &[u8]) -> Vec<u8> {
    use k256::ecdsa::SigningKey as SecpSigningKey;
    let sk = SecpSigningKey::from_slice(signing_key).expect("valid test signing key");
    sk.verifying_key()
        .to_encoded_point(true)
        .as_bytes()
        .to_vec()
}

fn node(id: u8) -> NodeId {
    NodeId(vec![id; 33])
}

fn bonds(entries: &[(u8, u64)]) -> HashMap<NodeId, u64> {
    entries.iter().map(|(b, s)| (node(*b), *s)).collect()
}

fn simple_payload(block_number: u64, post_byte: u8) -> CordialBlockPayload {
    CordialBlockPayload {
        state: cordial_miners_core::execution::BlockState {
            pre_state_hash: if block_number == 0 {
                vec![]
            } else {
                vec![post_byte.wrapping_sub(1); 32]
            },
            post_state_hash: vec![post_byte; 32],
            bonds: vec![
                Bond {
                    validator: node(1),
                    stake: 100,
                },
                Bond {
                    validator: node(2),
                    stake: 100,
                },
            ],
            block_number,
        },
        deploys: vec![],
        rejected_deploys: vec![],
        system_deploys: vec![],
    }
}

fn make_block(creator: NodeId, payload: CordialBlockPayload, predecessors: HashSet<BlockIdentity>) -> Block {
    let content = BlockContent {
        payload: payload.to_bytes(),
        predecessors,
    };
    Block {
        identity: BlockIdentity {
            content_hash: hash_content(&content),
            creator,
            signature: vec![0xAB; 72],
        },
        content,
    }
}

fn insert(blocklace: &mut Blocklace, block: Block) {
    blocklace
        .insert(block, &MockVerifier)
        .expect("fixture insert");
}

fn make_deploy(sig_byte: u8) -> SignedDeploy {
    SignedDeploy {
        deploy: Deploy {
            term: format!("@0!(\"tx-{sig_byte}\")").into_bytes(),
            timestamp: 1000 + sig_byte as u64,
            phlo_price: 1,
            phlo_limit: 10_000,
            valid_after_block_number: 0,
            shard_id: "root".to_string(),
        },
        deployer: vec![sig_byte; 32],
        signature: vec![sig_byte; 64],
    }
}

fn build_proposer(
    creator: NodeId,
    bonds_map: HashMap<NodeId, u64>,
    runtime: MockRuntime,
    signer: Secp256k1BlockSigner,
    broadcaster: RecordingBroadcaster,
    close_block: bool,
) -> CordialProposer<
    DisseminationTipSelector,
    RuntimeExecutionEngine<MockRuntime>,
    Secp256k1BlockSigner,
    RecordingBroadcaster,
> {
    CordialProposer::new(
        DisseminationTipSelector,
        RuntimeExecutionEngine::new(runtime),
        signer,
        broadcaster,
        creator,
        bonds_map,
        DeployPoolConfig::default(),
    )
    .with_close_block(close_block)
}

#[test]
fn proposer_selects_live_tips_from_blocklace() {
    let bond_map = bonds(&[(1, 100), (2, 100), (3, 100)]);

    let mut blocklace = Blocklace::new();
    let g1 = make_block(node(1), simple_payload(0, 0x01), HashSet::new());
    let g2 = make_block(node(2), simple_payload(0, 0x02), HashSet::new());
    let g3 = make_block(node(3), simple_payload(0, 0x03), HashSet::new());
    insert(&mut blocklace, g1);
    insert(&mut blocklace, g2);
    insert(&mut blocklace, g3);

    let sk = test_signing_key(9);
    let creator = NodeId(test_public_key(&sk));
    let recorder = RecordingBroadcaster::new();
    let mut proposer = build_proposer(
        creator,
        bond_map.clone(),
        MockRuntime::permissive(),
        Secp256k1BlockSigner::new(sk),
        recorder,
        false,
    );

    let block = proposer
        .propose(&blocklace, &DeployPool::new(DeployPoolConfig::default()))
        .expect("propose should succeed");

    let expected_tips = select_predecessors(&blocklace, &bond_map);
    assert_eq!(block.content.predecessors, expected_tips);
    assert_eq!(expected_tips.len(), 3, "one tip per honest validator");
}

#[test]
fn proposer_packages_post_state_hash_from_execution() {
    let bond_map = bonds(&[(1, 100)]);

    let mut blocklace = Blocklace::new();
    let genesis = make_block(node(1), simple_payload(0, 0x10), HashSet::new());
    insert(&mut blocklace, genesis);

    let mut pool = DeployPool::new(DeployPoolConfig::default());
    pool.add(make_deploy(7)).expect("add deploy");

    let sk = test_signing_key(11);
    let creator = NodeId(test_public_key(&sk));
    let recorder = RecordingBroadcaster::new();
    let mut proposer = build_proposer(
        creator,
        bond_map.clone(),
        MockRuntime::new(),
        Secp256k1BlockSigner::new(sk),
        recorder,
        false,
    );

    let block = proposer.propose(&blocklace, &pool).expect("propose");

    let payload = CordialBlockPayload::from_bytes(&block.content.payload).expect("decode payload");
    assert!(!payload.deploys.is_empty(), "deploy should be executed");

    let predecessors = select_predecessors(&blocklace, &bond_map);
    let in_scope = compute_deploys_in_scope(&blocklace, &predecessors, 1, 50);
    let selected_deploys = pool.select_for_block(1, 0, &in_scope).deploys;

    let expected = {
        let mut rt = MockRuntime::new();
        rt.execute_block(ExecutionRequest {
            pre_state_hash: vec![0x10; 32],
            deploys: selected_deploys,
            system_deploys: vec![],
            bonds: vec![Bond {
                validator: node(1),
                stake: 100,
            }],
            block_number: 1,
        })
        .expect("direct execution")
    };

    assert_eq!(payload.state.post_state_hash, expected.post_state_hash);
    assert_eq!(payload.state.block_number, 1);
    assert_eq!(payload.state.pre_state_hash, vec![0x10; 32]);
}

#[test]
fn proposed_block_passes_f1r3fly_crypto_verifier() {
    let bond_map = bonds(&[(1, 100), (2, 100)]);

    let mut blocklace = Blocklace::new();
    insert(
        &mut blocklace,
        make_block(node(1), simple_payload(0, 0x21), HashSet::new()),
    );
    insert(
        &mut blocklace,
        make_block(node(2), simple_payload(0, 0x22), HashSet::new()),
    );

    let sk = test_signing_key(42);
    let pk = test_public_key(&sk);
    let creator = NodeId(pk.clone());

    let recorder = RecordingBroadcaster::new();
    let mut proposer = build_proposer(
        creator.clone(),
        bond_map,
        MockRuntime::permissive(),
        Secp256k1BlockSigner::new(sk),
        recorder,
        false,
    );

    let block = proposer
        .propose(&blocklace, &DeployPool::new(DeployPoolConfig::default()))
        .expect("propose");

    let adapter = F1r3flyCryptoAdapter::new(SigAlgorithm::Secp256k1);
    adapter
        .verify_block(
            &block.content,
            &block.identity.signature,
            &block.identity.creator,
        )
        .expect("signature must verify");

    assert_eq!(block.identity.creator.0, pk);

    blocklace
        .insert(block.clone(), &adapter)
        .expect("blocklace must accept verified block");
}

#[test]
fn proposer_genesis_bonds_are_deterministically_sorted() {
    let bond_map = bonds(&[(3, 100), (1, 50), (2, 75)]);

    let mut proposer = build_proposer(
        node(9),
        bond_map.clone(),
        MockRuntime::new(),
        Secp256k1BlockSigner::new(test_signing_key(5)),
        RecordingBroadcaster::new(),
        false,
    );

    let blocklace = Blocklace::new();
    let block = proposer
        .propose(&blocklace, &DeployPool::new(DeployPoolConfig::default()))
        .expect("propose genesis");

    let payload = CordialBlockPayload::from_bytes(&block.content.payload).expect("decode payload");

    let mut expected: Vec<Bond> = bond_map
        .iter()
        .map(|(validator, stake)| Bond {
            validator: validator.clone(),
            stake: *stake,
        })
        .collect();
    expected.sort_by(|a, b| a.validator.0.cmp(&b.validator.0));

    assert_eq!(payload.state.bonds, expected);
    assert_eq!(payload.state.block_number, 0);
    assert!(payload.state.pre_state_hash.is_empty());
}

#[test]
fn proposer_chain_head_tiebreak_is_deterministic() {
    let bond_map = bonds(&[(1, 100), (2, 100)]);

    let mut blocklace = Blocklace::new();
    let g1 = make_block(node(1), simple_payload(0, 0x10), HashSet::new());
    let g2 = make_block(node(2), simple_payload(0, 0x20), HashSet::new());
    insert(&mut blocklace, g1.clone());
    insert(&mut blocklace, g2.clone());

    let sk = test_signing_key(33);
    let mut proposer = build_proposer(
        NodeId(test_public_key(&sk)),
        bond_map.clone(),
        MockRuntime::permissive(),
        Secp256k1BlockSigner::new(sk),
        RecordingBroadcaster::new(),
        false,
    );

    let expected_tips = select_predecessors(&blocklace, &bond_map);

    let expected_best = expected_tips
        .iter()
        .max_by(|a, b| {
            a.content_hash
                .cmp(&b.content_hash)
                .then_with(|| a.creator.0.cmp(&b.creator.0))
                .then_with(|| a.signature.cmp(&b.signature))
        })
        .expect("expected tips");

    let expected_pre_state_hash = blocklace
        .get(expected_best)
        .map(|block| {
            CordialBlockPayload::from_bytes(&block.content.payload)
                .expect("decode payload")
                .state
                .post_state_hash
        })
        .expect("expected tip payload");

    let block = proposer
        .propose(&blocklace, &DeployPool::new(DeployPoolConfig::default()))
        .expect("propose");

    let payload = CordialBlockPayload::from_bytes(&block.content.payload).expect("decode payload");
    assert_eq!(payload.state.pre_state_hash, expected_pre_state_hash);
    assert_eq!(payload.state.block_number, 1);
}