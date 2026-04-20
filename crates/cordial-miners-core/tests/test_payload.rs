use cordial_miners_core::execution::{
    CordialBlockPayload, BlockState, Bond, Deploy, SignedDeploy,
    ProcessedDeploy, RejectedDeploy, RejectReason, ProcessedSystemDeploy,
};
use cordial_miners_core::{Block, BlockContent, BlockIdentity, NodeId};
use std::collections::HashSet;

fn node(id: u8) -> NodeId {
    NodeId(vec![id])
}

fn sample_bonds() -> Vec<Bond> {
    vec![
        Bond { validator: node(1), stake: 100 },
        Bond { validator: node(2), stake: 200 },
    ]
}

fn sample_deploy() -> SignedDeploy {
    SignedDeploy {
        deploy: Deploy {
            term: b"@0!(\"hello\")".to_vec(),
            timestamp: 1700000000000,
            phlo_price: 1,
            phlo_limit: 10000,
            valid_after_block_number: 0,
            shard_id: "root".to_string(),
        },
        deployer: vec![0x01; 32],
        signature: vec![0xab; 64],
    }
}

// ── Serialization roundtrip ──

#[test]
fn genesis_payload_roundtrips() {
    let payload = CordialBlockPayload::genesis(sample_bonds());
    let bytes = payload.to_bytes();
    let decoded = CordialBlockPayload::from_bytes(&bytes).unwrap();
    assert_eq!(payload, decoded);
}

#[test]
fn payload_with_deploys_roundtrips() {
    let payload = CordialBlockPayload {
        state: BlockState {
            pre_state_hash: vec![0x01; 32],
            post_state_hash: vec![0x02; 32],
            bonds: sample_bonds(),
            block_number: 5,
        },
        deploys: vec![ProcessedDeploy {
            deploy: sample_deploy(),
            cost: 500,
            is_failed: false,
        }],
        rejected_deploys: vec![],
        system_deploys: vec![],
    };
    let bytes = payload.to_bytes();
    let decoded = CordialBlockPayload::from_bytes(&bytes).unwrap();
    assert_eq!(payload, decoded);
}

#[test]
fn payload_with_rejected_deploys_roundtrips() {
    let payload = CordialBlockPayload {
        state: BlockState {
            pre_state_hash: vec![],
            post_state_hash: vec![],
            bonds: vec![],
            block_number: 1,
        },
        deploys: vec![],
        rejected_deploys: vec![RejectedDeploy {
            deploy: sample_deploy(),
            reason: RejectReason::Expired,
        }],
        system_deploys: vec![],
    };
    let bytes = payload.to_bytes();
    let decoded = CordialBlockPayload::from_bytes(&bytes).unwrap();
    assert_eq!(payload, decoded);
}

#[test]
fn payload_with_system_deploys_roundtrips() {
    let payload = CordialBlockPayload {
        state: BlockState {
            pre_state_hash: vec![],
            post_state_hash: vec![],
            bonds: sample_bonds(),
            block_number: 10,
        },
        deploys: vec![],
        rejected_deploys: vec![],
        system_deploys: vec![
            ProcessedSystemDeploy::Slash {
                validator: node(3),
                succeeded: true,
            },
            ProcessedSystemDeploy::CloseBlock { succeeded: true },
        ],
    };
    let bytes = payload.to_bytes();
    let decoded = CordialBlockPayload::from_bytes(&bytes).unwrap();
    assert_eq!(payload, decoded);
}

#[test]
fn all_reject_reasons_roundtrip() {
    let reasons = vec![
        RejectReason::InvalidSignature,
        RejectReason::Expired,
        RejectReason::Duplicate,
        RejectReason::InsufficientPhloPrice,
        RejectReason::NotYetValid,
    ];
    for reason in reasons {
        let rejected = RejectedDeploy {
            deploy: sample_deploy(),
            reason: reason.clone(),
        };
        let bytes = bincode::serialize(&rejected).unwrap();
        let decoded: RejectedDeploy = bincode::deserialize(&bytes).unwrap();
        assert_eq!(decoded.reason, reason);
    }
}

// ── Integration with BlockContent ──

#[test]
fn payload_fits_in_block_content() {
    let payload = CordialBlockPayload {
        state: BlockState {
            pre_state_hash: vec![0xaa; 32],
            post_state_hash: vec![0xbb; 32],
            bonds: sample_bonds(),
            block_number: 1,
        },
        deploys: vec![ProcessedDeploy {
            deploy: sample_deploy(),
            cost: 100,
            is_failed: false,
        }],
        rejected_deploys: vec![],
        system_deploys: vec![ProcessedSystemDeploy::CloseBlock { succeeded: true }],
    };

    // Serialize into a block
    let block = Block {
        identity: BlockIdentity {
            content_hash: [0x01; 32],
            creator: node(1),
            signature: vec![],
        },
        content: BlockContent {
            payload: payload.to_bytes(),
            predecessors: HashSet::new(),
        },
    };

    // Deserialize back from the block
    let recovered = CordialBlockPayload::from_bytes(&block.content.payload).unwrap();
    assert_eq!(recovered, payload);
    assert_eq!(recovered.state.block_number, 1);
    assert_eq!(recovered.deploys.len(), 1);
}

#[test]
fn invalid_bytes_returns_error() {
    let bad_bytes = vec![0xff, 0x00, 0x01];
    let result = CordialBlockPayload::from_bytes(&bad_bytes);
    assert!(result.is_err());
}

// ── Bonds map helper ──

#[test]
fn bonds_map_extracts_correctly() {
    let payload = CordialBlockPayload::genesis(sample_bonds());
    let map = payload.bonds_map();
    assert_eq!(map.len(), 2);
    assert_eq!(*map.get(&node(1)).unwrap(), 100);
    assert_eq!(*map.get(&node(2)).unwrap(), 200);
}

#[test]
fn genesis_has_block_number_zero() {
    let payload = CordialBlockPayload::genesis(sample_bonds());
    assert_eq!(payload.state.block_number, 0);
    assert!(payload.deploys.is_empty());
    assert!(payload.rejected_deploys.is_empty());
    assert!(payload.system_deploys.is_empty());
}

// ── Full chain with typed payloads ──

#[test]
fn chain_of_typed_blocks() {
    use cordial_miners_core::blocklace::Blocklace;
    use cordial_miners_core::crypto::hash_content;

    let mut bl = Blocklace::new();
    let bonds = sample_bonds();

    // Genesis block with typed payload
    let genesis_payload = CordialBlockPayload::genesis(bonds.clone());
    let genesis_content = BlockContent {
        payload: genesis_payload.to_bytes(),
        predecessors: HashSet::new(),
    };
    let genesis = Block {
        identity: BlockIdentity {
            content_hash: hash_content(&genesis_content),
            creator: node(1),
            signature: vec![],
        },
        content: genesis_content,
    };
    bl.insert(genesis.clone()).unwrap();

    // Block 1 with a deploy
    let b1_payload = CordialBlockPayload {
        state: BlockState {
            pre_state_hash: vec![0x00; 32],
            post_state_hash: vec![0x01; 32],
            bonds: bonds.clone(),
            block_number: 1,
        },
        deploys: vec![ProcessedDeploy {
            deploy: sample_deploy(),
            cost: 500,
            is_failed: false,
        }],
        rejected_deploys: vec![],
        system_deploys: vec![ProcessedSystemDeploy::CloseBlock { succeeded: true }],
    };
    let b1_content = BlockContent {
        payload: b1_payload.to_bytes(),
        predecessors: [genesis.identity.clone()].iter().cloned().collect(),
    };
    let b1 = Block {
        identity: BlockIdentity {
            content_hash: hash_content(&b1_content),
            creator: node(2),
            signature: vec![],
        },
        content: b1_content,
    };
    bl.insert(b1.clone()).unwrap();

    assert_eq!(bl.dom().len(), 2);
    assert!(bl.is_closed());

    // Verify both payloads can be recovered
    let g_recovered = CordialBlockPayload::from_bytes(
        &bl.get(&genesis.identity).unwrap().content.payload,
    ).unwrap();
    assert_eq!(g_recovered.state.block_number, 0);

    let b1_recovered = CordialBlockPayload::from_bytes(
        &bl.get(&b1.identity).unwrap().content.payload,
    ).unwrap();
    assert_eq!(b1_recovered.state.block_number, 1);
    assert_eq!(b1_recovered.deploys.len(), 1);
    assert_eq!(b1_recovered.deploys[0].cost, 500);
}
