use cordial_miners_core::blocklace::Blocklace;
use cordial_miners_core::consensus::{
    InvalidBlock, ValidationConfig, ValidationResult, validate_block, validated_insert,
};
use cordial_miners_core::crypto::CryptoVerifier;
use cordial_miners_core::crypto::{hash_content, sign};
use cordial_miners_core::{Block, BlockContent, BlockIdentity, NodeId};
use ed25519_dalek::SigningKey as EdSigningKey;
use rand::rngs::OsRng;
use std::collections::HashMap;
use std::collections::HashSet;

struct MockVerifier;

impl CryptoVerifier for MockVerifier {
    type Error = String;
    fn verify_block(
        &self,
        _content: &BlockContent,
        _sig: &[u8],
        _creator: &NodeId,
    ) -> Result<(), Self::Error> {
        Ok(()) // Always allow in tests
    }
}

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
    let signing_key = EdSigningKey::generate(&mut OsRng);
    (
        signing_key.to_bytes().to_vec(),
        signing_key.verifying_key().to_bytes().to_vec(),
    )
}

fn generate_secp_keypair() -> (Vec<u8>, Vec<u8>) {
    let signing_key = k256::ecdsa::SigningKey::random(&mut OsRng);
    let private = signing_key.to_bytes().to_vec();
    let public = signing_key.verifying_key().to_sec1_bytes().to_vec();
    (private, public)
}

fn insert(bl: &mut Blocklace, block: &Block) {
    let verifier = MockVerifier;
    bl.insert(block.clone(), &verifier).expect("insert failed");
}

fn bonds(entries: &[(u8, u64)]) -> HashMap<NodeId, u64> {
    entries
        .iter()
        .map(|(id, stake)| (node(*id), *stake))
        .collect()
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
    assert!(
        result
            .errors()
            .iter()
            .any(|e| matches!(e, InvalidBlock::MissingPredecessors { .. }))
    );
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
    assert!(
        result
            .errors()
            .iter()
            .any(|e| matches!(e, InvalidBlock::UnknownSender { .. }))
    );
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
    assert!(
        result
            .errors()
            .iter()
            .any(|e| matches!(e, InvalidBlock::Equivocation { .. }))
    );
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

/// Regression: a block that arrives with a multi-round gap in its history must
/// report only the missing predecessors, never an equivocation.
///
/// The chain-axiom check decides comparability by walking the local blocklace
/// from the creator's existing blocks through the arriving block's
/// predecessors. Across a gap of two or more rounds that walk cannot complete,
/// because the intermediate block is not there to be walked through — so the
/// honest block used to be reported as `Equivocation` on top of
/// `MissingPredecessors`.
///
/// That combination is what made it damaging: callers buffer a block for retry
/// only when *every* error is `MissingPredecessors`, so the extra error caused
/// an honest block to be dropped outright instead of retried once its history
/// arrived. See `pending_buffer_keeps_multi_round_gap_when_creator_is_known`
/// in test_dissemination.rs for the buffering half of this behaviour.
#[test]
fn multi_round_gap_reports_missing_predecessors_without_equivocation() {
    let mut bl = Blocklace::new();
    let v1 = node(1);

    // The creator's round-0 block is known locally. This is the precondition
    // the bug needed: `blocks_by(creator)` must be non-empty for the
    // chain-axiom loop to run at all.
    let round0 = genesis_unsigned(&v1, 1);
    insert(&mut bl, &round0);

    // Round 1 is created but never delivered, so round 2 arrives with a
    // two-round gap: its predecessor is absent from the local view.
    let round1 = child_unsigned(&v1, 2, &[&round0]);
    let round2 = child_unsigned(&v1, 3, &[&round1]);

    let b = bonds(&[(1, 100)]);
    let result = validate_block(&round2, &bl, &b, &no_crypto_config());

    assert!(
        !result.is_valid(),
        "the gap itself is still a closure failure"
    );
    assert!(
        result
            .errors()
            .iter()
            .any(|e| matches!(e, InvalidBlock::MissingPredecessors { .. })),
        "the missing round-1 predecessor should be reported: {:?}",
        result.errors()
    );
    assert!(
        !result
            .errors()
            .iter()
            .any(|e| matches!(e, InvalidBlock::Equivocation { .. })),
        "an honest block must not be reported as equivocation while its \
         history is missing: {:?}",
        result.errors()
    );

    // Deferring the check must not lose it. Once the gap is filled, the same
    // block validates cleanly.
    insert(&mut bl, &round1);
    assert!(validate_block(&round2, &bl, &b, &no_crypto_config()).is_valid());
}

/// Guard against over-correcting the above: deferring the chain-axiom check
/// must not let genuine equivocation through once the history *is* present.
///
/// `equivocating_block_fails_chain_axiom` covers this at round 0, where the
/// creator has no predecessors at all. This covers a mid-chain round, which is
/// the case the deferral logic actually touches.
#[test]
fn genuine_equivocation_is_still_detected_at_a_mid_chain_round() {
    let mut bl = Blocklace::new();
    let v1 = node(1);

    let round0 = genesis_unsigned(&v1, 1);
    let round1 = child_unsigned(&v1, 2, &[&round0]);
    insert(&mut bl, &round0);
    insert(&mut bl, &round1);

    // A second, conflicting round-1 block over the same predecessor. Nothing is
    // missing here, so the chain axiom is decidable and must fire.
    let round1_conflicting = child_unsigned(&v1, 3, &[&round0]);

    let b = bonds(&[(1, 100)]);
    let result = validate_block(&round1_conflicting, &bl, &b, &no_crypto_config());

    assert!(!result.is_valid());
    assert!(
        result
            .errors()
            .iter()
            .any(|e| matches!(e, InvalidBlock::Equivocation { .. })),
        "a same-round conflicting block with all predecessors present is a \
         real equivocation: {:?}",
        result.errors()
    );
    assert!(
        !result
            .errors()
            .iter()
            .any(|e| matches!(e, InvalidBlock::MissingPredecessors { .. })),
        "nothing is missing in this scenario: {:?}",
        result.errors()
    );
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
    assert!(
        result
            .errors()
            .iter()
            .any(|e| matches!(e, InvalidBlock::InvalidContentHash { .. }))
    );
}

// ── Signature ──

#[test]
fn valid_signature_passes() {
    let (private_key, public_key) = generate_secp_keypair();
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
    assert!(
        result
            .errors()
            .iter()
            .any(|e| matches!(e, InvalidBlock::InvalidSignature))
    );
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
    assert!(
        result
            .errors()
            .iter()
            .any(|e| matches!(e, InvalidBlock::NotCordial { .. }))
    );
}

// Test that a block that hides a known equivocation fails strict validation. We create two equivocation blocks by the same creator and then create a candidate block that only acknowledges one of them. The candidate block should be considered as hiding the other equivocation block, and therefore should fail strict validation with an InvalidBlock::HiddenEquivocation error.
#[test]
fn block_hiding_known_equivocation_fails_strict_validation() {
    let mut bl = Blocklace::new();
    let v1 = node(1);
    let v2 = node(2);
    let v3 = node(3);

    let e1 = genesis_unsigned(&v1, 1);
    let e2 = genesis_unsigned(&v1, 2);
    let g2 = genesis_unsigned(&v2, 3);
    insert(&mut bl, &e1);
    insert(&mut bl, &e2);
    insert(&mut bl, &g2);

    let hidden = child_unsigned(&v3, 4, &[&e1, &g2]);
    let b = bonds(&[(1, 100), (2, 100), (3, 100)]);
    let config = ValidationConfig {
        check_cordial: true,
        ..no_crypto_config()
    };

    let result = validate_block(&hidden, &bl, &b, &config);
    assert!(!result.is_valid());
    assert!(
        result
            .errors()
            .iter()
            .any(|e| matches!(e, InvalidBlock::HiddenEquivocation { .. }))
    );
}

/// The cordial checks are deferred while predecessors are missing, for the same
/// reason as the chain axiom.
///
/// `missing_known_tips` and `hidden_equivocations` both judge a block against a
/// predecessor closure reconstructed from the local blocklace, so an incomplete
/// view yields a verdict about the view rather than the block. Worse, reporting
/// `HiddenEquivocation` alongside `MissingPredecessors` makes the error set
/// non-uniform, and callers retain a block for retry only when *every* error is
/// `MissingPredecessors` — so an honest block would be discarded outright under
/// `check_cordial`, exactly as it was under the chain axiom.
///
/// This only affects configurations that enable `check_cordial`, which is off in
/// `ValidationConfig::default()`.
#[test]
fn cordial_checks_are_deferred_while_predecessors_are_missing() {
    let mut bl = Blocklace::new();
    let v1 = node(1);
    let v2 = node(2);
    let v3 = node(3);

    // A known equivocation by v1 sits in the local view.
    let e1 = genesis_unsigned(&v1, 1);
    let e2 = genesis_unsigned(&v1, 2);
    insert(&mut bl, &e1);
    insert(&mut bl, &e2);

    // v3's block acknowledges only one branch of that equivocation *and*
    // references a predecessor that has not arrived.
    let undelivered = genesis_unsigned(&v2, 3);
    let candidate = child_unsigned(&v3, 4, &[&e1, &undelivered]);

    let b = bonds(&[(1, 100), (2, 100), (3, 100)]);
    let config = ValidationConfig {
        check_cordial: true,
        ..no_crypto_config()
    };

    let result = validate_block(&candidate, &bl, &b, &config);
    assert!(!result.is_valid(), "the gap is still a closure failure");
    assert!(
        result
            .errors()
            .iter()
            .all(|e| matches!(e, InvalidBlock::MissingPredecessors { .. })),
        "while history is missing, only the gap may be reported, otherwise the \
         block is discarded instead of buffered: {:?}",
        result.errors()
    );

    // Once the gap is filled the cordial verdict is decidable, and the hidden
    // equivocation is reported as it should be.
    insert(&mut bl, &undelivered);
    let result = validate_block(&candidate, &bl, &b, &config);
    assert!(
        result
            .errors()
            .iter()
            .any(|e| matches!(e, InvalidBlock::HiddenEquivocation { .. })),
        "deferring must not lose the check: {:?}",
        result.errors()
    );
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
    let (pk1, pub1) = generate_secp_keypair();
    let (pk2, pub2) = generate_secp_keypair();
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
