use cordial_miners_core::BlockContent;
use cordial_miners_core::crypto::{hash_content, sign, verify};
use k256::ecdsa::SigningKey;
use rand::rngs::OsRng;
use std::collections::HashSet;

/// Helper: generate a random Secp256k1 keypair (Firenode default)
fn generate_secp_keypair() -> (Vec<u8>, Vec<u8>) {
    let signing_key = SigningKey::random(&mut OsRng);
    let private = signing_key.to_bytes().to_vec();
    let public = signing_key.verifying_key().to_sec1_bytes().to_vec(); // Uncompressed SEC1
    (private, public)
}

#[test]
fn secp_sign_and_verify_roundtrip() {
    let (private_key, public_key) = generate_secp_keypair();
    let content = BlockContent {
        payload: vec![1, 2, 3],
        predecessors: HashSet::new(),
    };
    let hash = hash_content(&content);
    let signature = sign(&hash, &private_key);
    assert!(
        verify(&hash, &public_key, &signature),
        "Default Secp256k1 verification failed"
    );
}

#[test]
fn secp_signature_length_is_der_variable() {
    let (private_key, _) = generate_secp_keypair();
    let hash = [0xab; 32];
    let signature = sign(&hash, &private_key);
    // Secp256k1 DER signatures are usually 70-72 bytes, not 64.
    assert!(signature.len() >= 70 && signature.len() <= 72);
}

#[test]
fn secp_verify_fails_with_wrong_public_key() {
    let (private_key, _) = generate_secp_keypair();
    let (_, wrong_public_key) = generate_secp_keypair();
    let hash = [0x42; 32];
    let signature = sign(&hash, &private_key);
    assert!(!verify(&hash, &wrong_public_key, &signature));
}

#[test]
fn secp_verify_fails_with_tampered_hash() {
    let (private_key, public_key) = generate_secp_keypair();
    let hash = [0x01; 32];
    let signature = sign(&hash, &private_key);

    let mut tampered_hash = hash;
    tampered_hash[0] ^= 0xff;
    assert!(!verify(&tampered_hash, &public_key, &signature));
}

#[test]
fn secp_verify_fails_with_tampered_signature() {
    let (private_key, public_key) = generate_secp_keypair();
    let hash = [0x01; 32];
    let mut signature = sign(&hash, &private_key);

    let last_byte = signature.len() - 1;
    signature[last_byte] ^= 0x01; // Tamper with the last byte
    assert!(!verify(&hash, &public_key, &signature));
}
