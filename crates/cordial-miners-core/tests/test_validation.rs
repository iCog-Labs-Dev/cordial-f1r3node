use std::collections::HashMap;
use cordial_miners_core::blocklace::Blocklace;
use cordial_miners_core::consensus::{
    validate_block, validated_insert, InvalidBlock, ValidationConfig, ValidationResult,
};
use cordial_miners_core::crypto::{hash_content, sign};
use cordial_miners_core::{Block, BlockContent, BlockIdentity, NodeId};
use ed25519_dalek::SigningKey;
use rand::rngs::OsRng;
use std::collections::HashSet;

// ── Helpers ──

fn node(id: u8) -> NodeId {
    NodeId(vec![id])
}

fn make_id(creator: &NodeId, tag: u8) -> BlockIdentity {
    let mut hash = [0u8; 32];
    hash[0] = creator.0[0];
    hash[1] = tag;
    BlockIdentity {
        content_hash: hash,
        creator: creator.clone(),
        signature: vec![],
    }
}

fn genesis_unsigned(creator: &NodeId, tag: u8) -> Block {
    Block {
        identity: make_id(creator, tag),
        content: BlockContent {
            payload: vec![tag],
            predecessors: HashSet::new(),
        },
    }
}

fn child_unsigned(creator: &NodeId, tag: u8, parents: &[&Block]) -> Block {
    let preds = parents.iter().map(|b| b.identity.clone()).collect();
    Block {
        identity: make_id(creator, tag),
        content: BlockContent {
            payload: vec![tag],
            predecessors: preds,
        },
    }
}

/// Create a properly signed genesis block.
fn genesis_signed(private_key: &[u8], creator: &NodeId, tag: u8) -> Block {
    let content = BlockContent {
        payload: vec![tag],
        predecessors: HashSet::new(),
    };
    let content_hash = hash_content(&content);
    let signature = sign(&content_hash, private_key);
    Block {
        identity: BlockIdentity {
            content_hash,
            creator: creator.clone(),
            signature,
        },
        content,
    }
}

/// Create a properly signed child block.
fn child_signed(private_key: &[u8], creator: &NodeId, tag: u8, parents: &[&Block]) -> Block {
    let preds = parents.iter().map(|b| b.identity.clone()).collect();
    let content = BlockContent {
        payload: vec![tag],
        predecessors: preds,
    };
    let content_hash = hash_content(&content);
    let signature = sign(&content_hash, private_key);
    Block {
        identity: BlockIdentity {
            content_hash,
            creator: creator.clone(),
            signature,
        },
        content,
    }
}

fn generate_keypair() -> (Vec<u8>, Vec<u8>) {
    let signing_key = SigningKey::generate(&mut OsRng);
    (
        signing_key.to_bytes().to_vec(),
        signing_key.verifying_key().to_bytes().to_vec(),
    )
}

fn insert(bl: &mut Blocklace, block: &Block) {
    bl.insert(block.clone()).expect("insert failed");
}

fn bonds(entries: &[(u8, u64)]) -> HashMap<NodeId, u64> {
    entries.iter().map(|(id, stake)| (node(*id), *stake)).collect()
}

/// Config that skips crypto checks (for testing structural validation).
fn no_crypto_config() -> ValidationConfig {
    ValidationConfig {
        check_content_hash: false,
        check_signature: false,
        ..Default::default()
    }
}

// ── Closure axiom ──

#[test]
fn valid_genesis_passes_validation() {
    let bl = Blocklace::new();
    let g = genesis_unsigned(&node(1), 1);
    let b = bonds(&[(1, 100)]);
    let result = validate_block(&g, &bl, &b, &no_crypto_config());
    assert!(result.is_valid());
}

#[test]
fn missing_predecessor_fails_closure() {
    let bl = Blocklace::new();
    let g = genesis_unsigned(&node(1), 1);
    let c = child_unsigned(&node(1), 2, &[&g]); // g not in blocklace
    let b = bonds(&[(1, 100)]);
    let result = validate_block(&c, &bl, &b, &no_crypto_config());
    assert!(!result.is_valid());
    assert!(result.errors().iter().any(|e| matches!(e, InvalidBlock::MissingPredecessors { .. })));
}

#[test]
fn known_predecessor_passes_closure() {
    let mut bl = Blocklace::new();
    let g = genesis_unsigned(&node(1), 1);
    insert(&mut bl, &g);
    let c = child_unsigned(&node(1), 2, &[&g]);
    let b = bonds(&[(1, 100)]);
    let result = validate_block(&c, &bl, &b, &no_crypto_config());
    assert!(result.is_valid());
}

// ── Sender check ──

#[test]
fn unbonded_sender_fails() {
    let bl = Blocklace::new();
    let g = genesis_unsigned(&node(1), 1);
    let b = bonds(&[(2, 100)]); // node 1 is NOT bonded
    let result = validate_block(&g, &bl, &b, &no_crypto_config());
    assert!(!result.is_valid());
    assert!(result.errors().iter().any(|e| matches!(e, InvalidBlock::UnknownSender { .. })));
}

#[test]
fn bonded_sender_passes() {
    let bl = Blocklace::new();
    let g = genesis_unsigned(&node(1), 1);
    let b = bonds(&[(1, 100)]);
    let result = validate_block(&g, &bl, &b, &no_crypto_config());
    assert!(result.is_valid());
}

// ── Chain axiom (equivocation) ──

#[test]
fn equivocating_block_fails_chain_axiom() {
    let mut bl = Blocklace::new();
    let v1 = node(1);
    let g1 = genesis_unsigned(&v1, 1);
    insert(&mut bl, &g1);

    // v1 creates a second genesis — equivocation
    let g2 = genesis_unsigned(&v1, 2);
    let b = bonds(&[(1, 100)]);
    let result = validate_block(&g2, &bl, &b, &no_crypto_config());
    assert!(!result.is_valid());
    assert!(result.errors().iter().any(|e| matches!(e, InvalidBlock::Equivocation { .. })));
}

#[test]
fn extending_own_chain_passes_chain_axiom() {
    let mut bl = Blocklace::new();
    let v1 = node(1);
    let g = genesis_unsigned(&v1, 1);
    insert(&mut bl, &g);

    let c = child_unsigned(&v1, 2, &[&g]);
    let b = bonds(&[(1, 100)]);
    let result = validate_block(&c, &bl, &b, &no_crypto_config());
    assert!(result.is_valid());
}

// ── Content hash ──

#[test]
fn correct_content_hash_passes() {
    let bl = Blocklace::new();
    let content = BlockContent {
        payload: vec![42],
        predecessors: HashSet::new(),
    };
    let content_hash = hash_content(&content);
    let block = Block {
        identity: BlockIdentity {
            content_hash,
            creator: node(1),
            signature: vec![],
        },
        content,
    };
    let b = bonds(&[(1, 100)]);
    let config = ValidationConfig {
        check_content_hash: true,
        check_signature: false,
        ..Default::default()
    };
    let result = validate_block(&block, &bl, &b, &config);
    assert!(result.is_valid());
}

#[test]
fn wrong_content_hash_fails() {
    let bl = Blocklace::new();
    let content = BlockContent {
        payload: vec![42],
        predecessors: HashSet::new(),
    };
    let block = Block {
        identity: BlockIdentity {
            content_hash: [0xff; 32], // wrong hash
            creator: node(1),
            signature: vec![],
        },
        content,
    };
    let b = bonds(&[(1, 100)]);
    let config = ValidationConfig {
        check_content_hash: true,
        check_signature: false,
        ..Default::default()
    };
    let result = validate_block(&block, &bl, &b, &config);
    assert!(!result.is_valid());
    assert!(result.errors().iter().any(|e| matches!(e, InvalidBlock::InvalidContentHash { .. })));
}

// ── Signature ──

#[test]
fn valid_signature_passes() {
    let (private_key, public_key) = generate_keypair();
    let creator = NodeId(public_key);
    let bl = Blocklace::new();
    let g = genesis_signed(&private_key, &creator, 1);
    let b: HashMap<NodeId, u64> = [(creator.clone(), 100)].into();
    let config = ValidationConfig::default();
    let result = validate_block(&g, &bl, &b, &config);
    assert!(result.is_valid());
}

#[test]
fn invalid_signature_fails() {
    let (private_key, public_key) = generate_keypair();
    let creator = NodeId(public_key);
    let bl = Blocklace::new();
    let mut g = genesis_signed(&private_key, &creator, 1);
    // Tamper with the signature
    g.identity.signature[0] ^= 0xff;
    let b: HashMap<NodeId, u64> = [(creator.clone(), 100)].into();
    let config = ValidationConfig::default();
    let result = validate_block(&g, &bl, &b, &config);
    assert!(!result.is_valid());
    assert!(result.errors().iter().any(|e| matches!(e, InvalidBlock::InvalidSignature)));
}

// ── Cordial condition ──

#[test]
fn cordial_block_passes_strict_validation() {
    let mut bl = Blocklace::new();
    let v1 = node(1);
    let v2 = node(2);

    let g1 = genesis_unsigned(&v1, 1);
    let g2 = genesis_unsigned(&v2, 2);
    insert(&mut bl, &g1);
    insert(&mut bl, &g2);

    // v1 creates a block referencing both tips — cordial
    let cordial = child_unsigned(&v1, 3, &[&g1, &g2]);
    let b = bonds(&[(1, 100), (2, 100)]);
    let config = ValidationConfig {
        check_cordial: true,
        ..no_crypto_config()
    };
    let result = validate_block(&cordial, &bl, &b, &config);
    assert!(result.is_valid());
}

#[test]
fn non_cordial_block_fails_strict_validation() {
    let mut bl = Blocklace::new();
    let v1 = node(1);
    let v2 = node(2);

    let g1 = genesis_unsigned(&v1, 1);
    let g2 = genesis_unsigned(&v2, 2);
    insert(&mut bl, &g1);
    insert(&mut bl, &g2);

    // v1 creates a block referencing only its own genesis — NOT cordial
    let non_cordial = child_unsigned(&v1, 3, &[&g1]);
    let b = bonds(&[(1, 100), (2, 100)]);
    let config = ValidationConfig {
        check_cordial: true,
        ..no_crypto_config()
    };
    let result = validate_block(&non_cordial, &bl, &b, &config);
    assert!(!result.is_valid());
    assert!(result.errors().iter().any(|e| matches!(e, InvalidBlock::NotCordial { .. })));
}

// ── validated_insert ──

#[test]
fn validated_insert_inserts_valid_block() {
    let mut bl = Blocklace::new();
    let g = genesis_unsigned(&node(1), 1);
    let b = bonds(&[(1, 100)]);
    let result = validated_insert(g.clone(), &mut bl, &b, &no_crypto_config());
    assert!(result.is_valid());
    assert!(bl.get(&g.identity).is_some());
}

#[test]
fn validated_insert_rejects_invalid_block() {
    let mut bl = Blocklace::new();
    let g = genesis_unsigned(&node(1), 1);
    let c = child_unsigned(&node(1), 2, &[&g]); // g not inserted
    let b = bonds(&[(1, 100)]);
    let result = validated_insert(c.clone(), &mut bl, &b, &no_crypto_config());
    assert!(!result.is_valid());
    assert!(bl.get(&c.identity).is_none()); // NOT inserted
}

// ── Multiple errors ──

#[test]
fn multiple_errors_collected() {
    let bl = Blocklace::new();
    let g = genesis_unsigned(&node(1), 1);
    // Make a child with missing predecessor AND unbonded sender
    let c = child_unsigned(&node(99), 2, &[&g]);
    let b = bonds(&[(1, 100)]); // node 99 not bonded
    let result = validate_block(&c, &bl, &b, &no_crypto_config());
    assert!(!result.is_valid());
    // Should have both MissingPredecessors AND UnknownSender
    assert!(result.errors().len() >= 2);
}

// ── ValidationResult helpers ──

#[test]
fn validation_result_helpers() {
    let valid = ValidationResult::Valid;
    assert!(valid.is_valid());
    assert!(valid.errors().is_empty());

    let invalid = ValidationResult::Invalid(vec![InvalidBlock::InvalidSignature]);
    assert!(!invalid.is_valid());
    assert_eq!(invalid.errors().len(), 1);
}

// ── Full signed chain ──

#[test]
fn full_signed_chain_validates() {
    let (pk1, pub1) = generate_keypair();
    let (pk2, pub2) = generate_keypair();
    let v1 = NodeId(pub1);
    let v2 = NodeId(pub2);

    let mut bl = Blocklace::new();
    let b: HashMap<NodeId, u64> = [(v1.clone(), 100), (v2.clone(), 100)].into();
    let config = ValidationConfig::default();

    // v1 creates signed genesis
    let g1 = genesis_signed(&pk1, &v1, 1);
    let result = validated_insert(g1.clone(), &mut bl, &b, &config);
    assert!(result.is_valid());

    // v2 creates signed block on top of g1
    let b2 = child_signed(&pk2, &v2, 2, &[&g1]);
    let result = validated_insert(b2.clone(), &mut bl, &b, &config);
    assert!(result.is_valid());

    // v1 extends the chain
    let b3 = child_signed(&pk1, &v1, 3, &[&b2]);
    let result = validated_insert(b3, &mut bl, &b, &config);
    assert!(result.is_valid());

    assert_eq!(bl.dom().len(), 3);
    assert!(bl.is_closed());
}
