use buzz_core::agent_activity::{
    AgentActivity, AgentActivityClass, AgentActivityFrame, AgentActivityStatus,
    AgentActivityToolKind, AgentActivityUsage, AGENT_ACTIVITY_FRAME_VERSION,
    AGENT_ACTIVITY_MAX_DURATION_MS, AGENT_ACTIVITY_MAX_FRAME_BYTES,
};
use chrono::{TimeZone, Utc};
use uuid::Uuid;

fn occurred_at() -> chrono::DateTime<Utc> {
    Utc.timestamp_opt(1_723_456_789, 0).single().unwrap()
}

fn turn(status: AgentActivityStatus) -> AgentActivity {
    AgentActivity {
        activity_id: Uuid::parse_str("63ca9483-c457-4b24-88de-1f14fa97c499").unwrap(),
        occurred_at: occurred_at(),
        activity_class: AgentActivityClass::Turn,
        status,
        tool_kind: None,
        duration_ms: None,
        usage: None,
    }
}

#[test]
fn valid_frame_round_trips_with_a_closed_camel_case_schema() {
    let frame = AgentActivityFrame {
        version: AGENT_ACTIVITY_FRAME_VERSION,
        activities: vec![
            turn(AgentActivityStatus::Started),
            AgentActivity {
                activity_id: Uuid::parse_str("dd55208d-05a9-41d1-8199-d57664885212").unwrap(),
                occurred_at: occurred_at(),
                activity_class: AgentActivityClass::Tool,
                status: AgentActivityStatus::Completed,
                tool_kind: Some(AgentActivityToolKind::Search),
                duration_ms: Some(325),
                usage: None,
            },
            AgentActivity {
                activity_id: Uuid::parse_str("684d0d5f-aacc-4670-9b63-72ecf805fa0d").unwrap(),
                occurred_at: occurred_at(),
                activity_class: AgentActivityClass::Usage,
                status: AgentActivityStatus::Completed,
                tool_kind: None,
                duration_ms: None,
                usage: Some(AgentActivityUsage {
                    input_tokens: Some(100),
                    output_tokens: Some(25),
                    total_tokens: Some(125),
                    cache_read_tokens: None,
                    cache_write_tokens: None,
                }),
            },
        ],
    };

    let json = frame.to_json().expect("valid frame");
    assert!(json.len() <= AGENT_ACTIVITY_MAX_FRAME_BYTES);
    assert!(json.contains("\"activityId\""));
    assert!(json.contains("\"occurredAt\""));
    assert!(json.contains("\"activityClass\":\"tool\""));
    assert!(json.contains("\"toolKind\":\"search\""));
    assert!(!json.contains("session"));
    assert!(!json.contains("prompt"));
    assert_eq!(AgentActivityFrame::parse(&json).unwrap(), frame);
}

#[test]
fn unknown_or_sensitive_fields_are_rejected_instead_of_ignored() {
    let hostile = r#"{
      "version": 1,
      "activities": [{
        "activityId": "63ca9483-c457-4b24-88de-1f14fa97c499",
        "occurredAt": "2024-08-12T08:39:49Z",
        "activityClass": "tool",
        "status": "running",
        "toolKind": "execute",
        "title": "cat /private/file",
        "arguments": {"path": "/private/file"},
        "result": "secret",
        "thought": "hidden reasoning",
        "sessionId": "raw-session-id"
      }]
    }"#;

    let error = AgentActivityFrame::parse(hostile).unwrap_err().to_string();
    assert!(error.contains("unknown field"), "unexpected error: {error}");
}

#[test]
fn explicit_null_optional_fields_are_rejected_canonically() {
    let cases = [
        r#"{"version":1,"activities":[{"activityId":"63ca9483-c457-4b24-88de-1f14fa97c499","occurredAt":"2024-08-12T08:39:49Z","activityClass":"turn","status":"started","toolKind":null}]}"#,
        r#"{"version":1,"activities":[{"activityId":"63ca9483-c457-4b24-88de-1f14fa97c499","occurredAt":"2024-08-12T08:39:49Z","activityClass":"turn","status":"completed","durationMs":null}]}"#,
        r#"{"version":1,"activities":[{"activityId":"63ca9483-c457-4b24-88de-1f14fa97c499","occurredAt":"2024-08-12T08:39:49Z","activityClass":"turn","status":"completed","usage":null}]}"#,
        r#"{"version":1,"activities":[{"activityId":"63ca9483-c457-4b24-88de-1f14fa97c499","occurredAt":"2024-08-12T08:39:49Z","activityClass":"usage","status":"completed","usage":{"inputTokens":null,"totalTokens":1}}]}"#,
    ];

    for content in cases {
        assert!(
            AgentActivityFrame::parse(content).is_err(),
            "explicit null must not be treated as an omitted optional field: {content}"
        );
    }
}

#[test]
fn invalid_class_specific_fields_fail_closed() {
    let mut bad_tool = turn(AgentActivityStatus::Running);
    bad_tool.activity_class = AgentActivityClass::Tool;
    let err = AgentActivityFrame {
        version: AGENT_ACTIVITY_FRAME_VERSION,
        activities: vec![bad_tool],
    }
    .to_json()
    .unwrap_err()
    .to_string();
    assert!(err.contains("toolKind"));

    let mut bad_usage = turn(AgentActivityStatus::Completed);
    bad_usage.activity_class = AgentActivityClass::Usage;
    let err = AgentActivityFrame {
        version: AGENT_ACTIVITY_FRAME_VERSION,
        activities: vec![bad_usage],
    }
    .to_json()
    .unwrap_err()
    .to_string();
    assert!(err.contains("usage"));

    let mut turn_with_tool = turn(AgentActivityStatus::Started);
    turn_with_tool.tool_kind = Some(AgentActivityToolKind::Other);
    assert!(AgentActivityFrame {
        version: AGENT_ACTIVITY_FRAME_VERSION,
        activities: vec![turn_with_tool],
    }
    .to_json()
    .is_err());
}

#[test]
fn version_cardinality_duration_and_byte_limits_are_enforced() {
    assert!(AgentActivityFrame {
        version: 2,
        activities: vec![turn(AgentActivityStatus::Started)],
    }
    .to_json()
    .is_err());

    assert!(AgentActivityFrame {
        version: AGENT_ACTIVITY_FRAME_VERSION,
        activities: vec![],
    }
    .to_json()
    .is_err());

    let mut too_long = turn(AgentActivityStatus::Completed);
    too_long.duration_ms = Some(AGENT_ACTIVITY_MAX_DURATION_MS + 1);
    assert!(AgentActivityFrame {
        version: AGENT_ACTIVITY_FRAME_VERSION,
        activities: vec![too_long],
    }
    .to_json()
    .is_err());

    let oversized = format!(
        "{{\"padding\":\"{}\"}}",
        "x".repeat(AGENT_ACTIVITY_MAX_FRAME_BYTES)
    );
    let error = AgentActivityFrame::parse(&oversized)
        .unwrap_err()
        .to_string();
    assert!(error.contains("too large"), "unexpected error: {error}");
}

#[test]
fn usage_requires_completed_status_and_at_least_one_count() {
    let usage = |status, counts| AgentActivity {
        activity_id: Uuid::new_v4(),
        occurred_at: occurred_at(),
        activity_class: AgentActivityClass::Usage,
        status,
        tool_kind: None,
        duration_ms: None,
        usage: Some(counts),
    };

    let empty = AgentActivityUsage {
        input_tokens: None,
        output_tokens: None,
        total_tokens: None,
        cache_read_tokens: None,
        cache_write_tokens: None,
    };
    assert!(AgentActivityFrame {
        version: AGENT_ACTIVITY_FRAME_VERSION,
        activities: vec![usage(AgentActivityStatus::Completed, empty)],
    }
    .to_json()
    .is_err());

    let counts = AgentActivityUsage {
        input_tokens: Some(1),
        output_tokens: None,
        total_tokens: Some(1),
        cache_read_tokens: None,
        cache_write_tokens: None,
    };
    assert!(AgentActivityFrame {
        version: AGENT_ACTIVITY_FRAME_VERSION,
        activities: vec![usage(AgentActivityStatus::Running, counts)],
    }
    .to_json()
    .is_err());
}
