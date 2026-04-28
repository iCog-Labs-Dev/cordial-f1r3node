use cordial_miners_core::network::{Message, Node};
use cordial_miners_core::{Block, BlockContent, BlockIdentity, NodeId};
use std::collections::HashSet;
use cordial_miners_core::crypto::{CryptoVerifier};

struct MockVerifier;

impl CryptoVerifier for MockVerifier {
    type Error = String;
    fn verify_block(
        &self, 
        _content: &BlockContent, 
        _sig: &[u8], 
        _creator: &NodeId
    ) -> Result<(), Self::Error> {
        Ok(()) // Always allow in tests
    }
}

fn make_genesis(tag: u8) -> Block {
    Block {
        identity: BlockIdentity {
            content_hash: [tag; 32],
            creator: NodeId(vec![tag]),
            signature: vec![],
        },
        content: BlockContent {
            payload: vec![tag],
            predecessors: HashSet::new(),
        },
    }
}

fn make_child(tag: u8, parent: &Block) -> Block {
    Block {
        identity: BlockIdentity {
            content_hash: [tag; 32],
            creator: NodeId(vec![tag]),
            signature: vec![],
        },
        content: BlockContent {
            payload: vec![tag],
            predecessors: [parent.identity.clone()].iter().cloned().collect(),
        },
    }
}

#[tokio::test]
async fn node_starts_with_empty_blocklace() {
    let node = Node::bind(vec![1], "127.0.0.1:0", MockVerifier).await.unwrap();
    let bl = node.blocklace.lock().await;
    assert_eq!(bl.dom().len(), 0);
}

#[tokio::test]
async fn create_block_inserts_into_local_blocklace() {
    let node = Node::bind(vec![1], "127.0.0.1:0", MockVerifier).await.unwrap();
    let genesis = make_genesis(1);

    node.create_block(genesis.clone()).await.unwrap();

    let bl = node.blocklace.lock().await;
    assert_eq!(bl.dom().len(), 1);
    assert!(bl.get(&genesis.identity).is_some());
}

#[tokio::test]
async fn create_block_with_missing_predecessor_fails() {
    let node = Node::bind(vec![1], "127.0.0.1:0", MockVerifier).await.unwrap();
    let genesis = make_genesis(1);
    // Don't insert genesis first — child should fail
    let child = make_child(2, &genesis);

    let result = node.create_block(child).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn create_block_chain_succeeds() {
    let node = Node::bind(vec![1], "127.0.0.1:0", MockVerifier).await.unwrap();
    let genesis = make_genesis(1);
    let child = make_child(2, &genesis);

    node.create_block(genesis.clone()).await.unwrap();
    node.create_block(child.clone()).await.unwrap();

    let bl = node.blocklace.lock().await;
    assert_eq!(bl.dom().len(), 2);
    assert!(bl.is_closed());
}

#[tokio::test]
async fn handle_ping_returns_pong() {
    let node = Node::bind(vec![1], "127.0.0.1:0", MockVerifier).await.unwrap();
    let fake_addr = "127.0.0.1:9999".parse().unwrap();

    let response = node.handle_message(fake_addr, Message::Ping).await;
    assert_eq!(response, Some(Message::Pong));
}

#[tokio::test]
async fn handle_request_block_returns_known_block() {
    let node = Node::bind(vec![1], "127.0.0.1:0", MockVerifier).await.unwrap();
    let genesis = make_genesis(1);
    node.create_block(genesis.clone()).await.unwrap();

    let fake_addr = "127.0.0.1:9999".parse().unwrap();
    let response = node
        .handle_message(
            fake_addr,
            Message::RequestBlock {
                id: genesis.identity.clone(),
            },
        )
        .await;

    match response {
        Some(Message::BlockResponse { block: Some(b) }) => {
            assert_eq!(b.identity, genesis.identity);
        }
        other => panic!("expected BlockResponse with block, got {other:?}"),
    }
}

#[tokio::test]
async fn handle_request_block_returns_none_for_unknown() {
    let node = Node::bind(vec![1], "127.0.0.1:0", MockVerifier).await.unwrap();
    let unknown_id = BlockIdentity {
        content_hash: [0xff; 32],
        creator: NodeId(vec![99]),
        signature: vec![],
    };
    let fake_addr = "127.0.0.1:9999".parse().unwrap();

    let response = node
        .handle_message(fake_addr, Message::RequestBlock { id: unknown_id })
        .await;

    match response {
        Some(Message::BlockResponse { block: None }) => {}
        other => panic!("expected BlockResponse with None, got {other:?}"),
    }
}

#[tokio::test]
async fn handle_sync_request_returns_dom() {
    let node = Node::bind(vec![1], "127.0.0.1:0", MockVerifier).await.unwrap();
    let g1 = make_genesis(1);
    let g2 = make_genesis(2);
    node.create_block(g1.clone()).await.unwrap();
    node.create_block(g2.clone()).await.unwrap();

    let fake_addr = "127.0.0.1:9999".parse().unwrap();
    let response = node.handle_message(fake_addr, Message::SyncRequest).await;

    match response {
        Some(Message::SyncResponse { block_ids }) => {
            assert_eq!(block_ids.len(), 2);
            assert!(block_ids.contains(&g1.identity));
            assert!(block_ids.contains(&g2.identity));
        }
        other => panic!("expected SyncResponse, got {other:?}"),
    }
}

#[tokio::test]
async fn handle_broadcast_block_inserts_genesis() {
    let node = Node::bind(vec![1], "127.0.0.1:0", MockVerifier).await.unwrap();
    let genesis = make_genesis(1);
    let fake_addr = "127.0.0.1:9999".parse().unwrap();

    let response = node
        .handle_message(
            fake_addr,
            Message::BroadcastBlock {
                block: genesis.clone(),
            },
        )
        .await;

    // BroadcastBlock returns no response
    assert!(response.is_none());

    // But the block should be in the blocklace
    let bl = node.blocklace.lock().await;
    assert!(bl.get(&genesis.identity).is_some());
}

#[tokio::test]
async fn handle_block_response_inserts_block() {
    let node = Node::bind(vec![1], "127.0.0.1:0", MockVerifier).await.unwrap();
    let genesis = make_genesis(1);
    let fake_addr = "127.0.0.1:9999".parse().unwrap();

    node.handle_message(
        fake_addr,
        Message::BlockResponse {
            block: Some(genesis.clone()),
        },
    )
    .await;

    let bl = node.blocklace.lock().await;
    assert!(bl.get(&genesis.identity).is_some());
}

#[tokio::test]
async fn handle_block_response_none_is_noop() {
    let node = Node::bind(vec![1], "127.0.0.1:0", MockVerifier).await.unwrap();
    let fake_addr = "127.0.0.1:9999".parse().unwrap();

    let response = node
        .handle_message(fake_addr, Message::BlockResponse { block: None })
        .await;

    assert!(response.is_none());
    let bl = node.blocklace.lock().await;
    assert_eq!(bl.dom().len(), 0);
}

#[tokio::test]
async fn handle_hello_is_noop() {
    let node = Node::bind(vec![1], "127.0.0.1:0", MockVerifier).await.unwrap();
    let fake_addr = "127.0.0.1:9999".parse().unwrap();

    let response = node
        .handle_message(
            fake_addr,
            Message::Hello {
                node_id: vec![2],
                listen_port: 8080,
            },
        )
        .await;
    assert!(response.is_none());
}

#[tokio::test]
async fn sync_request_on_empty_returns_empty() {
    let node = Node::bind(vec![1], "127.0.0.1:0", MockVerifier).await.unwrap();
    let fake_addr = "127.0.0.1:9999".parse().unwrap();

    let response = node.handle_message(fake_addr, Message::SyncRequest).await;

    match response {
        Some(Message::SyncResponse { block_ids }) => {
            assert!(block_ids.is_empty());
        }
        other => panic!("expected empty SyncResponse, got {other:?}"),
    }
}
