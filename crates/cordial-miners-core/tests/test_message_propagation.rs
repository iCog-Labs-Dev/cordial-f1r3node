use cordial_miners_core::network::Message;
use cordial_miners_core::{Block, BlockContent, BlockIdentity, NodeId};
use std::collections::HashSet;

fn make_genesis() -> Block {
    Block {
        identity: BlockIdentity {
            content_hash: [0x01; 32],
            creator: NodeId(vec![1]),
            signature: vec![],
        },
        content: BlockContent { payload: vec![42], predecessors: HashSet::new() },
    }
}

#[test]
fn broadcast_block_serializes_and_deserializes() {
    let block = make_genesis();
    let msg = Message::BroadcastBlock { block: block.clone() };
    let bytes = bincode::serialize(&msg).unwrap();
    let decoded: Message = bincode::deserialize(&bytes).unwrap();
    assert_eq!(msg, decoded);
}

#[test]
fn request_block_serializes_and_deserializes() {
    let id = BlockIdentity {
        content_hash: [0x02; 32],
        creator: NodeId(vec![2]),
        signature: vec![0xab],
    };
    let msg = Message::RequestBlock { id };
    let bytes = bincode::serialize(&msg).unwrap();
    let decoded: Message = bincode::deserialize(&bytes).unwrap();
    assert_eq!(msg, decoded);
}

#[test]
fn block_response_with_block_roundtrips() {
    let block = make_genesis();
    let msg = Message::BlockResponse { block: Some(block) };
    let bytes = bincode::serialize(&msg).unwrap();
    let decoded: Message = bincode::deserialize(&bytes).unwrap();
    assert_eq!(msg, decoded);
}

#[test]
fn block_response_none_roundtrips() {
    let msg = Message::BlockResponse { block: None };
    let bytes = bincode::serialize(&msg).unwrap();
    let decoded: Message = bincode::deserialize(&bytes).unwrap();
    assert_eq!(msg, decoded);
}

#[test]
fn sync_request_roundtrips() {
    let msg = Message::SyncRequest;
    let bytes = bincode::serialize(&msg).unwrap();
    let decoded: Message = bincode::deserialize(&bytes).unwrap();
    assert_eq!(msg, decoded);
}

#[test]
fn sync_response_roundtrips() {
    let ids = vec![
        BlockIdentity { content_hash: [0x01; 32], creator: NodeId(vec![1]), signature: vec![] },
        BlockIdentity { content_hash: [0x02; 32], creator: NodeId(vec![2]), signature: vec![] },
    ];
    let msg = Message::SyncResponse { block_ids: ids };
    let bytes = bincode::serialize(&msg).unwrap();
    let decoded: Message = bincode::deserialize(&bytes).unwrap();
    assert_eq!(msg, decoded);
}

#[test]
fn sync_response_empty_roundtrips() {
    let msg = Message::SyncResponse { block_ids: vec![] };
    let bytes = bincode::serialize(&msg).unwrap();
    let decoded: Message = bincode::deserialize(&bytes).unwrap();
    assert_eq!(msg, decoded);
}
