use cordial_f1r3node_adapter::por::{
    MAX_REPUTATION_BLOCK_PUBLICATION_LEN, PorReputationBlockChannelError,
    PorReputationBlockPublicationError, PorReputationBlockTransportError,
    ReputationBlockPublicationV1, ReputationBlockReceiveOutcome,
    bounded_reputation_block_envelope_channel, broadcast_reputation_block,
};
use cordial_miners_core::NodeId;
use cordial_por::{
    REPUTATION_BLOCK_VERSION, ReputationBlock, ReputationBlockHeader, ReputationEntry,
    ReputationList, reputation_list_commitment,
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

fn publication() -> ReputationBlockPublicationV1 {
    ReputationBlockPublicationV1::sign(node(7), block(), &private_key(7)).unwrap()
}

#[tokio::test]
async fn channel_delivers_an_authenticated_publication() {
    let (broadcaster, mut receiver) = bounded_reputation_block_envelope_channel(1).unwrap();

    let sent = broadcast_reputation_block(&broadcaster, node(7), block(), &private_key(7)).unwrap();

    assert_eq!(
        receiver.receive_next().await,
        Ok(ReputationBlockReceiveOutcome::Received(Box::new(sent)))
    );
}

#[tokio::test]
async fn bounded_channel_reports_backpressure_without_replacing_the_queued_block() {
    let (broadcaster, mut receiver) = bounded_reputation_block_envelope_channel(1).unwrap();
    let first = publication();
    let second = publication();

    broadcaster.try_broadcast(&first.encode()).unwrap();
    assert_eq!(
        broadcaster.try_broadcast(&second.encode()),
        Err(PorReputationBlockChannelError::Full)
    );
    assert_eq!(
        receiver.receive_next().await,
        Ok(ReputationBlockReceiveOutcome::Received(Box::new(first)))
    );
}

#[tokio::test]
async fn transport_maps_channel_backpressure_to_a_broadcast_failure() {
    let (broadcaster, mut receiver) = bounded_reputation_block_envelope_channel(1).unwrap();
    let first = publication();
    broadcaster.try_broadcast(&first.encode()).unwrap();

    assert_eq!(
        broadcast_reputation_block(&broadcaster, node(7), block(), &private_key(7)),
        Err(PorReputationBlockTransportError::Broadcast(
            PorReputationBlockChannelError::Full.to_string()
        ))
    );
    assert_eq!(
        receiver.receive_next().await,
        Ok(ReputationBlockReceiveOutcome::Received(Box::new(first)))
    );
}

#[tokio::test]
async fn malformed_channel_item_is_rejected_at_the_authenticated_boundary() {
    let (broadcaster, mut receiver) = bounded_reputation_block_envelope_channel(1).unwrap();
    broadcaster.try_broadcast(&[1, 2, 3]).unwrap();

    assert_eq!(
        receiver.receive_next().await,
        Err(PorReputationBlockTransportError::Publication(
            PorReputationBlockPublicationError::UnexpectedEnd
        ))
    );
}

#[tokio::test]
async fn receiver_reports_closed_after_every_sender_is_dropped() {
    let (broadcaster, mut receiver) = bounded_reputation_block_envelope_channel(1).unwrap();
    drop(broadcaster);

    assert_eq!(
        receiver.receive_next().await,
        Ok(ReputationBlockReceiveOutcome::Closed)
    );
}

#[test]
fn sender_rejects_oversized_envelopes_and_closed_receivers() {
    let (broadcaster, receiver) = bounded_reputation_block_envelope_channel(1).unwrap();
    let oversized = vec![0; MAX_REPUTATION_BLOCK_PUBLICATION_LEN + 1];

    assert_eq!(
        broadcaster.try_broadcast(&oversized),
        Err(PorReputationBlockChannelError::EnvelopeTooLong {
            actual: oversized.len(),
            maximum: MAX_REPUTATION_BLOCK_PUBLICATION_LEN,
        })
    );

    drop(receiver);
    assert_eq!(
        broadcaster.try_broadcast(&[1, 2, 3]),
        Err(PorReputationBlockChannelError::Closed)
    );
}

#[test]
fn zero_capacity_is_rejected_without_panicking() {
    assert!(matches!(
        bounded_reputation_block_envelope_channel(0),
        Err(PorReputationBlockChannelError::ZeroCapacity)
    ));
}
