use cordial_app_runtime::AppId;
use cordial_f1r3node_adapter::app_event_block_scan::{
    AppEventBlockScanError, scan_app_deploys_by_block_hash,
};
use cordial_f1r3node_adapter::app_event_envelope::AppEventEnvelopeError;
use cordial_f1r3node_adapter::block_translation::{
    BlockMessage, Body, DeployData, F1r3flyState, Header, ProcessedDeploy, SignedDeployData,
};

#[test]
fn empty_block_list_produces_empty_map() {
    let scan = scan_app_deploys_by_block_hash(&[]).unwrap();

    assert!(scan.deploys_by_block_hash.is_empty());
    assert!(scan.envelope_errors.is_empty());
}

#[test]
fn non_app_deploys_create_empty_scanned_block_entry() {
    let blocks = vec![block(
        1,
        vec!["@0!(\"regular rholang\")", r#"{"ordinary":"json"}"#],
    )];

    let scan = scan_app_deploys_by_block_hash(&blocks).unwrap();

    assert_eq!(
        scan.deploys_by_block_hash.get(&block_hash(1)),
        Some(&Vec::new())
    );
    assert!(scan.envelope_errors.is_empty());
}

#[test]
fn app_deploys_are_grouped_by_block_hash() {
    let blocks = vec![
        block(
            1,
            vec![app_term(
                "identity.registry",
                "NameRegistered",
                "616c696365",
            )],
        ),
        block(
            2,
            vec![app_term("payments.ledger", "PaymentRecorded", "010203")],
        ),
    ];

    let scan = scan_app_deploys_by_block_hash(&blocks).unwrap();

    assert_eq!(scan.deploys_by_block_hash.len(), 2);
    assert_eq!(
        scan.deploys_by_block_hash.get(&block_hash(1)).unwrap()[0].app_id,
        AppId("identity.registry".to_owned())
    );
    assert_eq!(
        scan.deploys_by_block_hash.get(&block_hash(2)).unwrap()[0].app_id,
        AppId("payments.ledger".to_owned())
    );
    assert!(scan.envelope_errors.is_empty());
}

#[test]
fn multiple_app_deploys_in_one_block_preserve_deploy_order() {
    let blocks = vec![block(
        7,
        vec![
            app_term("identity.registry", "First", "01"),
            "@0!(\"ordinary\")".to_owned(),
            app_term("identity.registry", "Second", "02"),
        ],
    )];

    let scan = scan_app_deploys_by_block_hash(&blocks).unwrap();
    let block_deploys = scan
        .deploys_by_block_hash
        .get(&block_hash(7))
        .expect("block app deploys");

    assert_eq!(block_deploys.len(), 2);
    assert_eq!(block_deploys[0].event_type, "First");
    assert_eq!(block_deploys[0].payload, vec![1]);
    assert_eq!(block_deploys[0].submitter, vec![7, 0]);
    assert_eq!(block_deploys[0].deploy_signature, Some(vec![7, 10]));
    assert_eq!(block_deploys[1].event_type, "Second");
    assert_eq!(block_deploys[1].payload, vec![2]);
    assert_eq!(block_deploys[1].submitter, vec![7, 2]);
    assert_eq!(block_deploys[1].deploy_signature, Some(vec![7, 12]));
    assert!(scan.envelope_errors.is_empty());
}

#[test]
fn malformed_app_envelope_is_reported_without_discarding_valid_deploys() {
    let blocks = vec![
        block(
            8,
            vec![app_term(
                "identity.registry",
                "NameRegistered",
                "616c696365",
            )],
        ),
        block(
            9,
            vec![
                "@0!(\"ordinary\")".to_owned(),
                r#"{"cordial_app":"#.to_owned(),
            ],
        ),
    ];

    let scan = scan_app_deploys_by_block_hash(&blocks).unwrap();

    assert_eq!(
        scan.deploys_by_block_hash.get(&block_hash(8)).unwrap()[0].app_id,
        AppId("identity.registry".to_owned())
    );
    assert_eq!(
        scan.deploys_by_block_hash.get(&block_hash(9)),
        Some(&Vec::new())
    );
    assert_eq!(scan.envelope_errors.len(), 1);
    assert_eq!(scan.envelope_errors[0].block_hash, block_hash(9));
    assert_eq!(scan.envelope_errors[0].deploy_index, 1);
    assert!(matches!(
        scan.envelope_errors[0].source,
        AppEventEnvelopeError::InvalidJson(_)
    ));
}

#[test]
fn duplicate_block_hashes_are_rejected() {
    let blocks = vec![
        block(5, Vec::<String>::new()),
        block(5, vec![app_term("app", "Event", "00")]),
    ];

    let err = scan_app_deploys_by_block_hash(&blocks).unwrap_err();

    assert_eq!(
        err,
        AppEventBlockScanError::DuplicateBlockHash {
            block_hash: block_hash(5)
        }
    );
}

fn block(tag: u8, terms: Vec<impl Into<String>>) -> BlockMessage {
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
