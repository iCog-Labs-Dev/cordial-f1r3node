use serde::{Serialize, Deserialize};
use crate::block::Block;
use crate::types::BlockIdentity;

/// Messages exchanged between peers over TCP.
///
/// This is the wire protocol for blocklace P2P communication.
/// Every message is length-prefixed (4 bytes big-endian) then bincode-serialized.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum Message {
    // ── Handshake ──

    /// Initial handshake: a peer announces its identity and listening port.
    Hello { node_id: Vec<u8>, listen_port: u16 },

    /// Acknowledge a successful handshake.
    HelloAck { node_id: Vec<u8> },

    // ── Keepalive ──

    /// Heartbeat to keep the connection alive and detect failures.
    Ping,

    /// Response to a Ping.
    Pong,

    // ── Block propagation ──

    /// Broadcast a newly created block to peers.
    BroadcastBlock { block: Block },

    /// Request a specific block by its identity (used when a predecessor is missing).
    RequestBlock { id: BlockIdentity },

    /// Response to a RequestBlock — delivers the requested block, or None if unknown.
    BlockResponse { block: Option<Block> },

    // ── Sync ──

    /// Request the peer's full set of block identities for synchronization.
    SyncRequest,

    /// Response to SyncRequest — the peer's full dom() set.
    SyncResponse { block_ids: Vec<BlockIdentity> },
}
