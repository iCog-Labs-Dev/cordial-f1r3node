use cordial_f1r3node_adapter::ordered_output::OrderedFinalizedOutput;
use cordial_f1r3node_adapter::ordered_output_file::{
    OrderedOutputFileError, write_latest_ordered_output_file,
};
use cordial_f1r3node_adapter::shared_ordered_output::SharedOrderedOutput;
use cordial_miners_core::types::{BlockIdentity, NodeId};

#[test]
fn writes_latest_ordered_output_as_json() {
    let output = OrderedFinalizedOutput::new(vec![block(1), block(2)], Some(block(2)), 3, 4, 9)
        .with_timestamp(123);
    let reader = SharedOrderedOutput::from_output(output.clone());
    let file = tempfile::NamedTempFile::new().expect("temp file should be created");

    write_latest_ordered_output_file(file.path(), &reader, false)
        .expect("ordered output should be written");

    let decoded: OrderedFinalizedOutput =
        serde_json::from_slice(&std::fs::read(file.path()).expect("file should be readable"))
            .expect("ordered output json should roundtrip");

    assert_eq!(decoded, output);
}

#[test]
fn rejects_empty_reader_by_default() {
    let reader = SharedOrderedOutput::new();
    let file = tempfile::NamedTempFile::new().expect("temp file should be created");

    let err = write_latest_ordered_output_file(file.path(), &reader, false)
        .expect_err("empty reader should not be written");

    assert!(matches!(err, OrderedOutputFileError::EmptyOutput));
}

#[test]
fn rejects_empty_output_by_default() {
    let reader = SharedOrderedOutput::from_output(OrderedFinalizedOutput::default());
    let file = tempfile::NamedTempFile::new().expect("temp file should be created");

    let err = write_latest_ordered_output_file(file.path(), &reader, false)
        .expect_err("empty output should not be written by default");

    assert!(matches!(err, OrderedOutputFileError::EmptyOutput));
}

#[test]
fn writes_empty_output_when_allowed() {
    let output = OrderedFinalizedOutput::default();
    let reader = SharedOrderedOutput::from_output(output.clone());
    let file = tempfile::NamedTempFile::new().expect("temp file should be created");

    write_latest_ordered_output_file(file.path(), &reader, true)
        .expect("empty output should be written when explicitly allowed");

    let decoded: OrderedFinalizedOutput =
        serde_json::from_slice(&std::fs::read(file.path()).expect("file should be readable"))
            .expect("ordered output json should roundtrip");

    assert_eq!(decoded, output);
}

fn block(tag: u8) -> BlockIdentity {
    BlockIdentity {
        content_hash: [tag; 32],
        creator: NodeId(vec![tag]),
        signature: vec![tag; 64],
    }
}
