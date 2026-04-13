use blocklace::network::{Message, Peer};

#[tokio::test]
async fn peer_binds_and_reports_listen_addr() {
    let peer = Peer::bind(vec![1], "127.0.0.1:0").await.unwrap();
    let addr = peer.listen_addr();
    assert_eq!(addr.ip(), std::net::Ipv4Addr::LOCALHOST);
    assert_ne!(addr.port(), 0); // OS assigned a real port
}

#[tokio::test]
async fn peer_reports_its_node_id() {
    let peer = Peer::bind(vec![10, 20, 30], "127.0.0.1:0").await.unwrap();
    assert_eq!(peer.node_id(), &[10, 20, 30]);
}

#[tokio::test]
async fn new_peer_has_zero_connections() {
    let peer = Peer::bind(vec![1], "127.0.0.1:0").await.unwrap();
    assert_eq!(peer.connection_count().await, 0);
}

#[tokio::test]
async fn two_peers_can_connect_via_handshake() {
    let peer_a = Peer::bind(vec![1], "127.0.0.1:0").await.unwrap();
    let peer_b = Peer::bind(vec![2], "127.0.0.1:0").await.unwrap();

    // peer_b connects to peer_a
    peer_b.connect(&peer_a.listen_addr().to_string()).await.unwrap();

    // Give the accept loop a moment to process the inbound connection
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    // peer_b should have registered peer_a's node id
    assert_eq!(peer_b.connection_count().await, 1);
    let peers = peer_b.connected_peers().await;
    assert_eq!(peers.len(), 1);
    assert_eq!(peers[0], vec![1]); // peer_a's node id

    // peer_a should have registered peer_b's node id (from the inbound handler)
    assert_eq!(peer_a.connection_count().await, 1);
    let peers = peer_a.connected_peers().await;
    assert_eq!(peers.len(), 1);
    assert_eq!(peers[0], vec![2]); // peer_b's node id
}

#[tokio::test]
async fn peer_a_receives_hello_from_peer_b() {
    let peer_a = Peer::bind(vec![1], "127.0.0.1:0").await.unwrap();
    let peer_b = Peer::bind(vec![2], "127.0.0.1:0").await.unwrap();

    peer_b.connect(&peer_a.listen_addr().to_string()).await.unwrap();

    // peer_a should receive the Hello message forwarded by handle_inbound
    let (_, msg) = tokio::time::timeout(
        std::time::Duration::from_secs(2),
        peer_a.recv(),
    ).await.unwrap().unwrap();

    match msg {
        Message::Hello { node_id, listen_port } => {
            assert_eq!(node_id, vec![2]);
            assert_eq!(listen_port, peer_b.listen_addr().port());
        }
        other => panic!("expected Hello, got {:?}", other),
    }
}

#[tokio::test]
async fn connect_to_nonexistent_peer_fails() {
    let peer = Peer::bind(vec![1], "127.0.0.1:0").await.unwrap();
    let result = peer.connect("127.0.0.1:1").await;
    assert!(result.is_err());
}

#[tokio::test]
async fn multiple_peers_can_connect_to_one() {
    let server = Peer::bind(vec![0], "127.0.0.1:0").await.unwrap();
    let client_a = Peer::bind(vec![1], "127.0.0.1:0").await.unwrap();
    let client_b = Peer::bind(vec![2], "127.0.0.1:0").await.unwrap();

    let addr = server.listen_addr().to_string();
    client_a.connect(&addr).await.unwrap();
    client_b.connect(&addr).await.unwrap();

    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    assert_eq!(server.connection_count().await, 2);
    assert_eq!(client_a.connection_count().await, 1);
    assert_eq!(client_b.connection_count().await, 1);
}
