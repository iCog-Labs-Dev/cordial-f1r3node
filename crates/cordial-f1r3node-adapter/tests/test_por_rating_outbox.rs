use std::{
    fs,
    sync::{Arc, Barrier},
};

use cordial_f1r3node_adapter::{
    por::transport::outbox::RatingOutbox, por_rating_transport::encode_rating_batch,
    por_ratings::rating_signing_hash,
};
use cordial_miners_core::{
    NodeId,
    crypto::{Secp256k1Scheme, SignatureScheme},
};
use cordial_por::{RatingBatch, RatingRecord};
use k256::ecdsa::SigningKey;

fn node(seed: u8) -> NodeId {
    let key = SigningKey::from_slice(&[seed; 32]).unwrap();
    NodeId(
        key.verifying_key()
            .to_encoded_point(true)
            .as_bytes()
            .to_vec(),
    )
}

fn batch(score: u64) -> RatingBatch {
    let mut rating = RatingRecord {
        round: 1,
        rater: node(1),
        recipient: node(2),
        score,
        signature: Vec::new(),
        interaction_ref: Some(vec![7; 32]),
    };
    rating.signature = Secp256k1Scheme
        .sign(&rating_signing_hash(&rating).unwrap(), &[1; 32])
        .unwrap();
    RatingBatch {
        round: 1,
        ratings: vec![rating],
    }
}

#[test]
fn signed_local_batch_survives_restart_and_conflicts_fail_closed() {
    let directory = tempfile::tempdir().unwrap();
    let rater = node(1);
    let first = batch(50);
    let expected = encode_rating_batch(0, &first).unwrap();
    let outbox = RatingOutbox::open(directory.path()).unwrap();

    assert!(outbox.load(0, &rater).unwrap().is_none());
    outbox.persist_local_batch(0, &rater, &first).unwrap();
    outbox.persist_local_batch(0, &rater, &first).unwrap();
    drop(outbox);

    let reopened = RatingOutbox::open(directory.path()).unwrap();
    assert_eq!(reopened.load(0, &rater).unwrap(), Some(expected));
    assert!(reopened.persist_local_batch(0, &rater, &batch(51)).is_err());
    assert_eq!(
        reopened.load(0, &rater).unwrap(),
        Some(encode_rating_batch(0, &first).unwrap())
    );
}

#[test]
fn rejects_wrong_rater_or_corrupted_durable_batch() {
    let directory = tempfile::tempdir().unwrap();
    let outbox = RatingOutbox::open(directory.path()).unwrap();
    let rater = node(1);
    assert!(outbox.persist_local_batch(0, &node(3), &batch(50)).is_err());
    let mut invalid = batch(50);
    invalid.ratings[0].score += 1;
    assert!(outbox.persist_local_batch(0, &rater, &invalid).is_err());
    assert!(outbox.load(0, &rater).unwrap().is_none());
    outbox.persist_local_batch(0, &rater, &batch(50)).unwrap();

    let path = fs::read_dir(directory.path().join("por/rating-outbox"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    fs::write(&path, b"truncated").unwrap();
    assert!(outbox.load(0, &rater).is_err());
}

#[test]
fn empty_local_batch_has_a_durable_round_marker() {
    let directory = tempfile::tempdir().unwrap();
    let outbox = RatingOutbox::open(directory.path()).unwrap();
    let rater = node(1);
    let empty = RatingBatch {
        round: 1,
        ratings: Vec::new(),
    };
    outbox.persist_local_batch(0, &rater, &empty).unwrap();
    assert_eq!(outbox.load(0, &rater).unwrap(), Some(Vec::new()));
}

#[test]
fn concurrent_conflicting_writers_cannot_replace_the_first_batch() {
    let directory = tempfile::tempdir().unwrap();
    let barrier = Arc::new(Barrier::new(2));
    let results = std::thread::scope(|scope| {
        let jobs: Vec<_> = [50, 51]
            .into_iter()
            .map(|score| {
                let barrier = barrier.clone();
                let data_dir = directory.path();
                scope.spawn(move || {
                    let outbox = RatingOutbox::open(data_dir).unwrap();
                    barrier.wait();
                    outbox
                        .persist_local_batch(0, &node(1), &batch(score))
                        .is_ok()
                })
            })
            .collect();
        jobs.into_iter()
            .map(|job| job.join().unwrap())
            .collect::<Vec<_>>()
    });
    assert_eq!(results.into_iter().filter(|accepted| *accepted).count(), 1);
    let stored = RatingOutbox::open(directory.path())
        .unwrap()
        .load(0, &node(1))
        .unwrap()
        .unwrap();
    assert!(
        stored == encode_rating_batch(0, &batch(50)).unwrap()
            || stored == encode_rating_batch(0, &batch(51)).unwrap()
    );
}
