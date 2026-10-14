use std::{collections::HashSet, fs};

use cordial_f1r3node_adapter::por::{
    ratings::rating_signing_hash,
    transport::{
        RatingEnvelopeBroadcaster, file_exchange::FileRatingExchange,
        wire::BlockProductionRatingEnvelopeV1,
    },
};
use cordial_miners_core::{
    NodeId,
    crypto::{Secp256k1Scheme, SignatureScheme},
};
use cordial_por::RatingRecord;
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
