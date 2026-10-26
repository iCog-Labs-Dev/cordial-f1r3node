use std::{
    collections::HashSet,
    fs,
    sync::{Arc, Barrier},
};

use cordial_f1r3node_adapter::por::{
    ratings::{checkpoint_rating_signing_hash, rating_signing_hash},
    transport::{
        RatingEnvelopeBroadcaster,
        file_exchange::FileRatingExchange,
        wire::{BlockProductionRatingEnvelopeV1, BlockProductionRatingEnvelopeV2},
    },
};
use cordial_miners_core::{
    NodeId,
    crypto::{Secp256k1Scheme, SignatureScheme},
};
use cordial_por::{RatingRecord, RatingSourceCheckpoint};
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

fn signed_envelope(rater: u8, recipient: u8, score: u64) -> Vec<u8> {
    let mut rating = RatingRecord {
        round: 1,
        rater: node(rater),
        recipient: node(recipient),
        score,
        signature: Vec::new(),
        interaction_ref: Some(vec![7; 32]),
    };
    rating.signature = Secp256k1Scheme
        .sign(&rating_signing_hash(&rating).unwrap(), &[rater; 32])
        .unwrap();
    BlockProductionRatingEnvelopeV1::new(0, rating)
        .unwrap()
        .encode()
}

fn source(hash: u8) -> RatingSourceCheckpoint {
    RatingSourceCheckpoint {
        round: 1,
        finalized_prefix_len: 16,
        finalized_prefix_hash: [hash; 32],
    }
}

fn checkpoint_envelope(
    source: RatingSourceCheckpoint,
    rater: u8,
    recipient: u8,
    score: u64,
) -> Vec<u8> {
    let mut rating = RatingRecord {
        round: source.round,
        rater: node(rater),
        recipient: node(recipient),
        score,
        signature: Vec::new(),
        interaction_ref: Some(vec![7; 32]),
    };
    rating.signature = Secp256k1Scheme
        .sign(
            &checkpoint_rating_signing_hash(&rating, &source).unwrap(),
            &[rater; 32],
        )
        .unwrap();
    BlockProductionRatingEnvelopeV2::new(source, rating)
        .unwrap()
        .encode()
}

#[test]
fn signed_peer_envelopes_survive_restart_and_read_in_key_order() {
    let directory = tempfile::tempdir().unwrap();
    let authorized = HashSet::from([node(1), node(2), node(3), node(4)]);
    let exchange = FileRatingExchange::open(directory.path(), b"root").unwrap();
    let first = signed_envelope(1, 3, 50);
    let second = signed_envelope(2, 4, 60);
    let third = signed_envelope(2, 3, 70);
    let local = exchange.broadcaster(0, node(1));
    let peer = exchange.broadcaster(0, node(2));
    local.broadcast_rating_envelope(&first).unwrap();
    peer.broadcast_rating_envelope(&second).unwrap();
    peer.broadcast_rating_envelope(&third).unwrap();
    peer.broadcast_rating_envelope(&second).unwrap();
    drop(exchange);

    let recovered = FileRatingExchange::open(directory.path(), b"root").unwrap();
    let mut expected = vec![second, third];
    expected.sort_by_key(|encoded| {
        BlockProductionRatingEnvelopeV1::decode(encoded)
            .unwrap()
            .rating()
            .recipient
            .clone()
    });
    assert_eq!(
        recovered
            .read_peer_envelopes(0, &node(1), &authorized)
            .unwrap(),
        expected
    );
    assert!(
        recovered
            .read_peer_envelopes(1, &node(1), &authorized)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn rejects_conflicting_retry_wrong_author_and_corrupt_peer_file() {
    let directory = tempfile::tempdir().unwrap();
    let exchange = FileRatingExchange::open(directory.path(), b"root").unwrap();
    let broadcaster = exchange.broadcaster(0, node(1));
    let original = signed_envelope(1, 2, 50);
    broadcaster.publish(&original).unwrap();
    assert!(broadcaster.publish(&signed_envelope(1, 2, 51)).is_err());
    assert!(broadcaster.publish(&signed_envelope(2, 1, 50)).is_err());
    assert!(exchange.broadcaster(1, node(1)).publish(&original).is_err());
    let mut invalid = original.clone();
    *invalid.last_mut().unwrap() ^= 1;
    assert!(exchange.broadcaster(0, node(3)).publish(&invalid).is_err());

    let authorized = HashSet::from([node(1), node(2)]);
    let rater_directory = directory
        .path()
        .join(hex::encode(b"root"))
        .join(format!("{:020}", 0))
        .join(hex::encode(node(1).0));
    fs::write(
        rater_directory.join(format!("{}.bin", hex::encode(node(2).0))),
        b"broken",
    )
    .unwrap();
    assert!(
        exchange
            .read_peer_envelopes(0, &node(2), &authorized)
            .is_err()
    );
}

#[test]
fn rejects_unauthorized_rater_and_isolates_shards() {
    let directory = tempfile::tempdir().unwrap();
    let root = FileRatingExchange::open(directory.path(), b"root").unwrap();
    root.broadcaster(0, node(9))
        .publish(&signed_envelope(9, 2, 50))
        .unwrap();
    let authorized = HashSet::from([node(1), node(2)]);
    assert!(root.read_peer_envelopes(0, &node(1), &authorized).is_err());

    let other = FileRatingExchange::open(directory.path(), b"other").unwrap();
    assert!(
        other
            .read_peer_envelopes(0, &node(1), &authorized)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn checkpoint_exchange_recovers_order_and_rejects_conflicting_source() {
    let directory = tempfile::tempdir().unwrap();
    let first = source(4);
    let exchange = FileRatingExchange::open(directory.path(), b"root").unwrap();
    let peer = exchange.checkpoint_broadcaster(first, node(2)).unwrap();
    let a = checkpoint_envelope(first, 2, 3, 50);
    let b = checkpoint_envelope(first, 2, 4, 60);
    peer.publish(&b).unwrap();
    peer.publish(&a).unwrap();
    peer.publish(&a).unwrap();
    drop(exchange);
    let recovered = FileRatingExchange::open(directory.path(), b"root").unwrap();
    let authorized = HashSet::from([node(1), node(2), node(3), node(4)]);
    assert_eq!(
        recovered
            .read_checkpoint_peer_envelopes(first, &node(1), &authorized)
            .unwrap(),
        vec![a, b]
    );
    assert!(
        recovered
            .read_checkpoint_peer_envelopes(source(5), &node(1), &authorized)
            .is_err()
    );
    let conflicting = recovered
        .checkpoint_broadcaster(source(5), node(3))
        .unwrap();
    assert!(
        conflicting
            .publish(&checkpoint_envelope(source(5), 3, 4, 50))
            .is_err()
    );
    assert!(peer.publish(&checkpoint_envelope(first, 2, 3, 51)).is_err());
    assert!(peer.publish(&signed_envelope(2, 3, 50)).is_err());
}

#[test]
fn checkpoint_exchange_rejects_corrupt_marker_and_envelope() {
    let directory = tempfile::tempdir().unwrap();
    let first = source(4);
    let exchange = FileRatingExchange::open(directory.path(), b"root").unwrap();
    exchange
        .checkpoint_broadcaster(first, node(2))
        .unwrap()
        .publish(&checkpoint_envelope(first, 2, 3, 50))
        .unwrap();
    let authorized = HashSet::from([node(1), node(2), node(3)]);
    let round = directory
        .path()
        .join(hex::encode(b"root"))
        .join("checkpoint-v2")
        .join(format!("{:020}", first.round));
    let envelope_path = round
        .join(hex::encode(node(2).0))
        .join(format!("{}.bin", hex::encode(node(3).0)));
    fs::write(&envelope_path, b"broken").unwrap();
    assert!(
        exchange
            .read_checkpoint_peer_envelopes(first, &node(1), &authorized)
            .is_err()
    );
    fs::write(round.join("source.bin"), b"broken").unwrap();
    assert!(
        exchange
            .read_checkpoint_peer_envelopes(first, &node(1), &authorized)
            .is_err()
    );
}

#[test]
fn concurrent_checkpoint_sources_cannot_mix_one_round() {
    let directory = tempfile::tempdir().unwrap();
    let barrier = Arc::new(Barrier::new(2));
    let accepted = std::thread::scope(|scope| {
        let jobs: Vec<_> = [(4, 2), (5, 3)]
            .into_iter()
            .map(|(hash, rater)| {
                let barrier = barrier.clone();
                let root = directory.path();
                scope.spawn(move || {
                    let exchange = FileRatingExchange::open(root, b"root").unwrap();
                    let publisher = exchange
                        .checkpoint_broadcaster(source(hash), node(rater))
                        .unwrap();
                    barrier.wait();
                    publisher
                        .publish(&checkpoint_envelope(source(hash), rater, 1, 50))
                        .is_ok()
                })
            })
            .collect();
        jobs.into_iter()
            .map(|job| job.join().unwrap())
            .collect::<Vec<_>>()
    });
    assert_eq!(accepted.into_iter().filter(|ok| *ok).count(), 1);
    let exchange = FileRatingExchange::open(directory.path(), b"root").unwrap();
    let authorized = HashSet::from([node(1), node(2), node(3)]);
    let readable = [source(4), source(5)]
        .into_iter()
        .filter(|source| {
            exchange
                .read_checkpoint_peer_envelopes(*source, &node(1), &authorized)
                .is_ok()
        })
        .count();
    assert_eq!(readable, 1);
}
