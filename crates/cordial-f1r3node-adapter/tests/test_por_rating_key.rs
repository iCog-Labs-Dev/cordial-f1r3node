use std::{collections::HashMap, fs};

use cordial_f1r3node_adapter::por::rating_key::load_bonded_rating_key;
use cordial_miners_core::NodeId;
use k256::ecdsa::SigningKey;

fn keys(seed: u8) -> (NodeId, NodeId) {
    let signing_key = SigningKey::from_slice(&[seed; 32]).unwrap();
    let verifying_key = signing_key.verifying_key();
    (
        NodeId(verifying_key.to_encoded_point(true).as_bytes().to_vec()),
        NodeId(verifying_key.to_encoded_point(false).as_bytes().to_vec()),
    )
}

fn key_file(directory: &tempfile::TempDir, encoded: &str) -> std::path::PathBuf {
    let path = directory.path().join("validator.key");
    fs::write(&path, encoded).unwrap();
    path
}

#[test]
fn accepts_only_the_bonded_public_key_encoding() {
    let directory = tempfile::tempdir().unwrap();
    let path = key_file(&directory, &format!("{}\n", hex::encode([1; 32])));
    let (compressed, uncompressed) = keys(1);

    let signer =
        load_bonded_rating_key(&path, &HashMap::from([(uncompressed.clone(), 1)])).unwrap();
    assert_eq!(signer.rater(), &uncompressed);
    assert_eq!(signer.private_key_bytes(), &[1; 32]);

    let signer = load_bonded_rating_key(&path, &HashMap::from([(compressed.clone(), 1)])).unwrap();
    assert_eq!(signer.rater(), &compressed);
}

#[test]
fn rejects_unbonded_ambiguous_or_invalid_keys() {
    let directory = tempfile::tempdir().unwrap();
    let secret = hex::encode([1; 32]);
    let path = key_file(&directory, &secret);
    let (compressed, uncompressed) = keys(1);
    let unrelated = keys(2).1;

    let error = load_bonded_rating_key(&path, &HashMap::from([(unrelated, 1)]))
        .err()
        .unwrap()
        .to_string();
    assert!(error.contains("not in the authorized bonds"));
    assert!(!error.contains(&secret));

    assert!(
        load_bonded_rating_key(&path, &HashMap::from([(compressed, 1), (uncompressed, 1)]),)
            .is_err()
    );
    key_file(&directory, "not-a-private-key");
    assert!(load_bonded_rating_key(&path, &HashMap::new()).is_err());
    key_file(&directory, &"00".repeat(32));
    assert!(load_bonded_rating_key(&path, &HashMap::new()).is_err());
}
