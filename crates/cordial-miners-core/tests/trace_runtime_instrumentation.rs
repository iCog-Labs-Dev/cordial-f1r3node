#![cfg(feature = "trace")]

use std::collections::HashSet;

use cordial_miners_core::NodeId;
use cordial_miners_core::crypto::CryptoVerifier;
use cordial_miners_core::network::Node;
use cordial_miners_core::simulation::adversary::{AdversarialNetwork, BlockFactory};
use cordial_miners_core::trace::TraceEvent;
use cordial_miners_core::{Block, BlockContent};

struct MockVerifier;

impl CryptoVerifier for MockVerifier {
    type Error = String;

    fn verify_block(
        &self,
        _content: &BlockContent,
        _signature: &[u8],
        _creator: &NodeId,
    ) -> Result<(), Self::Error> {
        Ok(())
    }
}

fn node(id: u8) -> NodeId {
    NodeId(vec![id])
}

/// Exercise the real scheduler, transport, validation, buffer, resolution and
/// wave-task paths.  Synthetic events belong only in the schema round-trip
/// test; this test proves these variants are emitted at their runtime sites.
#[tokio::test]
async fn runtime_emits_dissemination_and_scheduler_events() {
    let path = std::env::temp_dir().join(format!(
        "cordial-runtime-trace-{}-{}.json",
        std::process::id(),
        std::thread::current().name().unwrap_or("test")
    ));
    let _ = std::fs::remove_file(&path);

    // This integration-test binary contains one test, so its process-local
    // trace-sink environment cannot race another test thread.
    unsafe { std::env::set_var("CORDIAL_TRACE_FILE", &path) };

    // Exercise the production node's local creation boundary. Binding to port
    // zero asks the OS for an unused loopback port and does not contact a peer.
    let local = Node::bind(vec![9], "127.0.0.1:0", MockVerifier)
        .await
        .expect("bind trace test node");
    let mut local_factory = BlockFactory::new();
    let local_block: Block = local_factory.block(&node(9), HashSet::new());
    local
        .create_block(local_block)
        .await
        .expect("create local block");

    let recipient = node(1);
    let mut network = AdversarialNetwork::equal_stake(2);
    let mut factory = BlockFactory::new();
    let parent = factory.block(&node(1), HashSet::new());
    let child = factory.block(&node(2), HashSet::from([parent.identity.clone()]));

    // Force the actual missing-parent lifecycle: child buffers, parent arrives,
    // then retry emits one resolution and inserts the child.
    network.send_to(&child, std::slice::from_ref(&recipient));
    network.deliver_ready();
    network.send_to(&parent, std::slice::from_ref(&recipient));
    network.deliver_ready();
    network.retry_all_buffers();
    network.advance(1);

    let observer = network.node(&recipient).expect("recipient exists");
    let _ = observer.latest_weighted_final_leader(3, |_| Some(node(1)));

    unsafe { std::env::remove_var("CORDIAL_TRACE_FILE") };
    let content = std::fs::read_to_string(&path).expect("runtime trace was written");
    let _ = std::fs::remove_file(&path);
    let events: Vec<TraceEvent> = content
        .lines()
        .map(|line| serde_json::from_str(line).expect("runtime event is valid JSON"))
        .collect();

    assert!(
        events
            .iter()
            .any(|event| matches!(event, TraceEvent::CreateBlock(_)))
    );
    assert!(
        events
            .iter()
            .any(|event| matches!(event, TraceEvent::SendPackage(_)))
    );
    assert!(
        events
            .iter()
            .any(|event| matches!(event, TraceEvent::DeliverPackage(_)))
    );
    assert!(
        events
            .iter()
            .any(|event| matches!(event, TraceEvent::ValidateBlock(_)))
    );
    assert!(
        events
            .iter()
            .any(|event| matches!(event, TraceEvent::BufferBlock(_)))
    );
    assert!(
        events
            .iter()
            .any(|event| matches!(event, TraceEvent::ResolveMissingParent(_)))
    );
    assert!(
        events
            .iter()
            .any(|event| matches!(event, TraceEvent::InsertBlock(_)))
    );
    assert!(
        events
            .iter()
            .any(|event| matches!(event, TraceEvent::SchedulerTick(_)))
    );
    assert!(
        events
            .iter()
            .any(|event| matches!(event, TraceEvent::RunWaveTask(_)))
    );
}
