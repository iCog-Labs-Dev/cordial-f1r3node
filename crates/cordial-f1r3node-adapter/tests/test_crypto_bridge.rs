//! Tests for `crypto_bridge` — Blake2b-256, Secp256k1, ED25519, and the
//! f1r3node-style block hash that mixes the sender into the input.

use cordial_f1r3node_adapter::block_translation::{
    BlockMessage, Body, F1r3flyState, Header, ProcessedSystemDeploy,
};
use cordial_f1r3node_adapter::crypto_bridge::{
    Blake2b256Hasher, CryptoError, Ed25519, F1r3flyCryptoAdapter, Hasher, Secp256k1, Sha256Hasher,
    SigAlgorithm, Signer, Verifier, compute_block_hash,
};

use cordial_miners_core::crypto::{CryptoVerifier, hash_content as core_hash};
use cordial_miners_core::types::{BlockContent, NodeId};
use std::collections::HashSet;

// ── Hasher correctness ───────────────────────────────────────────────────

#[test]
fn sha256_hasher_matches_known_vector() {
    // RFC: sha256("abc") = ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad
    let expected = [
        0xba, 0x78, 0x16, 0xbf, 0x8f, 0x01, 0xcf, 0xea, 0x41, 0x41, 0x40, 0xde, 0x5d, 0xae, 0x22,
        0x23, 0xb0, 0x03, 0x61, 0xa3, 0x96, 0x17, 0x7a, 0x9c, 0xb4, 0x10, 0xff, 0x61, 0xf2, 0x00,
        0x15, 0xad,
    ];
    let h = Sha256Hasher;
    assert_eq!(h.hash(b"abc"), expected);
    assert_eq!(h.name(), "sha256");
}

#[test]
fn blake2b256_hasher_matches_f1r3node_output() {
    // f1r3node uses `Blake2b::<U32>::new()` (see
    // f1r3node/crypto/src/rust/hash/blake2b256.rs). This is "Blake2b with
    // a 32-byte digest length parameter", which RFC 7693 distinguishes
    // from "Blake2b truncated to 32 bytes". The two produce different
    // outputs — we use the same primitive as f1r3node, so this test
    // pins our output to bytewise equality with theirs.
    //
    // Vector: Blake2b<U32>("abc") = bddd813c63423972 3171ef3fee98579b
    //                               94964e3bb1cb3e42 7262c8c068d52319
    let expected = [
        0xbd, 0xdd, 0x81, 0x3c, 0x63, 0x42, 0x39, 0x72, 0x31, 0x71, 0xef, 0x3f, 0xee, 0x98, 0x57,
        0x9b, 0x94, 0x96, 0x4e, 0x3b, 0xb1, 0xcb, 0x3e, 0x42, 0x72, 0x62, 0xc8, 0xc0, 0x68, 0xd5,
        0x23, 0x19,
    ];
    let h = Blake2b256Hasher;
    assert_eq!(h.hash(b"abc"), expected);
    assert_eq!(h.name(), "blake2b256");
}

#[test]
fn hashers_produce_different_outputs_for_same_input() {
    let s = Sha256Hasher.hash(b"hello");
    let b = Blake2b256Hasher.hash(b"hello");
    assert_ne!(s, b);
}

#[test]
fn empty_input_hashes_are_consistent() {
    let s1 = Sha256Hasher.hash(b"");
    let s2 = Sha256Hasher.hash(b"");
    assert_eq!(s1, s2);

    let b1 = Blake2b256Hasher.hash(b"");
    let b2 = Blake2b256Hasher.hash(b"");
    assert_eq!(b1, b2);
}

// ── ED25519 ──────────────────────────────────────────────────────────────

fn ed25519_keypair() -> ([u8; 32], [u8; 32]) {
    use ed25519_dalek::SigningKey;
    use rand::rngs::OsRng;
    let signing = SigningKey::generate(&mut OsRng);
    let pk: [u8; 32] = signing.verifying_key().to_bytes();
    let sk: [u8; 32] = signing.to_bytes();
    (sk, pk)
}

#[test]
fn ed25519_sign_and_verify_roundtrip() {
    let (sk, pk) = ed25519_keypair();
    let hash = [0x42u8; 32];
    let sig = Ed25519.sign(&hash, &sk).unwrap();
    assert_eq!(sig.len(), 64);
    let valid = Ed25519.verify(&hash, &pk, &sig).unwrap();
    assert!(valid);
}

#[test]
fn ed25519_verify_rejects_tampered_hash() {
    let (sk, pk) = ed25519_keypair();
    let hash = [0x42u8; 32];
    let sig = Ed25519.sign(&hash, &sk).unwrap();
    let tampered = [0x43u8; 32];
    assert!(!Ed25519.verify(&tampered, &pk, &sig).unwrap());
}

#[test]
fn ed25519_verify_rejects_wrong_public_key() {
    let (sk, _) = ed25519_keypair();
    let (_, other_pk) = ed25519_keypair();
    let hash = [0x42u8; 32];
    let sig = Ed25519.sign(&hash, &sk).unwrap();
    assert!(!Ed25519.verify(&hash, &other_pk, &sig).unwrap());
}

#[test]
fn ed25519_invalid_key_lengths_error() {
    let hash = [0x00u8; 32];
    assert!(matches!(
        Ed25519.sign(&hash, &[0u8; 16]),
        Err(CryptoError::InvalidPrivateKeyLength {
            expected: 32,
            actual: 16
        })
    ));
    assert!(matches!(
        Ed25519.verify(&hash, &[0u8; 16], &[0u8; 64]),
        Err(CryptoError::InvalidPublicKeyLength {
            expected: 32,
            actual: 16
        })
    ));
    let (_, pk) = ed25519_keypair();
    assert!(matches!(
        Ed25519.verify(&hash, &pk, &[0u8; 32]),
        Err(CryptoError::InvalidSignatureLength {
            expected: 64,
            actual: 32
        })
    ));
}

#[test]
fn ed25519_algorithm_id_string() {
    // Disambiguate: both Signer and Verifier have an `algorithm()` method
    assert_eq!(
        <Ed25519 as Signer>::algorithm(&Ed25519),
        SigAlgorithm::Ed25519
    );
    assert_eq!(
        <Ed25519 as Verifier>::algorithm(&Ed25519),
        SigAlgorithm::Ed25519
    );
    assert_eq!(SigAlgorithm::Ed25519.as_str(), "ed25519");
}

// ── Secp256k1 ────────────────────────────────────────────────────────────

fn secp256k1_keypair() -> ([u8; 32], Vec<u8>) {
    use k256::ecdsa::SigningKey;
    let sk = SigningKey::random(&mut rand::rngs::OsRng);
    let pk = sk.verifying_key().to_sec1_bytes().to_vec(); // compressed (33 bytes)
    let sk_bytes: [u8; 32] = sk.to_bytes().into();
    (sk_bytes, pk)
}

#[test]
fn secp256k1_sign_and_verify_roundtrip() {
    let (sk, pk) = secp256k1_keypair();
    let hash = [0x77u8; 32];
    let sig = Secp256k1.sign(&hash, &sk).unwrap();
    assert_eq!(sig.len(), 64);
    let valid = Secp256k1.verify(&hash, &pk, &sig).unwrap();
    assert!(valid);
}

#[test]
fn secp256k1_verify_rejects_tampered_hash() {
    let (sk, pk) = secp256k1_keypair();
    let hash = [0x77u8; 32];
    let sig = Secp256k1.sign(&hash, &sk).unwrap();
    let tampered = [0x78u8; 32];
    assert!(!Secp256k1.verify(&tampered, &pk, &sig).unwrap());
}

#[test]
fn secp256k1_verify_rejects_wrong_public_key() {
    let (sk, _) = secp256k1_keypair();
    let (_, other_pk) = secp256k1_keypair();
    let hash = [0x77u8; 32];
    let sig = Secp256k1.sign(&hash, &sk).unwrap();
    assert!(!Secp256k1.verify(&hash, &other_pk, &sig).unwrap());
}

#[test]
fn secp256k1_invalid_inputs_error() {
    let hash = [0u8; 32];
    assert!(matches!(
        Secp256k1.sign(&hash, &[0u8; 16]),
        Err(CryptoError::InvalidPrivateKeyLength {
            expected: 32,
            actual: 16
        })
    ));
    assert!(matches!(
        Secp256k1.verify(&hash, &[0u8; 16], &[0u8; 64]),
        Err(CryptoError::InvalidPublicKeyLength {
            expected: 33,
            actual: 16
        })
    ));
}

#[test]
fn secp256k1_algorithm_id_string() {
    assert_eq!(
        <Secp256k1 as Signer>::algorithm(&Secp256k1),
        SigAlgorithm::Secp256k1
    );
    assert_eq!(
        <Secp256k1 as Verifier>::algorithm(&Secp256k1),
        SigAlgorithm::Secp256k1
    );
    assert_eq!(SigAlgorithm::Secp256k1.as_str(), "secp256k1");
}

// ── Block hash: the snapshot collision fix ───────────────────────────────

fn empty_msg(sender: Vec<u8>) -> BlockMessage {
    BlockMessage {
        block_hash: vec![],
        header: Header {
            parents_hash_list: vec![],
            timestamp: 0,
            version: 1,
            extra_bytes: vec![],
        },
        body: Body {
            state: F1r3flyState {
                pre_state_hash: vec![],
                post_state_hash: vec![],
                bonds: vec![],
                block_number: 0,
            },
            deploys: vec![],
            rejected_deploys: vec![],
            system_deploys: vec![],
            extra_bytes: vec![],
        },
        justifications: vec![],
        sender,
        seq_num: 0,
        sig: vec![],
        sig_algorithm: "ed25519".to_string(),
        shard_id: "root".to_string(),
        extra_bytes: vec![],
    }
}

#[test]
fn compute_block_hash_returns_32_bytes() {
    let msg = empty_msg(vec![1]);
    let hash = compute_block_hash(&msg);
    assert_eq!(hash.len(), 32);
}

#[test]
fn compute_block_hash_is_deterministic() {
    let msg = empty_msg(vec![1, 2, 3]);
    let h1 = compute_block_hash(&msg);
    let h2 = compute_block_hash(&msg);
    assert_eq!(h1, h2);
}

#[test]
fn different_senders_produce_different_block_hashes_for_same_body() {
    // This is the central guarantee — what fixes the snapshot collision
    // documented in src/snapshot.rs.
    let mut a = empty_msg(vec![1]);
    let mut b = empty_msg(vec![2]);
    // Same body, same header, different sender.
    a.body.state.block_number = 7;
    b.body.state.block_number = 7;
    let ha = compute_block_hash(&a);
    let hb = compute_block_hash(&b);
    assert_ne!(ha, hb);
}

#[test]
fn different_bodies_produce_different_block_hashes_for_same_sender() {
    let mut a = empty_msg(vec![1]);
    let mut b = empty_msg(vec![1]);
    a.body.state.block_number = 1;
    b.body.state.block_number = 2;
    assert_ne!(compute_block_hash(&a), compute_block_hash(&b));
}

#[test]
fn different_shard_ids_produce_different_block_hashes() {
    let mut a = empty_msg(vec![1]);
    let mut b = empty_msg(vec![1]);
    a.shard_id = "root".to_string();
    b.shard_id = "child".to_string();
    assert_ne!(compute_block_hash(&a), compute_block_hash(&b));
}

#[test]
fn block_hash_is_independent_of_bond_ordering() {
    use cordial_f1r3node_adapter::block_translation::Bond as MirrorBond;
    let mut a = empty_msg(vec![1]);
    let mut b = empty_msg(vec![1]);
    a.body.state.bonds = vec![
        MirrorBond {
            validator: vec![10],
            stake: 100,
        },
        MirrorBond {
            validator: vec![20],
            stake: 200,
        },
    ];
    b.body.state.bonds = vec![
        MirrorBond {
            validator: vec![20],
            stake: 200,
        },
        MirrorBond {
            validator: vec![10],
            stake: 100,
        },
    ];
    assert_eq!(compute_block_hash(&a), compute_block_hash(&b));
}

#[test]
fn block_hash_changes_when_bond_stake_changes() {
    use cordial_f1r3node_adapter::block_translation::Bond as MirrorBond;
    let mut a: BlockMessage = empty_msg(vec![1]);
    let mut b = empty_msg(vec![1]);
    a.body.state.bonds = vec![MirrorBond {
        validator: vec![10],
        stake: 100,
    }];
    b.body.state.bonds = vec![MirrorBond {
        validator: vec![10],
        stake: 101,
    }];
    assert_ne!(compute_block_hash(&a), compute_block_hash(&b));
}

#[test]
fn block_hash_includes_system_deploys() {
    let a = empty_msg(vec![1]);
    let mut b = empty_msg(vec![1]);
    b.body
        .system_deploys
        .push(ProcessedSystemDeploy::CloseBlock { succeeded: true });
    assert_ne!(compute_block_hash(&a), compute_block_hash(&b));

    // Different system deploy variant tags hash differently
    let mut c = empty_msg(vec![1]);
    c.body.system_deploys.push(ProcessedSystemDeploy::Slash {
        validator: vec![99],
        succeeded: true,
    });
    assert_ne!(compute_block_hash(&b), compute_block_hash(&c));
}

// ── F1r3flyCryptoAdapter tests ───────────────────────────────────────────
// sk means private key and pk means public key
// I used core_hash since it comes from cordial_miners_core::crypto::hash_content
fn make_content(payload: &[u8]) -> BlockContent {
    BlockContent {
        payload: payload.to_vec(),
        predecessors: HashSet::new(),
    }
}
// SECP256K1 TESTS
// TEST 1 — Acceptance Criterion : "Adapter returns Ok(()) for a valid signature."
#[test]
fn adapter_secp256k1_valid_signature_returns_ok() {
    let (sk, pk) = secp256k1_keypair();
    let content = make_content(b"test block");
    let hash = core_hash(&content);
    let signature = Secp256k1.sign(&hash, &sk).unwrap();
    let creator = NodeId(pk);
    let adapter = F1r3flyCryptoAdapter::new(SigAlgorithm::Secp256k1);
    assert!(adapter.verify_block(&content, &signature, &creator).is_ok());
}

// TEST 2 — Acceptance Criterion : "Adapter returns CryptoError for an corrupted signature."
#[test]
fn adapter_secp256k1_corrupted_signature_returns_err() {
    let (sk, pk) = secp256k1_keypair();
    let content = make_content(b"test block");
    let hash = core_hash(&content);
    let mut signature = Secp256k1.sign(&hash, &sk).unwrap();
    *signature.last_mut().unwrap() ^= 0xff; //flip last byte
    let creator = NodeId(pk);
    let adapter = F1r3flyCryptoAdapter::new(SigAlgorithm::Secp256k1);
    assert!(
        adapter
            .verify_block(&content, &signature, &creator)
            .is_err()
    );
}

// TEST 3 — acceptance criterion : "The adapter returns Err for a block with a forged signature." forged means pretending real creator".
#[test]
fn adapter_secp256k1_forged_signature_returns_err() {
    let (sk, pk) = secp256k1_keypair();
    let content = make_content(b"test block");
    let hash = core_hash(&content);
    let _signature = Secp256k1.sign(&hash, &sk).unwrap();
    // let try our fake creater trying forging the content
    let (forger_sk, _forger_pk) = secp256k1_keypair();
    let forged_signature = Secp256k1.sign(&hash, &forger_sk).unwrap();
    let creator = NodeId(pk); //real creator
    let adapter = F1r3flyCryptoAdapter::new(SigAlgorithm::Secp256k1);
    assert!(
        adapter
            .verify_block(&content, &forged_signature, &creator)
            .is_err()
    );
}

// TEST 4 — acceptance criterion : "An empty signature slice (zero bytes) must always be rejected".
#[test]
fn adapter_secp256k1_empty_signature_returns_err() {
    let (_sk, pk) = secp256k1_keypair();
    let content = make_content(b"test block");
    let creator = NodeId(pk);
    let adapter = F1r3flyCryptoAdapter::new(SigAlgorithm::Secp256k1);
    assert!(adapter.verify_block(&content, &[], &creator).is_err());
}

// TEST 5 — acceptance criterion: "Tampered_content with the original signature must always be rejected".
#[test]
fn adapter_secp256k1_tampered_content_returns_err() {
    let (sk, pk) = secp256k1_keypair();
    let content = make_content(b"test block");
    let hash = core_hash(&content);
    let signature = Secp256k1.sign(&hash, &sk).unwrap();

    let tampered_content = make_content(b"I changed but I used the previous signature");
    let creator = NodeId(pk);
    let adapter = F1r3flyCryptoAdapter::new(SigAlgorithm::Secp256k1);
    assert!(
        adapter
            .verify_block(&tampered_content, &signature, &creator)
            .is_err()
    );
}

// ED25519 TESTS
// TEST 6 — acceptance criterion: "A valid Ed25519 signature must return Ok(())".
#[test]
fn adapter_ed25519_valid_signature_returns_ok() {
    let (sk, pk) = ed25519_keypair();
    let content = make_content(b"test block for ed25519");
    let hash = core_hash(&content);
    let signature = Ed25519.sign(&hash, &sk).unwrap();
    let creator = NodeId(pk.to_vec());
    let adapter = F1r3flyCryptoAdapter::new(SigAlgorithm::Ed25519);
    assert!(adapter.verify_block(&content, &signature, &creator).is_ok());
}

// TEST 7 — acceptance criterion: "A corrupted Ed25519 signature must return Err".
#[test]
fn adapter_ed25519_corrupted_signature_returns_err() {
    let (sk, pk) = ed25519_keypair();
    let content = make_content(b"test block for ed25519");
    let hash = core_hash(&content);
    let mut signature = Ed25519.sign(&hash, &sk).unwrap();
    signature[0] ^= 0xff; //flip the first byte
    let creator = NodeId(pk.to_vec());
    let adapter = F1r3flyCryptoAdapter::new(SigAlgorithm::Ed25519);
    assert!(
        adapter
            .verify_block(&content, &signature, &creator)
            .is_err()
    );
}

// TEST 8 — acceptance criterion : "The adapter returns Err for an Ed25519 block with a forged signature." forged means pretending real creator".
#[test]
fn adapter_ed25519_forged_signature_returns_err() {
    let (_sk, pk) = ed25519_keypair();
    let content = make_content(b"test block");
    let hash = core_hash(&content);

    // let try our fake creator trying forging the content
    let (forger_sk, _forger_private_key) = ed25519_keypair();
    let forged_signature = Ed25519.sign(&hash, &forger_sk).unwrap();

    let creator = NodeId(pk.to_vec()); // real creator
    let adapter = F1r3flyCryptoAdapter::new(SigAlgorithm::Ed25519);
    assert!(
        adapter
            .verify_block(&content, &forged_signature, &creator)
            .is_err()
    );
}

// TEST 9 — acceptance criterion : "An empty Ed25519 signature slice (zero bytes) must always be rejected".
#[test]
fn ed25519_empty_signature_returns_err() {
    let (_sk, pk) = ed25519_keypair();
    let content = make_content(b"test block");
    let creator = NodeId(pk.to_vec());

    let adapter = F1r3flyCryptoAdapter::new(SigAlgorithm::Ed25519);
    assert!(adapter.verify_block(&content, &[], &creator).is_err());
}

// TEST 10 — acceptance criterion : "If Ed25519 content is changed after signing, verification must fail".
#[test]
fn ed25519_tampered_content_returns_err() {
    let (sk, pk) = ed25519_keypair();
    let orginal_content = make_content(b"test block");
    let hash = core_hash(&orginal_content);
    let signature = Ed25519.sign(&hash, &sk).unwrap();

    // Try to verify same signature but with tampered content
    let tampered_content = make_content(b"Tampered content");
    let creator = NodeId(pk.to_vec());
    let adapter = F1r3flyCryptoAdapter::new(SigAlgorithm::Ed25519);
    assert!(
        adapter
            .verify_block(&tampered_content, &signature, &creator)
            .is_err()
    );
}

// TEST 11 — acceptance criterion : "\"secp256k1\" string must create a Secp256k1 adapter".
#[test]
fn adapter_from_algorithm_str_works() {
    assert_eq!(
        F1r3flyCryptoAdapter::from_algorithm_str("") //empty string by default secp256k1 like in f1r3node
            .unwrap()
            .algorithm(),
        SigAlgorithm::Secp256k1
    );
    assert_eq!(
        F1r3flyCryptoAdapter::from_algorithm_str("secp256k1")
            .unwrap()
            .algorithm(),
        SigAlgorithm::Secp256k1
    );
    assert_eq!(
        F1r3flyCryptoAdapter::from_algorithm_str("ed25519")
            .unwrap()
            .algorithm(),
        SigAlgorithm::Ed25519
    );
    assert!(F1r3flyCryptoAdapter::from_algorithm_str("rsa").is_err());
}
