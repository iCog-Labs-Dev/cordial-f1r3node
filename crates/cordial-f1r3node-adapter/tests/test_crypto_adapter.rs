// Tests for F1r3flyCryptoAdapter.
// Each test: create & sign block then run verify then check result.
// Run: cargo test -p cordial-f1r3node-adapter --test test_crypto_adapter (for test file)

use std::collections::HashSet;

// Import the things we want to test.
use cordial_f1r3node_adapter::crypto_impl::{CryptoAlgorithm, F1r3flyCryptoAdapter};

// Import the tools we need to create test data
use cordial_miners_core::{
    crypto::{CryptoVerifier, hash_content, sign},
    types::{BlockContent, NodeId},
};

//--------------------------------------------------------------------
// Helper function used to create a simple block content with a given payload and no parent block
//--------------------------------------------------------------------
//instead of writing the same setup in every test,we implemented this make content function.
fn make_content(payload: &[u8]) -> BlockContent {
    BlockContent {
        payload: payload.to_vec(), // the data we want to put in the block changed to vec of bytes
        predecessors: HashSet::new(), // no parent blocks for simplicity (for testing)
    }
}

// Generates a new Secp256k1 key pair which Returns: (private_key as bytes, public_key as bytes)
// Private key = 32 bytes (the secret, used to sign)
// Public key  = 33 bytes compressed (the shareable one, used to verify

fn secp256k1_keypair() -> (Vec<u8>, Vec<u8>) {
    use k256::ecdsa::SigningKey;

    // Generate a random signing key (private key) using OS randomness
    let signing_key = SigningKey::random(&mut rand::rngs::OsRng);
    // Get Private Key as bytes
    let private_key = signing_key.to_bytes().to_vec();
    // Get the corresponding public key as 33-byte compressed format
    let public_key = signing_key
        .verifying_key()
        .to_encoded_point(true) // true means compressed representation
        .as_bytes()
        .to_vec();

    // Return the key pair as (private_key, public_key)
    (private_key, public_key)
}

// Generates a fresh Ed25519 key pair. Returns: (private_key, public_key)
// Private key = 32 bytes (the secret, used to sign)
// Public key  = 32 bytes (the shareable one, used to verify)
fn ed25519_keypair() -> (Vec<u8>, Vec<u8>) {
    use ed25519_dalek::SigningKey;

    // Generate a random signing key (private key) using OS randomness
    let signing_key = SigningKey::generate(&mut rand::rngs::OsRng);
    // Get Private Key as bytes
    let private_key = signing_key.to_bytes().to_vec();
    // Get Public Key as bytes
    let public_key = signing_key.verifying_key().to_bytes().to_vec();

    // Return the key pair as (private_key, public_key)
    (private_key, public_key)
}

//----------------------------------------------------
// SECP256K1 TESTS
//----------------------------------------------------
// TEST 1 — Acceptance Criterion 1: "Adapter returns Ok(()) for a valid signature."
// Technical Steps:
//   - Generate key pair (private , public)
//   - Sign content
//   - Verify with adapter
//   - Expect Ok(())

#[test]
fn secp256k1_valid_signature_returns_ok() {
    // Create Content and hash it
    let content = make_content(b" This is test block we made for testing the secp256k1 signature verification in F1r3flyCryptoAdapter."); // using byte string literal for payload(content data)
    let hash = hash_content(&content);

    // Generate a real key pair and sign.
    // `sign()` is found in cordial_miners_core::crypto and  It uses Secp256k1Scheme internally and returns DER-encoded bytes
    let (private_key, public_key) = secp256k1_keypair();
    let signature = sign(&hash, &private_key);

    // Wrap the public key in a NodeId struct, as expected by the adapter.
    let creator = NodeId(public_key);

    // Create the adapter configured for Secp256k1 and verify the block.
    let adapter = F1r3flyCryptoAdapter::secp256k1();
    let result = adapter.verify_block(&content, &signature, &creator);

    // Assert that the result is Ok(()), meaning the signature was valid.
    assert!(
        result.is_ok(),
        "Expected Ok(()), got Err: {:?}",
        result.err()
    );
}

// TEST 2 — Acceptance Criterion 2: "Adapter returns Err(msg) for an corrupted signature."
// "The adapter returns Err for a block with a corrupted signature."
//
// Technical Steps:
//   - Same setup as test 1
//   - But flip one byte in the signature before verifying
//   - Expect Err

#[test]
fn secp256k1_corrupted_signature_returns_err() {
    // Create Content and hash it
    let content = make_content(b" This is test block we made for testing the secp256k1 signature verification in F1r3flyCryptoAdapter."); // using byte string literal for payload(content data)
    let hash = hash_content(&content);

    // Generate a real key pair and sign.
    let (private_key, public_key) = secp256k1_keypair();
    let mut signature = sign(&hash, &private_key);

    // Flip the last byte intentionally.
    let last_byte = signature.last_mut().unwrap(); // safe because signature should never be empty as we defined out logic inside crypto_impl.rs to return error if signature is empty.
    *last_byte = last_byte.wrapping_add(1);

    // Create the adapter configured for Secp256k1 and verify the block.
    let adapter = F1r3flyCryptoAdapter::secp256k1();
    let creator = NodeId(public_key);
    let result = adapter.verify_block(&content, &signature, &creator);

    // Assert that the result is Err, meaning the signature was invalid.
    assert!(result.is_err(), "Expected Err, got Ok(())");
}

// TEST 3 — acceptance criterion 2:
// "The adapter returns Err for a block with a forged signature."
// "Forged" in this context means: someone else signed it pretending to be the real creator.
// The signature bytes are valid on their own, but for a different key.

#[test]
fn secp256k1_forged_signature_returns_err() {
    let content = make_content(b" This is test block we made for testing the secp256k1 signature verification in F1r3flyCryptoAdapter."); // using byte string literal for payload(content data)
    let hash = hash_content(&content);

    // Real creator's key pair and signature
    let (_real_private_key, real_public_key) = secp256k1_keypair();

    // Forger's key pair and signature (forging the content)
    let (forger_private_key, _forger_public_key) = secp256k1_keypair();
    let forged_signature = sign(&hash, &forger_private_key);

    // The block is claiming to be from the real creator, but the signature is from the forger.
    let creator = NodeId(real_public_key);
    let adapter = F1r3flyCryptoAdapter::secp256k1();
    let result = adapter.verify_block(&content, &forged_signature, &creator);

    assert!(
        result.is_err(),
        "Expected Err for forged signature, got Ok(())"
    );
}

// TEST 4:
// An empty signature slice (zero bytes) must always be rejected.
#[test]
fn secp256k1_empty_signature_returns_err() {
    let content = make_content(b" This is test block we made for testing the secp256k1 signature verification in F1r3flyCryptoAdapter."); // using byte string literal for payload(content data)
    let (_private_key, public_key) = secp256k1_keypair();
    let creator = NodeId(public_key);

    let adapter = F1r3flyCryptoAdapter::secp256k1();
    let result = adapter.verify_block(&content, &[], &creator); // empty signature

    assert!(
        result.is_err(),
        "Expected Err for empty signature, got Ok(())"
    );

    // check the error message contains the word "empty" so it's readable.
    let err_msg = result.unwrap_err();
    assert!(
        err_msg.contains("empty"),
        "Error should say 'empty', got: {err_msg}"
    );
}

// TEST 5:
//If the content was tampered with after signing, verification fails.
// This covers the scenario where an attacker modifies the block data
// but keeps the original signature. The hash won't match anymore.
#[test]
fn secp256k1_tampered_content_returns_err() {
    let orginal_content = make_content(b" This is test block we made for testing the secp256k1 signature verification in F1r3flyCryptoAdapter."); // using byte string literal for payload(content data)
    let hash = hash_content(&orginal_content);
    let (private_key, public_key) = secp256k1_keypair();
    let signature = sign(&hash, &private_key);

    // Try to verify same signature but with tampered content
    let tampered_content = make_content(b" Tampered content"); // different content
    let creator = NodeId(public_key);
    let adapter = F1r3flyCryptoAdapter::secp256k1();
    let result = adapter.verify_block(&tampered_content, &signature, &creator);

    assert!(
        result.is_err(),
        "Expected Err for tampered content, got Ok(())"
    );
}

//----------------------------------------------------
// ED25519 TESTS
//----------------------------------------------------
// TEST 6 — Acceptance criterion 1 for Ed25519:
// A valid Ed25519 signature must return Ok(())
#[test]
fn ed25519_valid_signature_returns_ok() {
    use ed25519_dalek::Signer; // needed to call .sign() on an Ed25519 key

    let content = make_content(b"ed25519 test block");
    let hash = hash_content(&content);

    // Generate a key pair and sign.
    let (private_key_bytes, public_key_bytes) = ed25519_keypair();
    let key_array: [u8; 32] = private_key_bytes.try_into().unwrap();
    let signing_key = ed25519_dalek::SigningKey::from_bytes(&key_array);
    let signature = signing_key.sign(&hash).to_bytes().to_vec();

    let creator = NodeId(public_key_bytes);
    let adapter = F1r3flyCryptoAdapter::ed25519();
    let result = adapter.verify_block(&content, &signature, &creator);

    assert!(result.is_ok(), "Expected Ok(()), got: {:?}", result);
}

// TEST 7 — Acceptance criterion 2 for Ed25519:
// A corrupted Ed25519 signature must return Err.
#[test]
fn ed25519_corrupted_signature_returns_err() {
    use ed25519_dalek::Signer;

    let content = make_content(b"This is test block we made for testing the ed25519 signature verification in F1r3flyCryptoAdapter.");
    let hash = hash_content(&content);

    let (private_key, public_key) = ed25519_keypair();
    let key_array: [u8; 32] = private_key.try_into().unwrap();
    let signing_key = ed25519_dalek::SigningKey::from_bytes(&key_array);
    let mut signature = signing_key.sign(&hash).to_bytes().to_vec();

    // Flip the first byte.
    signature[0] = signature[0].wrapping_add(1);
    let creator = NodeId(public_key);
    let adapter = F1r3flyCryptoAdapter::ed25519();
    let result = adapter.verify_block(&content, &signature, &creator);

    assert!(
        result.is_err(),
        "Expected Err for corrupted Ed25519 signature"
    );
}

//----------------------------------------------------
// from_algorithm_str TESTS
//----------------------------------------------------
// TEST 8: "secp256k1" string → Secp256k1 adapter.
#[test]
fn from_str_secp256k1_gives_secp256k1_adapter() {
    let adapter = F1r3flyCryptoAdapter::from_algorithm_str("secp256k1").unwrap();
    assert_eq!(adapter.algorithm(), CryptoAlgorithm::Secp256k1);
}

// TEST 9: "ed25519" string → Ed25519 adapter.
#[test]
fn from_str_ed25519_gives_ed25519_adapter() {
    let adapter = F1r3flyCryptoAdapter::from_algorithm_str("ed25519").unwrap();
    assert_eq!(adapter.algorithm(), CryptoAlgorithm::Ed25519);
}

// TEST 10: empty string is defaults to Secp256k1.
#[test]
fn from_str_empty_defaults_to_secp256k1() {
    let adapter = F1r3flyCryptoAdapter::from_algorithm_str("").unwrap();
    assert_eq!(adapter.algorithm(), CryptoAlgorithm::Secp256k1);
}

// TEST 11: unknown algorithm string → Err.
#[test]
fn from_str_unknown_returns_err() {
    let result = F1r3flyCryptoAdapter::from_algorithm_str("rsa");
    assert!(result.is_err(), "Expected Err for unknown algorithm 'rsa'");

    let err = result.unwrap_err();
    // The error should mention what we received as its mentioned.
    assert!(
        err.contains("rsa"),
        "Error should mention 'rsa', got: {err}"
    );
}
