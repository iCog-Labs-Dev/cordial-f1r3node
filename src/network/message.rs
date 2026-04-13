use serde::{Serialize, Deserialize};

/// Messages exchanged between peers over TCP.
///
/// This is the wire protocol for blocklace P2P communication.
/// Every message is length-prefixed (4 bytes big-endian) then bincode-serialized.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum Message {
    /// Initial handshake: a peer announces its identity and listening port.
    Hello { node_id: Vec<u8>, listen_port: u16 },

    /// Acknowledge a successful handshake.
    HelloAck { node_id: Vec<u8> },

    /// Heartbeat to keep the connection alive and detect failures.
    Ping,

    /// Response to a Ping.
    Pong,
}
