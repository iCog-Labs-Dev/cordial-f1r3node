use cordial_miners_core::crypto::{hash_content, sign, verify};
use cordial_miners_core::BlockContent;
use ed25519_dalek::SigningKey;
use rand::rngs::OsRng;
use std::collections::HashSet;

/// Helper: generate a random ED25519 keypair, returning (private_key, public_key) as byte vecs.
fn generate_keypair() -> (Vec<u8>, Vec<u8>) {
    let signing_key = SigningKey::generate(&mut OsRng);
    let private = signing_key.to_bytes().to_vec();
    let public = signing_key.verifying_key().to_bytes().to_vec();
    (private, public)
}

#[test]
fn sign_and_verify_roundtrip() {
    let (private_key, public_key) = generate_keypair();
    let content = BlockContent {
        payload: vec![1, 2, 3],
        predecessors: HashSet::new(),
    };
    let hash = hash_content(&content);
    let signature = sign(&hash, &private_key);
    assert!(verify(&hash, &public_key, &signature));
}

#[test]
fn signature_is_64_bytes() {
    let (private_key, _) = generate_keypair();
    let hash = [0xab; 32];
    let signature = sign(&hash, &private_key);
    assert_eq!(signature.len(), 64);
}

#[test]
fn verify_fails_with_wrong_public_key() {
    let (private_key, _) = generate_keypair();
    let (_, wrong_public_key) = generate_keypair();
    let hash = hash_content(&BlockContent {
        payload: vec![42],
        predecessors: HashSet::new(),
    });
    let signature = sign(&hash, &private_key);
    assert!(!verify(&hash, &wrong_public_key, &signature));
}

#[test]
fn verify_fails_with_tampered_hash() {
    let (private_key, public_key) = generate_keypair();
    let hash = hash_content(&BlockContent {
        payload: vec![1],
        predecessors: HashSet::new(),
    });
    let signature = sign(&hash, &private_key);

    let mut tampered_hash = hash;
    tampered_hash[0] ^= 0xff;
    assert!(!verify(&tampered_hash, &public_key, &signature));
}

#[test]
fn verify_fails_with_tampered_signature() {
    let (private_key, public_key) = generate_keypair();
    let hash = [0x01; 32];
    let mut signature = sign(&hash, &private_key);

    signature[0] ^= 0xff;
    assert!(!verify(&hash, &public_key, &signature));
}

#[test]
fn different_content_produces_different_signatures() {
    let (private_key, _) = generate_keypair();
    let h1 = hash_content(&BlockContent {
        payload: vec![1],
        predecessors: HashSet::new(),
    });
    let h2 = hash_content(&BlockContent {
        payload: vec![2],
        predecessors: HashSet::new(),
    });
    let sig1 = sign(&h1, &private_key);
    let sig2 = sign(&h2, &private_key);
    assert_ne!(sig1, sig2);
}
