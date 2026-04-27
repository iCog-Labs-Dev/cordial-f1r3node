use cordial_miners_core::crypto::{Ed25519Scheme, SignatureScheme};
use ed25519_dalek::SigningKey;
use rand::rngs::OsRng;

/// Helper: generate a random Ed25519 keypair
fn generate_ed_keypair() -> (Vec<u8>, Vec<u8>) {
    let signing_key = SigningKey::generate(&mut OsRng);
    let private = signing_key.to_bytes().to_vec();
    let public = signing_key.verifying_key().to_bytes().to_vec();
    (private, public)
}

#[test]
fn ed25519_sign_and_verify_roundtrip() {
    let (private_key, public_key) = generate_ed_keypair();
    let scheme = Ed25519Scheme;
    let hash = [0x13; 32];
    
    let signature = scheme.sign(&hash, &private_key).expect("Ed sign failed");
    assert!(scheme.verify(&hash, &public_key, &signature));
}

#[test]
fn ed25519_signature_is_exactly_64_bytes() {
    let (private_key, _) = generate_ed_keypair();
    let scheme = Ed25519Scheme;
    let hash = [0xab; 32];
    
    let signature = scheme.sign(&hash, &private_key).unwrap();
    assert_eq!(signature.len(), 64);
}

#[test]
fn ed25519_verify_fails_with_wrong_public_key() {
    let (private_key, _) = generate_ed_keypair();
    let (_, wrong_public_key) = generate_ed_keypair();
    let scheme = Ed25519Scheme;
    let hash = [0x01; 32];
    
    let signature = scheme.sign(&hash, &private_key).unwrap();
    assert!(!scheme.verify(&hash, &wrong_public_key, &signature));
}

#[test]
fn ed25519_verify_fails_with_tampered_signature() {
    let (private_key, public_key) = generate_ed_keypair();
    let scheme = Ed25519Scheme;
    let hash = [0x01; 32];
    
    let mut signature = scheme.sign(&hash, &private_key).unwrap();
    signature[0] ^= 0xff;
    
    assert!(!scheme.verify(&hash, &public_key, &signature));
}

#[test]
fn ed25519_different_hashes_produce_different_signatures() {
    let (private_key, _) = generate_ed_keypair();
    let scheme = Ed25519Scheme;
    let h1 = [0x01; 32];
    let h2 = [0x02; 32];
    
    let sig1 = scheme.sign(&h1, &private_key).unwrap();
    let sig2 = scheme.sign(&h2, &private_key).unwrap();
    assert_ne!(sig1, sig2);
}