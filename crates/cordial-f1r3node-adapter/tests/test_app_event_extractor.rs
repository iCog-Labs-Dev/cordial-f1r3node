use std::collections::BTreeMap;

use cordial_app_runtime::AppId;
use cordial_f1r3node_adapter::app_event_extractor::{
    AppEventExtractionInput, ExtractableAppDeploy, extract_app_events,
};
use cordial_f1r3node_adapter::ordered_output::OrderedFinalizedOutput;
use cordial_miners_core::types::{BlockIdentity, NodeId};

#[test]
fn empty_ordered_output_produces_no_app_events() {
    let input = AppEventExtractionInput {
        ordered_output: OrderedFinalizedOutput::default(),
        deploys_by_block_hash: BTreeMap::new(),
    };

    let events = extract_app_events(input);

    assert!(events.is_empty());
}

#[test]
fn ordered_blocks_produce_events_in_finalized_order() {
    let first_block = block(2);
    let second_block = block(1);
    let input = AppEventExtractionInput {
        ordered_output: output(
            vec![first_block.clone(), second_block.clone()],
            Some(block(9)),
        ),
        deploys_by_block_hash: deploys_by_block_hash(vec![
            (
                second_block.content_hash.to_vec(),
                vec![deploy(
                    "beta",
                    "SecondBlockEvent",
                    b"second",
                    Some(vec![22]),
                )],
            ),
            (
                first_block.content_hash.to_vec(),
                vec![deploy("alpha", "FirstBlockEvent", b"first", Some(vec![11]))],
            ),
        ]),
    };

    let events = extract_app_events(input);

    assert_eq!(events.len(), 2);
    assert_eq!(events[0].app_id, app_id("alpha"));
    assert_eq!(events[0].event_type, "FirstBlockEvent");
    assert_eq!(events[0].payload, b"first");
    assert_eq!(events[0].ordered_index, 0);
    assert_eq!(events[0].block_hash, first_block.content_hash.to_vec());
    assert_eq!(events[1].app_id, app_id("beta"));
    assert_eq!(events[1].event_type, "SecondBlockEvent");
    assert_eq!(events[1].payload, b"second");
    assert_eq!(events[1].ordered_index, 1);
    assert_eq!(events[1].block_hash, second_block.content_hash.to_vec());
}

#[test]
fn multiple_deploys_in_one_block_preserve_deploy_order() {
    let ordered_block = block(3);
    let input = AppEventExtractionInput {
        ordered_output: output(vec![ordered_block.clone()], Some(block(9))),
        deploys_by_block_hash: deploys_by_block_hash(vec![(
            ordered_block.content_hash.to_vec(),
            vec![
                deploy("alpha", "FirstDeploy", b"first", Some(vec![1])),
                deploy("alpha", "SecondDeploy", b"second", Some(vec![2])),
            ],
        )]),
    };

    let events = extract_app_events(input);

    assert_eq!(events.len(), 2);
    assert_eq!(events[0].event_type, "FirstDeploy");
    assert_eq!(events[0].ordered_index, 0);
    assert_eq!(events[1].event_type, "SecondDeploy");
    assert_eq!(events[1].ordered_index, 1);
    assert_ne!(events[0].event_id, events[1].event_id);
}

#[test]
fn blocks_without_app_deploys_are_skipped() {
    let empty_block = block(1);
    let app_block = block(2);
    let later_empty_block = block(3);
    let input = AppEventExtractionInput {
        ordered_output: output(
            vec![empty_block, app_block.clone(), later_empty_block],
            Some(block(9)),
        ),
        deploys_by_block_hash: deploys_by_block_hash(vec![(
            app_block.content_hash.to_vec(),
            vec![deploy("alpha", "OnlyAppEvent", b"payload", None)],
        )]),
    };

    let events = extract_app_events(input);

    assert_eq!(events.len(), 1);
    assert_eq!(events[0].event_type, "OnlyAppEvent");
    assert_eq!(events[0].ordered_index, 0);
    assert_eq!(events[0].block_hash, app_block.content_hash.to_vec());
}

#[test]
fn finalized_anchor_is_copied_into_each_event() {
    let ordered_block = block(4);
    let anchor = block(9);
    let input = AppEventExtractionInput {
        ordered_output: output(vec![ordered_block.clone()], Some(anchor.clone())),
        deploys_by_block_hash: deploys_by_block_hash(vec![(
            ordered_block.content_hash.to_vec(),
            vec![
                deploy("alpha", "First", b"first", None),
                deploy("beta", "Second", b"second", None),
            ],
        )]),
    };

    let events = extract_app_events(input);

    assert_eq!(events.len(), 2);
    assert!(
        events
            .iter()
            .all(|event| event.finalized_anchor == anchor.content_hash.to_vec())
    );
}

#[test]
fn event_ids_are_deterministic_across_repeated_extraction() {
    let ordered_block = block(5);
    let input = AppEventExtractionInput {
        ordered_output: output(vec![ordered_block.clone()], Some(block(9))),
        deploys_by_block_hash: deploys_by_block_hash(vec![(
            ordered_block.content_hash.to_vec(),
            vec![
                deploy("alpha", "First", b"first", Some(vec![1, 2])),
                deploy("alpha", "Second", b"second", Some(vec![3, 4])),
            ],
        )]),
    };

    let first = extract_app_events(input.clone());
    let second = extract_app_events(input);

    assert_eq!(first, second);
    assert!(first.iter().all(|event| event.event_id.0.len() == 64));
}

#[test]
fn opaque_payload_bytes_are_not_decoded_or_mutated() {
    let ordered_block = block(6);
    let payload = vec![0, 255, b'{', b'}', b'\n'];
    let input = AppEventExtractionInput {
        ordered_output: output(vec![ordered_block.clone()], Some(block(9))),
        deploys_by_block_hash: deploys_by_block_hash(vec![(
            ordered_block.content_hash.to_vec(),
            vec![deploy("alpha", "OpaquePayload", &payload, Some(vec![6]))],
        )]),
    };

    let events = extract_app_events(input);

    assert_eq!(events.len(), 1);
    assert_eq!(events[0].payload, payload);
}

#[test]
fn event_fields_are_projected_from_extractable_deploy_metadata() {
    let ordered_block = block(7);
    let input = AppEventExtractionInput {
        ordered_output: output(vec![ordered_block.clone()], Some(block(9))),
        deploys_by_block_hash: deploys_by_block_hash(vec![(
            ordered_block.content_hash.to_vec(),
            vec![deploy(
                "identity.registry",
                "NameRegistered",
                b"alice",
                Some(vec![7, 7, 7]),
            )],
        )]),
    };

    let events = extract_app_events(input);
    let event = events.first().expect("one app event");

    assert_eq!(event.app_id, app_id("identity.registry"));
    assert_eq!(event.event_type, "NameRegistered");
    assert_eq!(event.payload, b"alice");
    assert_eq!(event.submitter, vec![42]);
    assert_eq!(event.deploy_signature, Some(vec![7, 7, 7]));
}

fn output(blocks: Vec<BlockIdentity>, anchor: Option<BlockIdentity>) -> OrderedFinalizedOutput {
    OrderedFinalizedOutput::new(blocks, anchor, 3, 1, 0).with_timestamp(0)
}

fn block(tag: u8) -> BlockIdentity {
    BlockIdentity {
        content_hash: [tag; 32],
        creator: NodeId(vec![tag]),
        signature: vec![tag; 64],
    }
}

fn deploy(
    app_id: &str,
    event_type: &str,
    payload: &[u8],
    deploy_signature: Option<Vec<u8>>,
) -> ExtractableAppDeploy {
    ExtractableAppDeploy {
        app_id: AppId(app_id.to_owned()),
        event_type: event_type.to_owned(),
        payload: payload.to_vec(),
        submitter: vec![42],
        deploy_signature,
    }
}

fn deploys_by_block_hash(
    entries: Vec<(Vec<u8>, Vec<ExtractableAppDeploy>)>,
) -> BTreeMap<Vec<u8>, Vec<ExtractableAppDeploy>> {
    entries.into_iter().collect()
}

fn app_id(value: &str) -> AppId {
    AppId(value.to_owned())
}
