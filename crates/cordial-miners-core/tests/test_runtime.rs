use cordial_miners_core::execution::{
    Bond, Deploy, ExecutionRequest, ExecutionResult, MockRuntime, ProcessedSystemDeploy,
    RejectReason, RuntimeError, RuntimeManager, SignedDeploy, SystemDeployRequest,
};
use cordial_miners_core::types::NodeId;

// ── Helpers ──

fn node(id: u8) -> NodeId {
    NodeId(vec![id])
}

fn make_deploy(sig_byte: u8, term: &[u8], phlo_limit: u64) -> SignedDeploy {
    SignedDeploy {
        deploy: Deploy {
            term: term.to_vec(),
            timestamp: 1000 + sig_byte as u64,
            phlo_price: 1,
            phlo_limit,
            valid_after_block_number: 0,
            shard_id: "root".to_string(),
        },
        deployer: vec![sig_byte; 32],
        signature: vec![sig_byte; 64],
    }
}

fn empty_request(block_number: u64) -> ExecutionRequest {
    ExecutionRequest {
        pre_state_hash: vec![],
        deploys: vec![],
        system_deploys: vec![],
        bonds: vec![Bond {
            validator: node(1),
            stake: 100,
        }],
        block_number,
    }
}

// ── Basic execution ──

#[test]
fn genesis_block_executes_from_empty_pre_state() {
    let mut rt = MockRuntime::new();
    let result = rt.execute_block(empty_request(0)).unwrap();
    assert!(!result.post_state_hash.is_empty());
    assert!(result.processed_deploys.is_empty());
    assert!(result.rejected_deploys.is_empty());
}

#[test]
fn unknown_pre_state_is_rejected() {
    let mut rt = MockRuntime::new();
    let mut req = empty_request(1);
    req.pre_state_hash = vec![0xde, 0xad]; // not the genesis pre-state
    let err = rt.execute_block(req).unwrap_err();
    assert_eq!(err, RuntimeError::UnknownPreState);
}

#[test]
fn permissive_runtime_accepts_any_pre_state() {
    let mut rt = MockRuntime::permissive();
    let mut req = empty_request(5);
    req.pre_state_hash = vec![0xca, 0xfe];
    assert!(rt.execute_block(req).is_ok());
}

// ── Determinism ──

#[test]
fn same_input_produces_same_post_state_hash() {
    let mut rt1 = MockRuntime::new();
    let mut rt2 = MockRuntime::new();

    let req = ExecutionRequest {
        pre_state_hash: vec![],
        deploys: vec![make_deploy(1, b"hello", 1000)],
        system_deploys: vec![],
        bonds: vec![Bond {
            validator: node(1),
            stake: 100,
        }],
        block_number: 1,
    };

    let r1 = rt1.execute_block(req.clone()).unwrap();
    let r2 = rt2.execute_block(req).unwrap();
    assert_eq!(r1.post_state_hash, r2.post_state_hash);
}

#[test]
fn different_deploys_produce_different_post_state_hashes() {
    let mut rt = MockRuntime::permissive();

    let base = ExecutionRequest {
        pre_state_hash: vec![],
        deploys: vec![make_deploy(1, b"hello", 1000)],
        system_deploys: vec![],
        bonds: vec![],
        block_number: 1,
    };
    let alt = ExecutionRequest {
        deploys: vec![make_deploy(2, b"world", 1000)],
        ..base.clone()
    };

    let r1 = rt.execute_block(base).unwrap();
    let r2 = rt.execute_block(alt).unwrap();
    assert_ne!(r1.post_state_hash, r2.post_state_hash);
}

#[test]
fn bond_ordering_does_not_affect_post_state_hash() {
    let mut rt = MockRuntime::permissive();
    let bonds_a = vec![
        Bond {
            validator: node(1),
            stake: 100,
        },
        Bond {
            validator: node(2),
            stake: 200,
        },
    ];
    let bonds_b = vec![
        Bond {
            validator: node(2),
            stake: 200,
        },
        Bond {
            validator: node(1),
            stake: 100,
        },
    ];

    let req_a = ExecutionRequest {
        pre_state_hash: vec![],
        deploys: vec![],
        system_deploys: vec![],
        bonds: bonds_a,
        block_number: 1,
    };
    let req_b = ExecutionRequest {
        bonds: bonds_b,
        ..req_a.clone()
    };

    let r1 = rt.execute_block(req_a).unwrap();
    let r2 = rt.execute_block(req_b).unwrap();
    assert_eq!(r1.post_state_hash, r2.post_state_hash);
}

// ── Deploy classification ──

#[test]
fn deploy_cost_equals_term_length_when_within_limit() {
    let mut rt = MockRuntime::new();
    let term = b"12345"; // 5 bytes
    let req = ExecutionRequest {
        pre_state_hash: vec![],
        deploys: vec![make_deploy(1, term, 1000)],
        system_deploys: vec![],
        bonds: vec![],
        block_number: 1,
    };
    let r = rt.execute_block(req).unwrap();
    assert_eq!(r.processed_deploys.len(), 1);
    assert_eq!(r.processed_deploys[0].cost, 5);
    assert!(!r.processed_deploys[0].is_failed);
}

#[test]
fn deploy_marked_failed_when_exceeds_phlo_limit() {
    let mut rt = MockRuntime::new();
    let term = b"this is a longer term than limit"; // 32 bytes
    let req = ExecutionRequest {
        pre_state_hash: vec![],
        deploys: vec![make_deploy(1, term, 10)], // limit = 10
        system_deploys: vec![],
        bonds: vec![],
        block_number: 1,
    };
    let r = rt.execute_block(req).unwrap();
    assert_eq!(r.processed_deploys.len(), 1);
    assert!(r.processed_deploys[0].is_failed);
    assert_eq!(r.processed_deploys[0].cost, 10); // capped at limit
}

#[test]
fn deploy_with_empty_signature_is_rejected() {
    let mut rt = MockRuntime::new();
    let mut d = make_deploy(1, b"hi", 100);
    d.signature = vec![];
    let req = ExecutionRequest {
        pre_state_hash: vec![],
        deploys: vec![d],
        system_deploys: vec![],
        bonds: vec![],
        block_number: 1,
    };
    let r = rt.execute_block(req).unwrap();
    assert_eq!(r.processed_deploys.len(), 0);
    assert_eq!(r.rejected_deploys.len(), 1);
    assert_eq!(r.rejected_deploys[0].reason, RejectReason::InvalidSignature);
}

// ── System deploys ──

#[test]
fn close_block_system_deploy_succeeds() {
    let mut rt = MockRuntime::new();
    let req = ExecutionRequest {
        pre_state_hash: vec![],
        deploys: vec![],
        system_deploys: vec![SystemDeployRequest::CloseBlock],
        bonds: vec![],
        block_number: 1,
    };
    let r = rt.execute_block(req).unwrap();
    assert_eq!(r.system_deploys.len(), 1);
    assert!(matches!(
        r.system_deploys[0],
        ProcessedSystemDeploy::CloseBlock { succeeded: true }
    ));
}

#[test]
fn guard_invalid_block_hash_rejects_wrong_length() {
    // Test with too short hash
    let result = SystemDeployRequest::validate_invalid_block_hash(&vec![0x01; 31]);
    assert!(result.is_err());
    assert!(
        result
            .unwrap_err()
            .contains("must be exactly 32 bytes, got 31 bytes")
    );

    // Test with too long hash
    let result = SystemDeployRequest::validate_invalid_block_hash(&vec![0x01; 33]);
    assert!(result.is_err());
    assert!(
        result
            .unwrap_err()
            .contains("must be exactly 32 bytes, got 33 bytes")
    );

    // Test with correct length (should succeed)
    let result = SystemDeployRequest::validate_invalid_block_hash(&vec![0x01; 32]);
    assert!(result.is_ok());
}

#[test]
fn slash_removes_validator_bond() {
    let mut rt = MockRuntime::new();
    let req = ExecutionRequest {
        pre_state_hash: vec![],
        deploys: vec![],
        system_deploys: vec![SystemDeployRequest::Slash {
            validator: node(2),
            invalid_block_hash: vec![0x02; 32],
        }],
        bonds: vec![
            Bond {
                validator: node(1),
                stake: 100,
            },
            Bond {
                validator: node(2),
                stake: 200,
            },
        ],
        block_number: 1,
    };
    let r = rt.execute_block(req).unwrap();
    assert_eq!(r.new_bonds.len(), 1);
    assert_eq!(r.new_bonds[0].validator, node(1));
    assert!(matches!(
        r.system_deploys[0],
        ProcessedSystemDeploy::Slash {
            succeeded: true,
            ..
        }
    ));
}

#[test]
fn slash_of_unknown_validator_reports_not_succeeded() {
    let mut rt = MockRuntime::new();
    let req = ExecutionRequest {
        pre_state_hash: vec![],
        deploys: vec![],
        system_deploys: vec![SystemDeployRequest::Slash {
            validator: node(99),
            invalid_block_hash: vec![0x99; 32],
        }],
        bonds: vec![Bond {
            validator: node(1),
            stake: 100,
        }],
        block_number: 1,
    };
    let r = rt.execute_block(req).unwrap();
    assert!(matches!(
        r.system_deploys[0],
        ProcessedSystemDeploy::Slash {
            succeeded: false,
            ..
        }
    ));
    assert_eq!(r.new_bonds.len(), 1); // no change
}

// ── State chaining ──

#[test]
fn consecutive_blocks_chain_pre_state_from_prior_post_state() {
    let mut rt = MockRuntime::new();

    // Block 1 starts from genesis (empty pre-state)
    let req1 = ExecutionRequest {
        pre_state_hash: vec![],
        deploys: vec![make_deploy(1, b"tx1", 100)],
        system_deploys: vec![],
        bonds: vec![],
        block_number: 1,
    };
    let r1 = rt.execute_block(req1).unwrap();

    // Block 2 uses block 1's post-state as its pre-state
    let req2 = ExecutionRequest {
        pre_state_hash: r1.post_state_hash.clone(),
        deploys: vec![make_deploy(2, b"tx2", 100)],
        system_deploys: vec![],
        bonds: vec![],
        block_number: 2,
    };
    let r2 = rt.execute_block(req2).unwrap();
    assert_ne!(r1.post_state_hash, r2.post_state_hash);
    assert!(!r2.post_state_hash.is_empty());
}

#[test]
fn block_with_wrong_pre_state_fails_in_strict_mode() {
    let mut rt = MockRuntime::new();

    // Build one valid block, ignore its post-state
    let req1 = ExecutionRequest {
        pre_state_hash: vec![],
        deploys: vec![make_deploy(1, b"tx", 100)],
        system_deploys: vec![],
        bonds: vec![],
        block_number: 1,
    };
    rt.execute_block(req1).unwrap();

    // Try block 2 from a never-produced pre-state
    let req2 = ExecutionRequest {
        pre_state_hash: vec![0xab, 0xcd],
        deploys: vec![],
        system_deploys: vec![],
        bonds: vec![],
        block_number: 2,
    };
    assert_eq!(
        rt.execute_block(req2).unwrap_err(),
        RuntimeError::UnknownPreState
    );
}

// ── validate_post_state ──

#[test]
fn validate_post_state_accepts_correct_hash() {
    let mut rt = MockRuntime::new();
    let req = ExecutionRequest {
        pre_state_hash: vec![],
        deploys: vec![make_deploy(1, b"tx", 100)],
        system_deploys: vec![],
        bonds: vec![],
        block_number: 1,
    };
    let r = rt.execute_block(req.clone()).unwrap();

    // A fresh runtime should compute the same post-state
    let mut rt2 = MockRuntime::new();
    let valid = rt2.validate_post_state(req, &r.post_state_hash).unwrap();
    assert!(valid);
}

#[test]
fn validate_post_state_rejects_wrong_hash() {
    let mut rt = MockRuntime::new();
    let req = ExecutionRequest {
        pre_state_hash: vec![],
        deploys: vec![make_deploy(1, b"tx", 100)],
        system_deploys: vec![],
        bonds: vec![],
        block_number: 1,
    };
    let wrong_hash = vec![0xff; 32];
    let valid = rt.validate_post_state(req, &wrong_hash).unwrap();
    assert!(!valid);
}

// ── Trait object usage ──

#[test]
fn runtime_usable_via_trait_object() {
    let mut rt: Box<dyn RuntimeManager> = Box::new(MockRuntime::new());
    let r = rt.execute_block(empty_request(0)).unwrap();
    assert!(!r.post_state_hash.is_empty());

    // Ensure helper is used to avoid unused-import warnings
    let _ = ExecutionResult {
        post_state_hash: r.post_state_hash,
        processed_deploys: vec![],
        rejected_deploys: vec![],
        system_deploys: vec![],
        new_bonds: vec![],
    };
}
