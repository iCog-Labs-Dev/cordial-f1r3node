use cordial_miners_core::network::Message;

#[test]
fn hello_serializes_and_deserializes() {
    let msg = Message::Hello { node_id: vec![1, 2, 3], listen_port: 8080 };
    let bytes = bincode::serialize(&msg).unwrap();
    let decoded: Message = bincode::deserialize(&bytes).unwrap();
    assert_eq!(msg, decoded);
}

#[test]
fn hello_ack_serializes_and_deserializes() {
    let msg = Message::HelloAck { node_id: vec![4, 5, 6] };
    let bytes = bincode::serialize(&msg).unwrap();
    let decoded: Message = bincode::deserialize(&bytes).unwrap();
    assert_eq!(msg, decoded);
}

#[test]
fn ping_serializes_and_deserializes() {
    let msg = Message::Ping;
    let bytes = bincode::serialize(&msg).unwrap();
    let decoded: Message = bincode::deserialize(&bytes).unwrap();
    assert_eq!(msg, decoded);
}

#[test]
fn pong_serializes_and_deserializes() {
    let msg = Message::Pong;
    let bytes = bincode::serialize(&msg).unwrap();
    let decoded: Message = bincode::deserialize(&bytes).unwrap();
    assert_eq!(msg, decoded);
}

#[test]
fn different_messages_produce_different_bytes() {
    let hello = bincode::serialize(&Message::Hello { node_id: vec![1], listen_port: 9000 }).unwrap();
    let ack = bincode::serialize(&Message::HelloAck { node_id: vec![1] }).unwrap();
    let ping = bincode::serialize(&Message::Ping).unwrap();
    let pong = bincode::serialize(&Message::Pong).unwrap();
    assert_ne!(hello, ack);
    assert_ne!(ping, pong);
    assert_ne!(hello, ping);
}

#[test]
fn hello_with_empty_node_id_roundtrips() {
    let msg = Message::Hello { node_id: vec![], listen_port: 0 };
    let bytes = bincode::serialize(&msg).unwrap();
    let decoded: Message = bincode::deserialize(&bytes).unwrap();
    assert_eq!(msg, decoded);
}
