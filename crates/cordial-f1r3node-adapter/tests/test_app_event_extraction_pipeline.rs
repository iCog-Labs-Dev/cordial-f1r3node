use cordial_app_runtime::AppId;
use cordial_f1r3node_adapter::app_event_block_scan::AppEventBlockScanError;
use cordial_f1r3node_adapter::app_event_envelope::AppEventEnvelopeError;
use cordial_f1r3node_adapter::app_event_extraction_pipeline::{
    AppEventExtractionPipelineError, extract_app_events_from_blocks,
};
use cordial_f1r3node_adapter::app_event_extractor::AppEventExtractionError;
use cordial_f1r3node_adapter::block_translation::{
    BlockMessage, Body, DeployData, F1r3flyState, Header, ProcessedDeploy, SignedDeployData,
};
use cordial_f1r3node_adapter::ordered_output::OrderedFinalizedOutput;
use cordial_miners_core::types::{BlockIdentity, NodeId};

#[test]
fn composed_pipeline_emits_events_in_finalized_order() {
    let ordered_output = output(
        vec![block_identity(2), block_identity(1)],
        Some(block_identity(9)),
    );
    let blocks = vec![
        block_message(
            1,
            vec![app_term("identity.registry", "FirstSeenInBlockList", "01")],
        ),
        block_message(
            2,
            vec![app_term("payments.ledger", "FirstInFinalizedOrder", "02")],
        ),
    ];

    let extraction = extract_app_events_from_blocks(ordered_output, &blocks, 0).unwrap();

    assert!(extraction.envelope_errors.is_empty());
    assert_eq!(extraction.events.len(), 2);
    assert_eq!(
        extraction.events[0].app_id,
        AppId("payments.ledger".to_owned())
    );
    assert_eq!(extraction.events[0].event_type, "FirstInFinalizedOrder");
    assert_eq!(extraction.events[0].ordered_index, 0);
    assert_eq!(extraction.events[0].block_hash, block_hash(2));
    assert_eq!(
        extraction.events[1].app_id,
        AppId("identity.registry".to_owned())
    );
    assert_eq!(extraction.events[1].event_type, "FirstSeenInBlockList");
    assert_eq!(extraction.events[1].ordered_index, 1);
    assert_eq!(extraction.events[1].block_hash, block_hash(1));
}

#[test]
fn malformed_envelopes_are_reported_without_discarding_valid_events() {
    let ordered_output = output(
        vec![block_identity(3), block_identity(4)],
        Some(block_identity(9)),
    );
    let blocks = vec![
        block_message(
            3,
            vec![app_term(
                "identity.registry",
                "NameRegistered",
                "616c696365",
            )],
        ),
        block_message(
            4,
            vec![
                "@0!(\"ordinary\")".to_owned(),
                r#"{"cordial_app":"#.to_owned(),
            ],
        ),
    ];

    let extraction = extract_app_events_from_blocks(ordered_output, &blocks, 0).unwrap();

    assert_eq!(extraction.events.len(), 1);
    assert_eq!(extraction.events[0].event_type, "NameRegistered");
    assert_eq!(extraction.events[0].payload, b"alice");
    assert_eq!(extraction.envelope_errors.len(), 1);
    assert_eq!(extraction.envelope_errors[0].block_hash, block_hash(4));
    assert_eq!(extraction.envelope_errors[0].deploy_index, 1);
    assert!(matches!(
        extraction.envelope_errors[0].source,
        AppEventEnvelopeError::InvalidJson(_)
    ));
}

#[test]
fn missing_finalized_block_body_is_rejected() {
    let missing_block = block_identity(6);
    let ordered_output = output(
        vec![block_identity(5), missing_block.clone()],
        Some(block_identity(9)),
    );
    let blocks = vec![block_message(
        5,
        vec![app_term("identity.registry", "NameRegistered", "00")],
    )];

    let err = extract_app_events_from_blocks(ordered_output, &blocks, 0).unwrap_err();

    assert_eq!(
        err,
        AppEventExtractionPipelineError::Extraction(
            AppEventExtractionError::MissingFinalizedBlockDeploys {
                block_hash: missing_block.content_hash.to_vec()
            }
        )
    );
}

#[test]
fn duplicate_block_hash_is_rejected() {
    let ordered_output = output(vec![block_identity(7)], Some(block_identity(9)));
    let blocks = vec![
        block_message(7, Vec::<String>::new()),
        block_message(7, vec![app_term("identity.registry", "Duplicate", "00")]),
    ];

    let err = extract_app_events_from_blocks(ordered_output, &blocks, 0).unwrap_err();

    assert_eq!(
        err,
        AppEventExtractionPipelineError::BlockScan(AppEventBlockScanError::DuplicateBlockHash {
            block_hash: block_hash(7)
        })
    );
}

#[test]
fn starting_ordered_index_offsets_composed_events() {
    let ordered_output = output(vec![block_identity(8)], Some(block_identity(9)));
    let blocks = vec![block_message(
        8,
        vec![
            app_term("identity.registry", "First", "01"),
            app_term("identity.registry", "Second", "02"),
        ],
    )];

    let extraction = extract_app_events_from_blocks(ordered_output, &blocks, 10).unwrap();

    assert_eq!(extraction.events[0].ordered_index, 10);
    assert_eq!(extraction.events[1].ordered_index, 11);
}

fn output(blocks: Vec<BlockIdentity>, anchor: Option<BlockIdentity>) -> OrderedFinalizedOutput {
    OrderedFinalizedOutput::new(blocks, anchor, 3, 1, 0).with_timestamp(0)
}

fn block_identity(tag: u8) -> BlockIdentity {
    BlockIdentity {
        content_hash: [tag; 32],
        creator: NodeId(vec![tag]),
        signature: vec![tag; 64],
    }
}

fn block_message(tag: u8, terms: Vec<impl Into<String>>) -> BlockMessage {
    BlockMessage {
        block_hash: block_hash(tag),
        header: Header {
            parents_hash_list: vec![],
            timestamp: 0,
            version: 1,
            extra_bytes: vec![],
        },
        body: Body {
            state: F1r3flyState {
                pre_state_hash: vec![],
                post_state_hash: vec![],
                bonds: vec![],
                block_number: tag as i64,
            },
            deploys: terms
                .into_iter()
                .enumerate()
                .map(|(index, term)| processed_deploy(tag, index as u8, term.into()))
                .collect(),
            rejected_deploys: vec![],
            system_deploys: vec![],
            extra_bytes: vec![],
        },
        justifications: vec![],
        sender: vec![tag],
        seq_num: 0,
        sig: vec![tag; 64],
        sig_algorithm: "ed25519".to_owned(),
        shard_id: "root".to_owned(),
        extra_bytes: vec![],
    }
}

fn processed_deploy(block_tag: u8, deploy_tag: u8, term: String) -> ProcessedDeploy {
    ProcessedDeploy {
        deploy: SignedDeployData {
            data: DeployData {
                term,
                time_stamp: 123,
                phlo_price: 1,
                phlo_limit: 100_000,
                valid_after_block_number: 0,
                shard_id: "root".to_owned(),
                expiration_timestamp: None,
            },
            pk: vec![block_tag, deploy_tag],
            sig: vec![block_tag, deploy_tag + 10],
            sig_algorithm: "ed25519".to_owned(),
        },
        cost: 1,
        deploy_log: vec![],
        is_failed: false,
        system_deploy_error: None,
    }
}

fn app_term(app_id: &str, event_type: &str, payload_hex: &str) -> String {
    format!(
        r#"{{"cordial_app":{{"version":1,"app_id":"{app_id}","event_type":"{event_type}","payload_hex":"{payload_hex}"}}}}"#
    )
}

fn block_hash(tag: u8) -> Vec<u8> {
    vec![tag; 32]
}
