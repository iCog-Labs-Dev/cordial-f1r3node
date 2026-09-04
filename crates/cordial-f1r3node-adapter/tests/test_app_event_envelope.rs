use cordial_app_runtime::AppId;
use cordial_f1r3node_adapter::app_event_envelope::{
    AppEventEnvelopeError, parse_app_deploy_envelope,
};
use cordial_f1r3node_adapter::block_translation::{DeployData, SignedDeployData};

#[test]
fn non_json_deploy_term_is_not_an_app_event() {
    let deploy = signed_deploy("@0!(\"regular rholang\")");

    let parsed = parse_app_deploy_envelope(&deploy).unwrap();

    assert_eq!(parsed, None);
}

#[test]
fn json_without_cordial_app_field_is_not_an_app_event() {
    let deploy = signed_deploy(r#"{"ordinary":"json"}"#);

    let parsed = parse_app_deploy_envelope(&deploy).unwrap();

    assert_eq!(parsed, None);
}

#[test]
fn valid_envelope_projects_app_deploy_metadata() {
    let deploy = signed_deploy(
        r#"{"cordial_app":{"version":1,"app_id":"identity.registry","event_type":"NameRegistered","payload_hex":"616c696365"}}"#,
    );

    let parsed = parse_app_deploy_envelope(&deploy)
        .unwrap()
        .expect("valid app envelope");

    assert_eq!(parsed.app_id, AppId("identity.registry".to_owned()));
    assert_eq!(parsed.event_type, "NameRegistered");
    assert_eq!(parsed.payload, b"alice");
    assert_eq!(parsed.submitter, vec![1, 2, 3]);
    assert_eq!(parsed.deploy_signature, Some(vec![9, 8, 7]));
}

#[test]
fn invalid_json_object_is_reported_as_invalid_json() {
    let deploy = signed_deploy(r#"{"cordial_app":"#);

    let err = parse_app_deploy_envelope(&deploy).unwrap_err();

    assert!(matches!(err, AppEventEnvelopeError::InvalidJson(_)));
}

#[test]
fn malformed_cordial_app_field_is_reported_as_invalid_envelope() {
    let deploy = signed_deploy(r#"{"cordial_app":"not an object"}"#);

    let err = parse_app_deploy_envelope(&deploy).unwrap_err();

    assert!(matches!(err, AppEventEnvelopeError::InvalidEnvelope(_)));
}

#[test]
fn unsupported_version_is_reported() {
    let deploy = signed_deploy(
        r#"{"cordial_app":{"version":2,"app_id":"identity.registry","event_type":"NameRegistered","payload_hex":"00"}}"#,
    );

    let err = parse_app_deploy_envelope(&deploy).unwrap_err();

    assert_eq!(
        err,
        AppEventEnvelopeError::UnsupportedVersion { version: 2 }
    );
}

#[test]
fn invalid_payload_hex_is_reported() {
    let deploy = signed_deploy(
        r#"{"cordial_app":{"version":1,"app_id":"identity.registry","event_type":"NameRegistered","payload_hex":"not-hex"}}"#,
    );

    let err = parse_app_deploy_envelope(&deploy).unwrap_err();

    assert!(matches!(err, AppEventEnvelopeError::InvalidPayloadHex(_)));
}

fn signed_deploy(term: &str) -> SignedDeployData {
    SignedDeployData {
        data: DeployData {
            term: term.to_owned(),
            time_stamp: 123,
            phlo_price: 1,
            phlo_limit: 100_000,
            valid_after_block_number: 0,
            shard_id: "root".to_owned(),
            expiration_timestamp: None,
        },
        pk: vec![1, 2, 3],
        sig: vec![9, 8, 7],
        sig_algorithm: "ed25519".to_owned(),
    }
}
