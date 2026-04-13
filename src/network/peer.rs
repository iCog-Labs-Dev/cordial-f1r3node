use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, Mutex};

use crate::network::message::Message;

/// A connected remote peer.
#[derive(Debug)]
struct Connection {
    /// The remote peer's self-reported node id.
    node_id: Vec<u8>,
}

/// A local peer that can listen for and initiate TCP connections.
///
/// Each Peer has:
/// - A node identity (public key bytes)
/// - A TCP listener for inbound connections
/// - A set of active outbound/inbound connections
/// - A channel for delivering received messages to the application layer
pub struct Peer {
    /// This peer's node identity.
    node_id: Vec<u8>,
    /// Address this peer listens on.
    listen_addr: SocketAddr,
    /// Active connections keyed by remote address.
    connections: Arc<Mutex<HashMap<SocketAddr, Connection>>>,
    /// Channel for incoming messages (sender side held by connection handlers).
    incoming_tx: mpsc::Sender<(SocketAddr, Message)>,
    /// Channel for incoming messages (receiver side for the application).
    incoming_rx: Arc<Mutex<mpsc::Receiver<(SocketAddr, Message)>>>,
}

impl Peer {
    /// Create a new peer bound to the given address.
    pub async fn bind(node_id: Vec<u8>, addr: &str) -> std::io::Result<Self> {
        let listener = TcpListener::bind(addr).await?;
        let listen_addr = listener.local_addr()?;
        let (incoming_tx, incoming_rx) = mpsc::channel(256);
        let connections: Arc<Mutex<HashMap<SocketAddr, Connection>>> =
            Arc::new(Mutex::new(HashMap::new()));

        // Spawn the accept loop
        let conns = connections.clone();
        let tx = incoming_tx.clone();
        let my_id = node_id.clone();
        tokio::spawn(async move {
            loop {
                match listener.accept().await {
                    Ok((stream, addr)) => {
                        let conns = conns.clone();
                        let tx = tx.clone();
                        let my_id = my_id.clone();
                        tokio::spawn(async move {
                            if let Err(e) = handle_inbound(stream, addr, my_id, conns, tx).await {
                                eprintln!("inbound connection from {} failed: {}", addr, e);
                            }
                        });
                    }
                    Err(e) => {
                        eprintln!("accept error: {}", e);
                    }
                }
            }
        });

        Ok(Self {
            node_id,
            listen_addr,
            connections,
            incoming_tx,
            incoming_rx: Arc::new(Mutex::new(incoming_rx)),
        })
    }

    /// Returns the address this peer is listening on.
    pub fn listen_addr(&self) -> SocketAddr {
        self.listen_addr
    }

    /// Returns this peer's node identity.
    pub fn node_id(&self) -> &[u8] {
        &self.node_id
    }

    /// Connect to a remote peer, perform the handshake, and start
    /// a background reader for incoming messages.
    pub async fn connect(&self, remote_addr: &str) -> std::io::Result<()> {
        let mut stream = TcpStream::connect(remote_addr).await?;
        let remote = stream.peer_addr()?;

        // Send Hello
        let hello = Message::Hello {
            node_id: self.node_id.clone(),
            listen_port: self.listen_addr.port(),
        };
        send_message(&mut stream, &hello).await?;

        // Wait for HelloAck
        let response = recv_message(&mut stream).await?;
        match &response {
            Message::HelloAck { node_id } => {
                let conn = Connection {
                    node_id: node_id.clone(),
                };
                self.connections.lock().await.insert(remote, conn);
            }
            _ => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "expected HelloAck",
                ));
            }
        }

        // Spawn reader loop for this connection
        let tx = self.incoming_tx.clone();
        tokio::spawn(async move {
            loop {
                match recv_message(&mut stream).await {
                    Ok(msg) => {
                        if tx.send((remote, msg)).await.is_err() {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
        });

        Ok(())
    }

    /// Send a message to a connected peer by address.
    pub async fn send(&self, remote_addr: SocketAddr, msg: &Message) -> std::io::Result<()> {
        let mut stream = TcpStream::connect(remote_addr).await?;
        send_message(&mut stream, msg).await
    }

    /// Receive the next incoming message (blocks until one arrives).
    pub async fn recv(&self) -> Option<(SocketAddr, Message)> {
        self.incoming_rx.lock().await.recv().await
    }

    /// Returns the number of active connections.
    pub async fn connection_count(&self) -> usize {
        self.connections.lock().await.len()
    }

    /// Returns the node ids of all connected peers.
    pub async fn connected_peers(&self) -> Vec<Vec<u8>> {
        self.connections
            .lock()
            .await
            .values()
            .map(|c| c.node_id.clone())
            .collect()
    }
}

/// Handle an inbound TCP connection: read Hello, send HelloAck, then loop reading messages.
async fn handle_inbound(
    mut stream: TcpStream,
    addr: SocketAddr,
    my_node_id: Vec<u8>,
    connections: Arc<Mutex<HashMap<SocketAddr, Connection>>>,
    tx: mpsc::Sender<(SocketAddr, Message)>,
) -> std::io::Result<()> {
    // Expect Hello
    let hello = recv_message(&mut stream).await?;
    let remote_node_id = match &hello {
        Message::Hello { node_id, .. } => node_id.clone(),
        _ => {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "expected Hello",
            ));
        }
    };

    // Send HelloAck
    let ack = Message::HelloAck { node_id: my_node_id };
    send_message(&mut stream, &ack).await?;

    // Register the connection
    connections.lock().await.insert(addr, Connection {
        node_id: remote_node_id,
    });

    // Forward the Hello to the application as well
    let _ = tx.send((addr, hello)).await;

    // Read loop
    loop {
        match recv_message(&mut stream).await {
            Ok(msg) => {
                if tx.send((addr, msg)).await.is_err() {
                    break;
                }
            }
            Err(_) => {
                connections.lock().await.remove(&addr);
                break;
            }
        }
    }
    Ok(())
}

/// Send a length-prefixed bincode-encoded message over a TCP stream.
pub async fn send_message(stream: &mut TcpStream, msg: &Message) -> std::io::Result<()> {
    let payload = bincode::serialize(msg)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    let len = (payload.len() as u32).to_be_bytes();
    stream.write_all(&len).await?;
    stream.write_all(&payload).await?;
    stream.flush().await?;
    Ok(())
}

/// Receive a length-prefixed bincode-encoded message from a TCP stream.
pub async fn recv_message(stream: &mut TcpStream) -> std::io::Result<Message> {
    let mut len_buf = [0u8; 4];
    stream.read_exact(&mut len_buf).await?;
    let len = u32::from_be_bytes(len_buf) as usize;

    let mut payload = vec![0u8; len];
    stream.read_exact(&mut payload).await?;

    bincode::deserialize(&payload)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
}
