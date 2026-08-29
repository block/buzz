use super::{ManagedAgentRecord, RuntimeReadinessStatus};

const LEGACY_RECORD_JSON: &str = r#"{
        "pubkey":"abc",
        "name":"legacy",
        "relay_url":"wss://example.test",
        "acp_command":"buzz-acp",
        "agent_command":"buzz-agent",
        "agent_args":[],
        "mcp_command":"",
        "turn_timeout_seconds":320,
        "system_prompt":null,
        "created_at":"2026-01-01T00:00:00Z",
        "updated_at":"2026-01-01T00:00:00Z",
        "last_started_at":null,
        "last_stopped_at":null,
        "last_exit_code":null,
        "last_error":null
    }"#;

#[test]
fn legacy_record_without_launch_runtime_id_deserializes() {
    let record: ManagedAgentRecord =
        serde_json::from_str(LEGACY_RECORD_JSON).expect("legacy record should deserialize");
    assert_eq!(record.launch_runtime_id, None);
}

#[test]
fn launch_runtime_id_stays_out_of_portable_definition_view() {
    let record: ManagedAgentRecord = serde_json::from_str(
        r#"{
                "pubkey":"abc",
                "name":"instance",
                "relay_url":"wss://example.test",
                "acp_command":"buzz-acp",
                "agent_command":"ompk",
                "agent_args":[],
                "mcp_command":"",
                "turn_timeout_seconds":320,
                "system_prompt":null,
                "created_at":"2026-01-01T00:00:00Z",
                "updated_at":"2026-01-01T00:00:00Z",
                "last_started_at":null,
                "last_stopped_at":null,
                "last_exit_code":null,
                "last_error":null,
                "slug":"portable-definition",
                "runtime":"claude",
                "launch_runtime_id":"ompk"
            }"#,
    )
    .expect("ID-backed record should deserialize");

    assert_eq!(record.launch_runtime_id.as_deref(), Some("ompk"));
    let persisted = serde_json::to_value(&record).expect("record should serialize");
    assert_eq!(persisted["launch_runtime_id"], "ompk");

    let portable = serde_json::to_value(
        record
            .to_definition_view()
            .expect("definition record should have a portable view"),
    )
    .expect("portable definition should serialize");
    assert_eq!(portable["runtime"], "claude");
    assert!(portable.get("launch_runtime_id").is_none());
}

#[test]
fn runtime_readiness_wire_values_are_frozen() {
    assert_eq!(
        serde_json::to_string(&RuntimeReadinessStatus::Ready).unwrap(),
        "\"ready\""
    );
    assert_eq!(
        serde_json::to_string(&RuntimeReadinessStatus::AuthenticationRequired).unwrap(),
        "\"authentication_required\""
    );
    assert_eq!(
        serde_json::to_string(&RuntimeReadinessStatus::ConfigurationRequired).unwrap(),
        "\"configuration_required\""
    );
    assert_eq!(
        serde_json::to_string(&RuntimeReadinessStatus::ModelUnavailable).unwrap(),
        "\"model_unavailable\""
    );
    assert_eq!(
        serde_json::to_string(&RuntimeReadinessStatus::Unknown).unwrap(),
        "\"unknown\""
    );
}
