use std::time::{SystemTime, UNIX_EPOCH};

use cordial_f1r3node_adapter::ordered_output::OrderedFinalizedOutput;
use cordial_f1r3node_adapter::shared_ordered_output::{
    ReadOrderedOutput, SharedOrderedOutput, SharedOrderedOutputError,
};
use cordial_miners_core::types::{BlockIdentity, NodeId};

#[test]
fn shared_ordered_output_starts_empty_and_stale() {
    let reader = SharedOrderedOutput::new();

    assert!(reader.latest().is_none());
    assert_eq!(reader.anchor_hash(), None);
    assert!(reader.is_stale(1_000));
}

#[test]
fn shared_ordered_output_reads_latest_anchor_hash() {
    let anchor = block(9);
    let output = OrderedFinalizedOutput::new(
        vec![block(1), anchor.clone()],
        Some(anchor.clone()),
        3,
        4,
        9,
    )
    .with_timestamp(now_ns());
    let reader = SharedOrderedOutput::from_output(output);

    assert_eq!(reader.latest().expect("output should exist").len(), 2);
    assert_eq!(reader.anchor_hash(), Some(anchor.content_hash.to_vec()));
    assert!(!reader.is_stale(u128::MAX));
}

#[test]
fn shared_ordered_output_accepts_prefix_preserving_updates() {
    let mut reader = SharedOrderedOutput::new();
    let first = output(vec![block(1), block(2)]);
    let second = output(vec![block(1), block(2), block(3)]);

    reader
        .update(first)
        .expect("first output should always publish");
    reader
        .update(second.clone())
        .expect("appended output should preserve prefix");

    assert_eq!(reader.latest(), Some(&second));
}

#[test]
fn shared_ordered_output_accepts_identical_updates() {
    let mut reader = SharedOrderedOutput::from_output(output(vec![block(1), block(2)]));
    let same = output(vec![block(1), block(2)]);

    reader
        .update(same.clone())
        .expect("identical output should preserve prefix");

    assert_eq!(reader.latest(), Some(&same));
}

#[test]
fn shared_ordered_output_rejects_reordered_updates() {
    let mut reader = SharedOrderedOutput::from_output(output(vec![block(1), block(2)]));
    let reordered = output(vec![block(2), block(1), block(3)]);

    let err = reader
        .update(reordered)
        .expect_err("reordered output must violate prefix preservation");

    assert_eq!(err, SharedOrderedOutputError::PrefixViolation);
    assert_eq!(
        reader
            .latest()
            .expect("previous output should remain after rejection")
            .block_hashes(),
        vec![vec![1; 32], vec![2; 32]]
    );
}

#[test]
fn shared_ordered_output_rejects_truncated_updates() {
    let mut reader = SharedOrderedOutput::from_output(output(vec![block(1), block(2), block(3)]));
    let truncated = output(vec![block(1), block(2)]);

    let err = reader
        .update(truncated)
        .expect_err("truncated output must violate prefix preservation");

    assert_eq!(err, SharedOrderedOutputError::PrefixViolation);
}

#[test]
fn shared_ordered_output_can_be_cleared() {
    let mut reader = SharedOrderedOutput::from_output(output(vec![block(1)]));

    reader.clear();

    assert!(reader.latest().is_none());
    assert!(reader.is_stale(0));
}

fn block(tag: u8) -> BlockIdentity {
    BlockIdentity {
        content_hash: [tag; 32],
        creator: NodeId(vec![tag]),
        signature: vec![tag; 64],
    }
}

fn output(blocks: Vec<BlockIdentity>) -> OrderedFinalizedOutput {
    OrderedFinalizedOutput::new(blocks, None, 3, 4, 9).with_timestamp(now_ns())
}

fn now_ns() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
}
