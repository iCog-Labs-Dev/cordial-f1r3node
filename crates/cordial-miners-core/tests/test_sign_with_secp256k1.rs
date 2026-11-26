use cordial_miners_core::crypto::{block_hash, sign, verify};
use cordial_miners_core::{BlockContent, NodeId};
use k256::ecdsa::{Signature, SigningKey};
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
    let hash = block_hash(&NodeId(vec![1]), &content);
    let signature = sign(&hash, &private_key);
    assert!(
        verify(&hash, &public_key, &signature),
        "Default Secp256k1 verification failed"
    );
}

#[test]
fn secp_signature_uses_canonical_der_encoding() {
    let (private_key, public_key) = generate_secp_keypair();
    let hash = [0xab; 32];
    let signature = sign(&hash, &private_key);
    // DER integers omit leading zero bytes, so valid signatures can be shorter
    // than the usual 70-72 bytes. Check the encoding itself, not that range.
    let parsed = Signature::from_der(&signature).expect("signature must be DER encoded");
    assert_eq!(parsed.to_der().as_bytes(), signature.as_slice());
    assert!(verify(&hash, &public_key, &signature));
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
