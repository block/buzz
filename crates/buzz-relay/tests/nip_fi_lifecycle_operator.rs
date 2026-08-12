use buzz_core::CommunityId;
use buzz_db::authorization_admission::{
    AdmissionApplicationResult, AdmissionApplicationResultSchema, AdmissionCommitDigests,
    AdmissionCommitReceipt, AdmissionObject, AdmissionObjectKind,
    MAX_ADMISSION_APPLICATION_RESULT_PAYLOAD_BYTES,
};
use uuid::Uuid;

const OPERATOR_API_SOURCE: &str = include_str!("../src/api/operator.rs");

#[test]
fn operator_allowlist_precedes_shared_replay_state() {
    let allowlist = OPERATOR_API_SOURCE
        .find(".relay_operator_pubkeys")
        .expect("operator allowlist decision");
    let replay = OPERATOR_API_SOURCE
        .find("check_operator_replay(state, event_id_bytes).await?")
        .expect("operator replay mutation");
    assert!(
        allowlist < replay,
        "unconfigured signers must fail before shared replay state"
    );
}

#[test]
fn canonical_result_api_is_bounded_versioned_and_object_bound() {
    let domain = CommunityId::from_uuid(Uuid::new_v4());
    let object = AdmissionObject::new(AdmissionObjectKind::Repository, [11; 32])
        .expect("server-owned repository coordinate");
    let schema =
        AdmissionApplicationResultSchema::new([12; 32], 1).expect("server-owned result schema");
    let result = AdmissionApplicationResult::new(schema, 1, b"stable-manifest".to_vec())
        .expect("bounded canonical result");
    assert_eq!(result.payload(), b"stable-manifest");
    assert_eq!(result.schema(), schema);
    assert!(AdmissionApplicationResult::new(
        schema,
        1,
        vec![0; MAX_ADMISSION_APPLICATION_RESULT_PAYLOAD_BYTES + 1],
    )
    .is_err());

    let receipt = AdmissionCommitReceipt::from_storage(
        domain,
        object,
        Uuid::new_v4(),
        [13; 32],
        [14; 32],
        AdmissionCommitDigests::new([15; 32], Some([16; 32])).expect("canonical result digests"),
        Uuid::new_v4(),
    )
    .expect("complete canonical receipt");
    assert_eq!(receipt.authorization_domain(), domain);
    assert_eq!(receipt.object(), object);
    assert_eq!(receipt.request_fingerprint(), &[13; 32]);
    assert_eq!(receipt.semantic_fingerprint(), &[14; 32]);
    assert_eq!(receipt.application_result_digest(), Some(&[16; 32]));
}
