use buzz_runtime::{
    JobStartRequest, ProtocolError, MAX_ARGV_ELEMENTS, MAX_ARG_BYTES, MAX_CWD_BYTES,
    MAX_SUMMARY_BYTES,
};
use uuid::Uuid;

fn request() -> JobStartRequest {
    JobStartRequest {
        channel_id: Uuid::new_v4(),
        source_event_id: None,
        driver: "lh".into(),
        argv: vec!["lockdown".into()],
        cwd: "/workspace".into(),
        summary: "run governed work".into(),
    }
}

#[test]
fn strict_job_request_rejects_unknown_and_secret_bearing_fields() {
    let value = serde_json::json!({
        "channelId": Uuid::new_v4(), "sourceEventId": null, "driver": "lh", "argv": [],
        "cwd": "/workspace", "summary": "work", "env": {"SECRET": "sentinel"}
    });
    assert!(serde_json::from_value::<JobStartRequest>(value).is_err());
}

#[test]
fn job_request_enforces_each_exact_size_boundary() {
    let mut value = request();
    value.argv = vec!["x".repeat(MAX_ARG_BYTES + 1)];
    assert!(matches!(
        value.validate(),
        Err(ProtocolError::BoundExceeded("argv element"))
    ));

    let mut value = request();
    value.argv = vec![String::new(); MAX_ARGV_ELEMENTS + 1];
    assert!(matches!(
        value.validate(),
        Err(ProtocolError::BoundExceeded("argv"))
    ));

    let mut value = request();
    value.argv = vec!["x".repeat(MAX_ARG_BYTES); 9];
    assert!(matches!(
        value.validate(),
        Err(ProtocolError::BoundExceeded("argv json"))
    ));

    let mut value = request();
    value.cwd = "x".repeat(MAX_CWD_BYTES + 1);
    assert!(matches!(
        value.validate(),
        Err(ProtocolError::BoundExceeded("cwd"))
    ));

    let mut value = request();
    value.summary = "x".repeat(MAX_SUMMARY_BYTES + 1);
    assert!(matches!(
        value.validate(),
        Err(ProtocolError::BoundExceeded("summary"))
    ));
}
