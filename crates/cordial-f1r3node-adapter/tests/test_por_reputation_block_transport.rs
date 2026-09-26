use std::cell::RefCell;

use cordial_f1r3node_adapter::por::{
    MAX_REPUTATION_BLOCK_PUBLICATION_LEN, PorReputationBlockPublicationError,
    PorReputationBlockTransportError, REPUTATION_BLOCK_PUBLICATION_DOMAIN,
    REPUTATION_BLOCK_PUBLICATION_VERSION, ReputationBlockEnvelopeBroadcaster,
    ReputationBlockPublicationV1, broadcast_reputation_block,
    broadcast_reputation_block_publication, receive_reputation_block_envelope,
    reputation_block_publication_signing_hash,
};
use cordial_miners_core::{
    NodeId,
    crypto::{Secp256k1Scheme, SignatureScheme},
};
use cordial_por::{
    MAX_REPUTATION_BLOCK_WIRE_LEN, PorError, REPUTATION_BLOCK_VERSION, ReputationBlock,
    ReputationBlockHeader, ReputationEntry, ReputationList, encode_reputation_block,
    reputation_list_commitment,
};
use k256::ecdsa::SigningKey;

fn private_key(seed: u8) -> [u8; 32] {
    [seed; 32]
}

fn node(seed: u8) -> NodeId {
    let signing_key = SigningKey::from_slice(&private_key(seed)).unwrap();
    NodeId(
        signing_key
            .verifying_key()
            .to_encoded_point(true)
            .as_bytes()
            .to_vec(),
    )
}

fn uncompressed_node(seed: u8) -> NodeId {
    let signing_key = SigningKey::from_slice(&private_key(seed)).unwrap();
    NodeId(
        signing_key
            .verifying_key()
            .to_encoded_point(false)
            .as_bytes()
            .to_vec(),
    )
}

fn block() -> ReputationBlock {
    let mut entries = vec![
        ReputationEntry {
            node_id: node(2),
            reputation: 40,
            is_excluded: false,
        },
        ReputationEntry {
            node_id: node(3),
            reputation: 60,
            is_excluded: false,
        },
    ];
    entries.sort_by(|left, right| left.node_id.cmp(&right.node_id));
    let reputation_list = ReputationList { round: 1, entries };

    ReputationBlock {
        header: ReputationBlockHeader {
            version: REPUTATION_BLOCK_VERSION,
            shard_id: b"root".to_vec(),
            source_finalized_wave: 0,
            round: 1,
            previous_reputation_hash: None,
            config_hash: [0x11; 32],
            ratings_hash: [0x22; 32],
            reputation_root: reputation_list_commitment(&reputation_list).unwrap(),
        },
        reputation_list,
    }
}

#[derive(Default)]
struct RecordingBroadcaster {
    envelopes: RefCell<Vec<Vec<u8>>>,
    failure: Option<String>,
}

impl ReputationBlockEnvelopeBroadcaster for RecordingBroadcaster {
    fn broadcast_reputation_block_envelope(&self, envelope: &[u8]) -> Result<(), String> {
        if let Some(message) = &self.failure {
            return Err(message.clone());
        }
        self.envelopes.borrow_mut().push(envelope.to_vec());
        Ok(())
    }
}

fn signed_publication() -> ReputationBlockPublicationV1 {
    ReputationBlockPublicationV1::sign(node(7), block(), &private_key(7)).unwrap()
}

#[test]
fn signed_publication_has_a_deterministic_canonical_roundtrip() {
    let publication = signed_publication();
    let repeated = signed_publication();
    assert_eq!(publication, repeated);

    let block_envelope = encode_reputation_block(publication.block()).unwrap();
    let mut expected = Vec::new();
    expected.extend_from_slice(REPUTATION_BLOCK_PUBLICATION_DOMAIN);
    expected.extend_from_slice(&REPUTATION_BLOCK_PUBLICATION_VERSION.to_be_bytes());
    expected.extend_from_slice(
        &u16::try_from(publication.publisher().0.len())
            .unwrap()
            .to_be_bytes(),
    );
    expected.extend_from_slice(&publication.publisher().0);
    expected.extend_from_slice(&u64::try_from(block_envelope.len()).unwrap().to_be_bytes());
    expected.extend_from_slice(&block_envelope);
    expected.extend_from_slice(
        &u16::try_from(publication.signature().len())
            .unwrap()
            .to_be_bytes(),
    );
    expected.extend_from_slice(publication.signature());

    assert_eq!(publication.encode(), expected);
    assert!(expected.len() <= MAX_REPUTATION_BLOCK_PUBLICATION_LEN);
    assert_eq!(
        ReputationBlockPublicationV1::decode(&expected).unwrap(),
        publication
    );

    let signing_hash =
        reputation_block_publication_signing_hash(publication.publisher(), publication.block())
            .unwrap();
    assert!(Secp256k1Scheme.verify(
        &signing_hash,
        &publication.publisher().0,
        publication.signature()
    ));
}

#[test]
fn framing_rejects_bad_domain_version_lengths_truncation_and_trailing_bytes() {
    let publication = signed_publication();
    let encoded = publication.encode();

    let mut bad_domain = encoded.clone();
    bad_domain[0] ^= 1;
    assert_eq!(
        ReputationBlockPublicationV1::decode(&bad_domain),
        Err(PorReputationBlockPublicationError::InvalidDomain)
    );

    let mut bad_version = encoded.clone();
    let version_offset = REPUTATION_BLOCK_PUBLICATION_DOMAIN.len();
    bad_version[version_offset..version_offset + 2].copy_from_slice(&2_u16.to_be_bytes());
    assert_eq!(
        ReputationBlockPublicationV1::decode(&bad_version),
        Err(PorReputationBlockPublicationError::UnsupportedVersion(2))
    );

    let publisher_length_offset = version_offset + 2;
    let mut bad_publisher_length = encoded.clone();
    bad_publisher_length[publisher_length_offset..publisher_length_offset + 2]
        .copy_from_slice(&32_u16.to_be_bytes());
    assert_eq!(
        ReputationBlockPublicationV1::decode(&bad_publisher_length),
        Err(PorReputationBlockPublicationError::InvalidPublisherKeyLength(32))
    );

    let block_length_offset = publisher_length_offset + 2 + publication.publisher().0.len();
    let mut oversized_block = encoded.clone();
    oversized_block[block_length_offset..block_length_offset + 8].copy_from_slice(
        &u64::try_from(MAX_REPUTATION_BLOCK_WIRE_LEN + 1)
            .unwrap()
            .to_be_bytes(),
    );
    assert_eq!(
        ReputationBlockPublicationV1::decode(&oversized_block),
        Err(PorReputationBlockPublicationError::BlockEnvelopeTooLong {
            actual: MAX_REPUTATION_BLOCK_WIRE_LEN + 1,
            maximum: MAX_REPUTATION_BLOCK_WIRE_LEN,
        })
    );

    assert_eq!(
        ReputationBlockPublicationV1::decode(&encoded[..3]),
        Err(PorReputationBlockPublicationError::UnexpectedEnd)
    );

    let mut trailing = encoded;
    trailing.push(0);
    assert_eq!(
        ReputationBlockPublicationV1::decode(&trailing),
        Err(PorReputationBlockPublicationError::TrailingBytes)
    );

    let too_long = vec![0; MAX_REPUTATION_BLOCK_PUBLICATION_LEN + 1];
    assert_eq!(
        ReputationBlockPublicationV1::decode(&too_long),
        Err(PorReputationBlockPublicationError::PublicationTooLong {
            actual: too_long.len(),
            maximum: MAX_REPUTATION_BLOCK_PUBLICATION_LEN,
        })
    );
}

#[test]
fn accepts_supported_uncompressed_publisher_keys() {
    let publisher = uncompressed_node(7);
    let publication =
        ReputationBlockPublicationV1::sign(publisher.clone(), block(), &private_key(7)).unwrap();

    assert_eq!(publisher.0.len(), 65);
    assert_eq!(publication.publisher(), &publisher);
    assert_eq!(
        ReputationBlockPublicationV1::decode(&publication.encode()).unwrap(),
        publication
    );
}

#[test]
fn constructors_reject_invalid_publishers_and_signatures() {
    assert_eq!(
        ReputationBlockPublicationV1::sign(NodeId(vec![0; 32]), block(), &private_key(7)),
        Err(PorReputationBlockPublicationError::InvalidPublisherKeyLength(32))
    );
    assert_eq!(
        ReputationBlockPublicationV1::sign(node(7), block(), &private_key(8)),
        Err(PorReputationBlockPublicationError::InvalidSignature)
    );
    assert_eq!(
        ReputationBlockPublicationV1::new(node(7), block(), Vec::new()),
        Err(PorReputationBlockPublicationError::MissingSignature)
    );
    assert_eq!(
        ReputationBlockPublicationV1::new(node(7), block(), vec![0; 73]),
        Err(PorReputationBlockPublicationError::SignatureTooLong {
            actual: 73,
            maximum: 72,
        })
    );
}

#[test]
fn block_and_signature_tampering_are_detected() {
    let publication = signed_publication();
    let encoded = publication.encode();

    let block_length_offset =
        REPUTATION_BLOCK_PUBLICATION_DOMAIN.len() + 2 + 2 + publication.publisher().0.len();
    let block_offset = block_length_offset + 8;
    let block_length = usize::try_from(u64::from_be_bytes(
        encoded[block_length_offset..block_offset]
            .try_into()
            .unwrap(),
    ))
    .unwrap();

    let mut corrupted_block = encoded.clone();
    corrupted_block[block_offset + block_length - 1] ^= 1;
    assert_eq!(
        ReputationBlockPublicationV1::decode(&corrupted_block),
        Err(PorReputationBlockPublicationError::Block(
            PorError::ReputationBlockWireChecksumMismatch
        ))
    );

    let mut corrupted_signature = encoded;
    *corrupted_signature.last_mut().unwrap() ^= 1;
    assert_eq!(
        ReputationBlockPublicationV1::decode(&corrupted_signature),
        Err(PorReputationBlockPublicationError::InvalidSignature)
    );

    let mut different_block = publication.block().clone();
    different_block.header.ratings_hash[0] ^= 1;
    assert_eq!(
        ReputationBlockPublicationV1::new(
            publication.publisher().clone(),
            different_block,
            publication.signature().to_vec(),
        ),
        Err(PorReputationBlockPublicationError::InvalidSignature)
    );
}

#[test]
fn broadcast_sends_exact_authenticated_bytes_and_supports_retry() {
    let broadcaster = RecordingBroadcaster::default();
    let publication =
        broadcast_reputation_block(&broadcaster, node(7), block(), &private_key(7)).unwrap();

    assert_eq!(
        broadcaster.envelopes.borrow().as_slice(),
        &[publication.encode()]
    );
    broadcast_reputation_block_publication(&broadcaster, &publication).unwrap();
    assert_eq!(broadcaster.envelopes.borrow().len(), 2);

    let failing = RecordingBroadcaster {
        failure: Some("peer unavailable".to_string()),
        ..RecordingBroadcaster::default()
    };
    assert_eq!(
        broadcast_reputation_block_publication(&failing, &publication),
        Err(PorReputationBlockTransportError::Broadcast(
            "peer unavailable".to_string()
        ))
    );
    assert!(failing.envelopes.borrow().is_empty());
}

#[test]
fn signing_failure_prevents_broadcast_side_effects() {
    let broadcaster = RecordingBroadcaster::default();

    assert_eq!(
        broadcast_reputation_block(&broadcaster, NodeId(vec![0; 32]), block(), &private_key(7),),
        Err(PorReputationBlockTransportError::Publication(
            PorReputationBlockPublicationError::InvalidPublisherKeyLength(32)
        ))
    );
    assert!(broadcaster.envelopes.borrow().is_empty());
}

#[test]
fn inbound_receive_authenticates_without_claiming_transition_admission() {
    let publication = signed_publication();

    let received = receive_reputation_block_envelope(&publication.encode()).unwrap();

    assert_eq!(received.publisher(), publication.publisher());
    assert_eq!(received.block(), publication.block());
    assert_eq!(received.into_block(), block());
}
