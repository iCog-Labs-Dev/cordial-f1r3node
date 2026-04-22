//! Tests for the translation helpers in `blocklace-f1r3rspace`.
//!
//! These test only the pure translation functions — they do not require a
//! running `f1r3node::RuntimeManager` or RSpace tuplespace.
//!
//! End-to-end tests that actually call `execute_block` against a real
//! RuntimeManager require bringing up LMDB storage + Rholang interpreter
//! bootstrapping, which is f1r3node's node-binary-sized setup. Those live
//! outside this crate (Phase 4 integration harness).

use cordial_miners_core::execution::{
    Bond, Deploy, ExecutionRequest, ProcessedDeploy, ProcessedSystemDeploy, SignedDeploy,
    SystemDeployRequest,
};
use cordial_miners_core::types::NodeId;

use cordial_f1r3space_adapter::{
    build_block_data, processed_deploy_from_f1r3node, signed_deploy_to_f1r3node,
    system_deploy_from_f1r3node, system_deploy_to_f1r3node,
};

use casper::rust::util::rholang::system_deploy_enum::SystemDeployEnum;
use models::rust::casper::protocol::casper_message::{
    ProcessedDeploy as F1r3ProcessedDeploy, ProcessedSystemDeploy as F1r3ProcessedSystemDeploy,
    SystemDeployData,
};

// ── Helpers ──────────────────────────────────────────────────────────────

fn sample_signed_deploy(sig_byte: u8) -> SignedDeploy {
    SignedDeploy {
        deploy: Deploy {
            term: b"@0!(\"hello\")".to_vec(),
            timestamp: 1_700_000_000_000,
            phlo_price: 1,
            phlo_limit: 10_000,
            valid_after_block_number: 0,
            shard_id: "root".to_string(),
        },
        deployer: vec![sig_byte; 33], // 33 bytes for secp256k1 compressed key
        signature: vec![sig_byte; 64],
    }
}

fn node(b: u8) -> NodeId {
    NodeId(vec![b])
}

// ── signed_deploy_to_f1r3node ────────────────────────────────────────────

#[test]
fn signed_deploy_translates_preserving_signature() {
    let sd = sample_signed_deploy(0xaa);
    let f1 = signed_deploy_to_f1r3node(&sd).unwrap();

    assert_eq!(f1.data.term, "@0!(\"hello\")");
    assert_eq!(f1.data.time_stamp, 1_700_000_000_000);
    assert_eq!(f1.data.phlo_price, 1);
    assert_eq!(f1.data.phlo_limit, 10_000);
    assert_eq!(f1.data.valid_after_block_number, 0);
    assert_eq!(f1.data.shard_id, "root");
    assert_eq!(f1.data.expiration_timestamp, None);

    // Signature and pubkey preserved verbatim
    assert_eq!(f1.sig.to_vec(), vec![0xaa; 64]);
    assert_eq!(f1.pk.bytes.to_vec(), vec![0xaa; 33]);
    assert_eq!(f1.sig_algorithm.name(), "secp256k1");
}

#[test]
fn signed_deploy_with_non_utf8_term_uses_replacement_chars() {
    let mut sd = sample_signed_deploy(1);
    // Invalid UTF-8 byte sequence
    sd.deploy.term = vec![0xff, 0xfe, 0xfd];
    let f1 = signed_deploy_to_f1r3node(&sd).unwrap();
    // from_utf8_lossy replaces invalid sequences with U+FFFD (REPLACEMENT CHARACTER)
    assert!(f1.data.term.contains('\u{FFFD}'));
}

// ── system_deploy_to_f1r3node ────────────────────────────────────────────

#[test]
fn close_block_translates_to_close_variant() {
    let pre_state = [0x42u8; 32];
    let f1 = system_deploy_to_f1r3node(&SystemDeployRequest::CloseBlock, &pre_state);
    assert!(matches!(f1, SystemDeployEnum::Close(_)));
}

#[test]
fn slash_translates_to_slash_variant_with_validator_pk() {
    let pre_state = [0x42u8; 32];
    let f1 = system_deploy_to_f1r3node(
        &SystemDeployRequest::Slash { validator: node(7) },
        &pre_state,
    );
    match f1 {
        SystemDeployEnum::Slash(s) => {
            assert_eq!(s.pk.bytes.to_vec(), vec![7u8]);
            // invalid_block_hash is set to the validator bytes as a placeholder
            assert_eq!(s.invalid_block_hash.to_vec(), vec![7u8]);
        }
        _ => panic!("expected Slash variant"),
    }
}

// ── build_block_data ─────────────────────────────────────────────────────

#[test]
fn block_data_picks_first_bond_as_sender() {
    let req = ExecutionRequest {
        pre_state_hash: vec![0x00; 32],
        deploys: vec![],
        system_deploys: vec![],
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
        block_number: 5,
    };
    let bd = build_block_data(&req).unwrap();
    assert_eq!(bd.block_number, 5);
    assert_eq!(bd.sender.bytes.to_vec(), vec![1u8]);
    assert_eq!(bd.seq_num, 0);
    assert_eq!(bd.time_stamp, 0);
}

#[test]
fn block_data_with_no_bonds_uses_zero_pubkey() {
    let req = ExecutionRequest {
        pre_state_hash: vec![],
        deploys: vec![],
        system_deploys: vec![],
        bonds: vec![],
        block_number: 0,
    };
    let bd = build_block_data(&req).unwrap();
    assert_eq!(bd.sender.bytes.to_vec(), vec![0u8; 33]);
}

#[test]
fn block_data_overflow_is_reported_not_panic() {
    let req = ExecutionRequest {
        pre_state_hash: vec![],
        deploys: vec![],
        system_deploys: vec![],
        bonds: vec![],
        block_number: u64::MAX, // can't fit in i64
    };
    let result = build_block_data(&req);
    assert!(result.is_err());
}

// ── processed_deploy_from_f1r3node ───────────────────────────────────────

#[test]
fn processed_deploy_round_trips_through_f1r3node_types() {
    let sd = sample_signed_deploy(0x55);
    let f1_signed = signed_deploy_to_f1r3node(&sd).unwrap();

    // Build a f1r3node ProcessedDeploy wrapping our translated Signed
    let f1_processed = F1r3ProcessedDeploy {
        deploy: f1_signed,
        cost: models::rhoapi::PCost { cost: 123 },
        deploy_log: vec![],
        is_failed: false,
        system_deploy_error: None,
    };

    // Translate back to our type
    let ours: ProcessedDeploy = processed_deploy_from_f1r3node(&f1_processed).unwrap();

    assert_eq!(ours.cost, 123);
    assert!(!ours.is_failed);
    assert_eq!(ours.deploy.deploy.term, b"@0!(\"hello\")");
    assert_eq!(ours.deploy.deployer, vec![0x55; 33]);
    assert_eq!(ours.deploy.signature, vec![0x55; 64]);
}

#[test]
fn failed_processed_deploy_preserves_is_failed_flag() {
    let sd = sample_signed_deploy(1);
    let f1_signed = signed_deploy_to_f1r3node(&sd).unwrap();
    let f1_processed = F1r3ProcessedDeploy {
        deploy: f1_signed,
        cost: models::rhoapi::PCost { cost: 99 },
        deploy_log: vec![],
        is_failed: true,
        system_deploy_error: Some("test failure".to_string()),
    };
    let ours = processed_deploy_from_f1r3node(&f1_processed).unwrap();
    assert!(ours.is_failed);
    assert_eq!(ours.cost, 99);
}

// ── system_deploy_from_f1r3node ──────────────────────────────────────────

#[test]
fn succeeded_close_block_translates_correctly() {
    let f1 = F1r3ProcessedSystemDeploy::Succeeded {
        event_list: vec![],
        system_deploy: SystemDeployData::CloseBlockSystemDeployData,
    };
    let ours = system_deploy_from_f1r3node(&f1);
    assert_eq!(ours, ProcessedSystemDeploy::CloseBlock { succeeded: true });
}

#[test]
fn succeeded_slash_translates_with_validator() {
    use crypto::rust::public_key::PublicKey;
    let validator_bytes = vec![0x99u8; 33];
    let f1 = F1r3ProcessedSystemDeploy::Succeeded {
        event_list: vec![],
        system_deploy: SystemDeployData::Slash {
            invalid_block_hash: prost::bytes::Bytes::from(vec![0u8; 32]),
            issuer_public_key: PublicKey::from_bytes(&validator_bytes),
        },
    };
    let ours = system_deploy_from_f1r3node(&f1);
    match ours {
        ProcessedSystemDeploy::Slash {
            validator,
            succeeded,
        } => {
            assert_eq!(validator.0, validator_bytes);
            assert!(succeeded);
        }
        _ => panic!("expected Slash"),
    }
}

#[test]
fn failed_system_deploy_reports_not_succeeded() {
    let f1 = F1r3ProcessedSystemDeploy::Failed {
        event_list: vec![],
        error_msg: "boom".to_string(),
    };
    let ours = system_deploy_from_f1r3node(&f1);
    // Failed variant lacks the original discriminant; we surface it as a
    // failed CloseBlock (documented in the adapter's module-level docs).
    assert_eq!(ours, ProcessedSystemDeploy::CloseBlock { succeeded: false });
}

#[test]
fn empty_system_deploy_data_translates_as_succeeded_close_block() {
    // SystemDeployData::Empty is a catch-all variant we map to a succeeded
    // CloseBlock — least-information-losing when the f1r3node side didn't
    // classify further.
    let f1 = F1r3ProcessedSystemDeploy::Succeeded {
        event_list: vec![],
        system_deploy: SystemDeployData::Empty,
    };
    let ours = system_deploy_from_f1r3node(&f1);
    assert_eq!(ours, ProcessedSystemDeploy::CloseBlock { succeeded: true });
}
