use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::Mutex;

use crate::block::Block;
use crate::blocklace::Blocklace;
use crate::network::message::Message;
use crate::network::peer::Peer;
use crate::types::BlockIdentity;
use crate::crypto::{CryptoVerifier, Secp256k1Scheme};

/// A blocklace network node — owns a Peer (networking) and a Blocklace (state).
///
/// Handles:
/// - Creating and broadcasting new blocks
/// - Receiving blocks from peers and inserting them (with predecessor fetching)
/// - Synchronizing state with peers on connect
pub struct Node <V: CryptoVerifier> {
    pub peer: Peer,
    pub blocklace: Arc<Mutex<Blocklace>>,
    pub verifier: V,
}

impl <V: CryptoVerifier> Node<V> {
    /// Create a new node bound to the given address.
    pub async fn bind(node_id: Vec<u8>, addr: &str, verifier: V) -> std::io::Result<Self> {
        let peer = Peer::bind(node_id, addr).await?;
        Ok(Self {
            peer,
            blocklace: Arc::new(Mutex::new(Blocklace::new())),
            verifier,
        })
    }

    /// Connect to a remote peer.
    pub async fn connect(&self, remote_addr: &str) -> std::io::Result<()> {
        self.peer.connect(remote_addr).await
    }

    /// Insert a block locally and broadcast it to all connected peers.
    pub async fn create_block(&self, block: Block) -> Result<(), String> where V::Error: std::fmt::Debug {
        
        // Insert into local blocklace
        self.blocklace.lock().await.insert(block.clone(), &self.verifier)?;

        // Broadcast to all connected peers
        let peers = self.peer.connected_peer_addrs().await;
        for addr in peers {
            let msg = Message::BroadcastBlock {
                block: block.clone(),
            };
            let _ = self.peer.send(addr, &msg).await;
        }
        Ok(())
    }

    /// Handle an incoming message from a peer.
    /// Returns an optional response message to send back.
    pub async fn handle_message(&self, from: SocketAddr, msg: Message) -> Option<Message> {
        match msg {
            Message::Ping => Some(Message::Pong),

            Message::BroadcastBlock { block } => {
                self.receive_block(block, from).await;
                None
            }

            Message::RequestBlock { id } => {
                let block = self.blocklace.lock().await.get(&id);
                Some(Message::BlockResponse { block })
            }

            Message::SyncRequest => {
                let bl = self.blocklace.lock().await;
                let block_ids: Vec<BlockIdentity> = bl.dom().into_iter().cloned().collect();
                Some(Message::SyncResponse { block_ids })
            }

            Message::SyncResponse { block_ids } => {
                // Request any blocks we don't have
                let bl = self.blocklace.lock().await;
                let our_dom = bl.dom();
                let missing: Vec<BlockIdentity> = block_ids
                    .into_iter()
                    .filter(|id| !our_dom.contains(id))
                    .collect();
                drop(bl);

                for id in missing {
                    let msg = Message::RequestBlock { id };
                    let _ = self.peer.send(from, &msg).await;
                }
                None
            }

            Message::BlockResponse { block } => {
                if let Some(block) = block {
                    self.receive_block(block, from).await;
                }
                None
            }

            // Handshake messages are handled by the Peer layer
            Message::Hello { .. } | Message::HelloAck { .. } | Message::Pong => None,
        }
    }

    /// Try to insert a received block. If predecessors are missing, request them.
    async fn receive_block(&self, block: Block, from: SocketAddr) {
        let mut bl = self.blocklace.lock().await;

        // Check which predecessors we're missing
        let missing_preds: Vec<BlockIdentity> = block
            .content
            .predecessors
            .iter()
            .filter(|pred_id| bl.content(pred_id).is_none())
            .cloned()
            .collect();
        let verifier = Secp256k1Scheme;
        if missing_preds.is_empty() {
            // All predecessors present — insert directly
            let _ = bl.insert(block, &self.verifier);
        } else {
            // Drop the lock before sending network requests
            drop(bl);

            // Request each missing predecessor
            for pred_id in missing_preds {
                let msg = Message::RequestBlock { id: pred_id };
                let _ = self.peer.send(from, &msg).await;
            }
        }
    }

    /// Run the message processing loop.
    /// This drives the node by receiving messages and dispatching responses.
    pub async fn run(&self) {
        loop {
            let Some((from, msg)) = self.peer.recv().await else {
                break;
            };

            if let Some(response) = self.handle_message(from, msg).await {
                let _ = self.peer.send(from, &response).await;
            }
        }
    }

    /// Initiate a sync with a remote peer — send SyncRequest to discover missing blocks.
    pub async fn sync_with(&self, remote_addr: SocketAddr) -> std::io::Result<()> {
        self.peer.send(remote_addr, &Message::SyncRequest).await
    }
}
