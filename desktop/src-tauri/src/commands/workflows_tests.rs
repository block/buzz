// Tests for commands/workflows.rs — split into a sibling file to keep
// workflows.rs focused. These exercise the pure helpers (no relay): event →
// wire conversion, YAML definition parsing, name derivation, and the
// create/update record shaping.

use super::*;
use nostr::{EventBuilder, Keys, Kind, Tag};

/// Build a signed kind:30620 workflow definition event with the given YAML
/// content and d/h tags.
fn wf_event(d: &str, h: &str, yaml: &str) -> nostr::Event {
    let keys = Keys::generate();
    let tags: Vec<Tag> = [vec!["d", d], vec!["h", h]]
        .into_iter()
        .map(|t| Tag::parse(t).expect("parse tag"))
        .collect();
    EventBuilder::new(Kind::Custom(30620), yaml)
        .tags(tags)
        .sign_with_keys(&keys)
        .expect("sign")
}

const CHAN: &str = "11111111-1111-1111-1111-111111111111";
const WF: &str = "22222222-2222-2222-2222-222222222222";

const TIMEZONE_YAML: &str = "\
name: Local standup
trigger:
  on: schedule
  cron: '0 9 * * 1-5'
  timezone: America/Chicago
steps:
  - id: notify
    action: send_message
    text: Standup time
";

const YAML: &str = "\
name: Greet on join
description: Says hi
enabled: true
trigger:
  on: message_posted
  filter: hello
steps:
  - id: reply
    action: post_message
";

#[test]
fn workflow_from_event_maps_all_fields() {
    let ev = wf_event(WF, CHAN, YAML);
    let wf = workflow_from_event(&ev);

    assert_eq!(wf.id, WF);
    assert_eq!(wf.revision, ev.id.to_hex());
    assert_eq!(wf.channel_id.as_deref(), Some(CHAN));
    assert_eq!(wf.owner_pubkey, ev.pubkey.to_hex());
    assert_eq!(wf.name, "Greet on join");
    assert_eq!(wf.status, "active");
    assert_eq!(wf.created_at, ev.created_at.as_secs() as i64);
    assert_eq!(wf.updated_at, ev.created_at.as_secs() as i64);
}

#[test]
fn definition_is_parsed_into_object_with_nested_fields() {
    let ev = wf_event(WF, CHAN, YAML);
    let wf = workflow_from_event(&ev);

    // The whole YAML document is preserved as a free-form object.
    let def = wf.definition.as_object().expect("definition is an object");
    assert_eq!(
        def.get("description").and_then(Value::as_str),
        Some("Says hi")
    );
    assert_eq!(def.get("enabled").and_then(Value::as_bool), Some(true));
    assert_eq!(
        wf.definition.pointer("/trigger/on").and_then(Value::as_str),
        Some("message_posted")
    );
    assert_eq!(
        wf.definition
            .pointer("/steps/0/action")
            .and_then(Value::as_str),
        Some("post_message")
    );
}

#[test]
fn name_falls_back_to_id_when_missing() {
    let yaml = "trigger:\n  on: schedule\n  cron: '* * * * *'\n";
    let ev = wf_event(WF, CHAN, yaml);
    let wf = workflow_from_event(&ev);
    assert_eq!(wf.name, WF);
}

#[test]
fn name_falls_back_to_id_when_blank() {
    let yaml = "name: '   '\ntrigger:\n  on: schedule\n";
    let ev = wf_event(WF, CHAN, yaml);
    let wf = workflow_from_event(&ev);
    assert_eq!(wf.name, WF);
}

#[test]
fn malformed_yaml_yields_empty_object_not_error() {
    // A broken workflow must not break the whole list — definition falls back
    // to an empty object and the name falls back to the id. (YAML is permissive,
    // so this uses an unterminated flow mapping that genuinely fails to parse.)
    let ev = wf_event(WF, CHAN, "{ name: oops, unterminated: [1, 2");
    let wf = workflow_from_event(&ev);
    assert_eq!(wf.definition, Value::Object(serde_json::Map::new()));
    assert_eq!(wf.name, WF);
}

#[test]
fn scalar_yaml_document_yields_empty_object() {
    // A bare scalar parses as valid YAML but isn't an object; treat as empty.
    let ev = wf_event(WF, CHAN, "just a string");
    let wf = workflow_from_event(&ev);
    assert_eq!(wf.definition, Value::Object(serde_json::Map::new()));
}

#[test]
fn tag_value_reads_d_and_h_and_misses_absent() {
    let ev = wf_event(WF, CHAN, YAML);
    assert_eq!(tag_value(&ev, "d").as_deref(), Some(WF));
    assert_eq!(tag_value(&ev, "h").as_deref(), Some(CHAN));
    assert_eq!(tag_value(&ev, "z"), None);
}

#[test]
fn workflow_record_shapes_save_inputs() {
    let wf = workflow_record(
        WF.to_string(),
        "revision-1".to_string(),
        Some(CHAN.to_string()),
        "deadbeef".to_string(),
        YAML,
        100,
        200,
    );
    assert_eq!(wf.id, WF);
    assert_eq!(wf.name, "Greet on join");
    assert_eq!(wf.owner_pubkey, "deadbeef");
    assert_eq!(wf.channel_id.as_deref(), Some(CHAN));
    assert_eq!(wf.created_at, 100);
    assert_eq!(wf.updated_at, 200);
    assert_eq!(wf.status, "active");
}

#[test]
fn save_wire_serializes_flat_with_optional_secret() {
    let workflow = workflow_record(
        WF.to_string(),
        "revision-1".to_string(),
        Some(CHAN.to_string()),
        "deadbeef".to_string(),
        YAML,
        1,
        1,
    );

    // With a secret: present, flattened alongside the workflow fields.
    let with = WorkflowSaveWire {
        workflow: workflow.clone(),
        webhook_secret: Some("s3cr3t".to_string()),
    };
    let v = serde_json::to_value(&with).expect("serialize");
    assert_eq!(v.get("id").and_then(Value::as_str), Some(WF));
    assert_eq!(v.get("name").and_then(Value::as_str), Some("Greet on join"));
    assert_eq!(
        v.get("webhook_secret").and_then(Value::as_str),
        Some("s3cr3t")
    );

    // Without a secret: the key is omitted entirely (frontend treats as null).
    let without = WorkflowSaveWire {
        workflow,
        webhook_secret: None,
    };
    let v = serde_json::to_value(&without).expect("serialize");
    assert!(v.get("webhook_secret").is_none());
    assert_eq!(v.get("id").and_then(Value::as_str), Some(WF));
}

#[test]
fn workflow_wire_serializes_with_snake_case_keys() {
    // Guard the wire contract the frontend's RawWorkflow depends on.
    let ev = wf_event(WF, CHAN, YAML);
    let v = serde_json::to_value(workflow_from_event(&ev)).expect("serialize");
    for key in [
        "id",
        "revision",
        "name",
        "owner_pubkey",
        "channel_id",
        "definition",
        "status",
        "created_at",
        "updated_at",
    ] {
        assert!(v.get(key).is_some(), "missing wire key: {key}");
    }
}

#[test]
fn multi_channel_workflow_query_uses_one_filter_per_channel() {
    let other_channel = "33333333-3333-3333-3333-333333333333";
    let filters = channel_workflow_filters(vec![CHAN.to_string(), other_channel.to_string()])
        .expect("valid channels");

    assert_eq!(filters.len(), 2);
    assert_eq!(
        filters[0],
        serde_json::json!({
            "kinds": [30620],
            "#h": [CHAN],
        })
    );
    assert_eq!(
        filters[1],
        serde_json::json!({
            "kinds": [30620],
            "#h": [other_channel],
        })
    );
}

#[test]
fn workflow_queries_respect_relay_explicit_channel_limit() {
    for (channel_count, expected_batch_sizes) in [
        (WORKFLOW_QUERY_CHANNEL_BATCH_SIZE, vec![128]),
        (WORKFLOW_QUERY_CHANNEL_BATCH_SIZE + 1, vec![128, 1]),
    ] {
        let channel_ids = (0..channel_count)
            .map(|index| uuid::Uuid::from_u128(index as u128 + 1).to_string())
            .collect();
        let batches = channel_workflow_filter_batches(channel_ids).expect("valid channels");

        assert_eq!(
            batches.iter().map(Vec::len).collect::<Vec<_>>(),
            expected_batch_sizes
        );
        assert!(batches.iter().flatten().all(|filter| filter["#h"]
            .as_array()
            .is_some_and(|values| values.len() == 1)));
    }
}

#[test]
fn workflow_query_results_are_deduplicated_by_event_id() {
    let first = wf_event(WF, CHAN, YAML);
    let second_workflow = "33333333-3333-3333-3333-333333333333";
    let second = wf_event(second_workflow, CHAN, YAML);
    let mut workflows = Vec::new();
    let mut seen_event_ids = HashSet::new();

    append_unique_workflows(
        &mut workflows,
        &mut seen_event_ids,
        &[first.clone(), second.clone()],
    );
    append_unique_workflows(&mut workflows, &mut seen_event_ids, &[first, second]);

    assert_eq!(workflows.len(), 2);
    assert_eq!(workflows[0].id, WF);
    assert_eq!(workflows[1].id, second_workflow);
}

#[test]
fn channel_workflow_filters_reject_malformed_or_blank_channel_ids() {
    for channel_id in ["not-a-uuid", "", "   "] {
        let error = channel_workflow_filters(vec![channel_id.to_string()])
            .expect_err("malformed channel id must fail before querying the relay");
        assert_eq!(error, "invalid channel id");
    }
}

#[test]
fn channel_workflow_filters_accepts_empty_input() {
    assert_eq!(
        channel_workflow_filters(Vec::new()).expect("empty input is valid"),
        Vec::<Value>::new()
    );
}

#[test]
fn trigger_response_uses_persisted_run_id_contract() {
    let wire = trigger_wire_from_message(
        WF.to_string(),
        "response:{\"run_id\":\"33333333-3333-3333-3333-333333333333\"}",
    )
    .expect("parse trigger response");

    assert_eq!(wire.run_id, "33333333-3333-3333-3333-333333333333");
    assert_eq!(wire.workflow_id, WF);
    assert_eq!(wire.status, "pending");
    let value = serde_json::to_value(wire).expect("serialize trigger response");
    assert!(value.get("event_id").is_none());
}

#[test]
fn trigger_response_rejects_missing_or_empty_run_id() {
    assert!(trigger_wire_from_message(WF.to_string(), "response:{}").is_err());
    assert!(trigger_wire_from_message(WF.to_string(), "response:{\"run_id\":\"   \"}",).is_err());
}

#[test]
fn run_reads_serialize_to_backend_envelopes() {
    let runs = WorkflowRunsWire {
        runs: Vec::new(),
        next: None,
    };
    let approvals = WorkflowApprovalsWire {
        approvals: Vec::new(),
    };
    assert_eq!(
        serde_json::to_value(runs).expect("serialize runs"),
        serde_json::json!({ "runs": [], "next": null })
    );
    assert_eq!(
        serde_json::to_value(approvals).expect("serialize approvals"),
        serde_json::json!({ "approvals": [] })
    );
}

async fn write_http_response(stream: &mut tokio::net::TcpStream, content_type: &str, body: &str) {
    use tokio::io::AsyncWriteExt;

    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(response.as_bytes()).await.unwrap();
}

async fn read_http_request(stream: &mut tokio::net::TcpStream) -> String {
    use tokio::io::AsyncReadExt;

    let mut request = Vec::new();
    loop {
        let mut chunk = [0_u8; 4096];
        let read = stream.read(&mut chunk).await.unwrap();
        if read == 0 {
            break;
        }
        request.extend_from_slice(&chunk[..read]);
        assert!(request.len() <= 1_048_576, "test request exceeded 1 MiB");

        let Some(header_end) = request.windows(4).position(|window| window == b"\r\n\r\n") else {
            continue;
        };
        let headers = String::from_utf8_lossy(&request[..header_end]).to_ascii_lowercase();
        let content_length = headers
            .lines()
            .find_map(|line| line.strip_prefix("content-length:"))
            .and_then(|value| value.trim().parse::<usize>().ok())
            .unwrap_or_default();
        if request.len() >= header_end + 4 + content_length {
            break;
        }
    }
    String::from_utf8_lossy(&request).to_string()
}

async fn serve_workflow_save(
    nip11_body: &'static str,
    expect_event: bool,
) -> (std::net::SocketAddr, tokio::task::JoinHandle<Vec<String>>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let mut requests = Vec::new();
        let (mut info_stream, _) = listener.accept().await.unwrap();
        requests.push(read_http_request(&mut info_stream).await);
        write_http_response(&mut info_stream, "application/nostr+json", nip11_body).await;

        let event_connection = tokio::time::timeout(
            std::time::Duration::from_millis(if expect_event { 2_000 } else { 250 }),
            listener.accept(),
        )
        .await;
        if let Ok(Ok((mut event_stream, _))) = event_connection {
            requests.push(read_http_request(&mut event_stream).await);
            write_http_response(
                &mut event_stream,
                "application/json",
                r#"{"event_id":"saved-event","accepted":true,"message":"response:{}"}"#,
            )
            .await;
        }
        requests
    });
    (addr, server)
}

async fn serve_workflow_update(
    prior_event_body: String,
    nip11_body: &'static str,
) -> (std::net::SocketAddr, tokio::task::JoinHandle<Vec<String>>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let mut requests = Vec::new();

        let (mut query_stream, _) = listener.accept().await.unwrap();
        requests.push(read_http_request(&mut query_stream).await);
        write_http_response(&mut query_stream, "application/json", &prior_event_body).await;

        let (mut info_stream, _) = listener.accept().await.unwrap();
        requests.push(read_http_request(&mut info_stream).await);
        write_http_response(&mut info_stream, "application/nostr+json", nip11_body).await;

        if let Ok(Ok((mut event_stream, _))) =
            tokio::time::timeout(std::time::Duration::from_millis(250), listener.accept()).await
        {
            requests.push(read_http_request(&mut event_stream).await);
            write_http_response(
                &mut event_stream,
                "application/json",
                r#"{"event_id":"saved-event","accepted":true,"message":"response:{}"}"#,
            )
            .await;
        }
        requests
    });
    (addr, server)
}

#[tokio::test]
async fn create_timezone_workflow_probes_then_publishes_to_supported_relay() {
    use tauri::Manager;

    let _serial = crate::relay_admission::TEST_SERIAL.lock().await;
    crate::relay_admission::reset_rate_limit_gate();
    let (addr, server) = serve_workflow_save(
        r#"{"supported_extensions":["buzz-workflow-timezone-v1"]}"#,
        true,
    )
    .await;
    let state = crate::app_state::build_app_state();
    *state.relay_url_override.lock().unwrap() = Some(format!("ws://{addr}"));
    let app = tauri::test::mock_builder()
        .manage(state)
        .build(tauri::test::mock_context(tauri::test::noop_assets()))
        .unwrap();

    let saved = create_workflow(CHAN.to_string(), TIMEZONE_YAML.to_string(), app.state())
        .await
        .expect("advertised timezone support permits publication");
    assert_eq!(saved.workflow.revision, "saved-event");

    let requests = server.await.unwrap();
    assert_eq!(requests.len(), 2, "capability GET then workflow POST");
    assert!(requests[0].starts_with("GET /info "), "{}", requests[0]);
    assert!(
        requests[0]
            .to_ascii_lowercase()
            .contains("accept: application/nostr+json"),
        "{}",
        requests[0]
    );
    assert!(requests[1].starts_with("POST /events "), "{}", requests[1]);
    assert!(requests[1].contains("America/Chicago"));
    crate::relay_admission::reset_rate_limit_gate();
}

#[tokio::test]
async fn create_timezone_workflow_does_not_publish_when_extension_is_absent() {
    use tauri::Manager;

    let _serial = crate::relay_admission::TEST_SERIAL.lock().await;
    crate::relay_admission::reset_rate_limit_gate();
    let (addr, server) = serve_workflow_save(r#"{"supported_extensions":["nip-ar"]}"#, false).await;
    let state = crate::app_state::build_app_state();
    *state.relay_url_override.lock().unwrap() = Some(format!("ws://{addr}"));
    let app = tauri::test::mock_builder()
        .manage(state)
        .build(tauri::test::mock_context(tauri::test::noop_assets()))
        .unwrap();

    let error = create_workflow(CHAN.to_string(), TIMEZONE_YAML.to_string(), app.state())
        .await
        .expect_err("an older relay must block timezone workflow publication");
    assert_eq!(error, WORKFLOW_TIMEZONE_UNSUPPORTED_ERROR);

    let requests = server.await.unwrap();
    assert_eq!(requests.len(), 1, "no workflow event may be published");
    assert!(requests[0].starts_with("GET /info "), "{}", requests[0]);
    crate::relay_admission::reset_rate_limit_gate();
}

#[tokio::test]
async fn update_timezone_workflow_does_not_publish_when_extension_is_absent() {
    use tauri::Manager;

    let _serial = crate::relay_admission::TEST_SERIAL.lock().await;
    crate::relay_admission::reset_rate_limit_gate();
    let prior = wf_event(WF, CHAN, YAML);
    let prior_body = serde_json::to_string(&vec![prior.clone()]).unwrap();
    let (addr, server) = serve_workflow_update(prior_body, r#"{"supported_extensions":[]}"#).await;
    let state = crate::app_state::build_app_state();
    *state.relay_url_override.lock().unwrap() = Some(format!("ws://{addr}"));
    let app = tauri::test::mock_builder()
        .manage(state)
        .build(tauri::test::mock_context(tauri::test::noop_assets()))
        .unwrap();

    let error = update_workflow(
        WF.to_string(),
        TIMEZONE_YAML.to_string(),
        prior.id.to_hex(),
        app.state(),
    )
    .await
    .expect_err("an older relay must block timezone workflow updates");
    assert_eq!(error, WORKFLOW_TIMEZONE_UNSUPPORTED_ERROR);

    let requests = server.await.unwrap();
    assert_eq!(requests.len(), 2, "no update event may be published");
    assert!(requests[0].starts_with("POST /query "), "{}", requests[0]);
    assert!(requests[1].starts_with("GET /info "), "{}", requests[1]);
    crate::relay_admission::reset_rate_limit_gate();
}

#[tokio::test]
async fn timezone_gate_fails_closed_on_malformed_or_unreachable_capability_document() {
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();
    let (addr, server) = serve_workflow_save("{", false).await;
    let malformed = build_workflow_definition_for_save(
        WF,
        CHAN,
        TIMEZONE_YAML,
        None,
        &format!("http://{addr}"),
        &client,
    )
    .await
    .expect_err("malformed capability documents must fail closed");
    assert_eq!(malformed, WORKFLOW_TIMEZONE_CHECK_ERROR);
    assert_eq!(server.await.unwrap().len(), 1);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let unavailable_addr = listener.local_addr().unwrap();
    drop(listener);
    let unavailable = build_workflow_definition_for_save(
        WF,
        CHAN,
        TIMEZONE_YAML,
        None,
        &format!("http://{unavailable_addr}"),
        &client,
    )
    .await
    .expect_err("transport failures must fail closed");
    assert_eq!(unavailable, WORKFLOW_TIMEZONE_CHECK_ERROR);
}

#[tokio::test]
async fn timezone_gate_rejects_a_capability_document_from_another_origin() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let destination = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let destination_addr = destination.local_addr().unwrap();
    let destination_server = tokio::spawn(async move {
        let (mut stream, _) = destination.accept().await.unwrap();
        let mut request = [0_u8; 1024];
        let _ = stream.read(&mut request).await.unwrap();
        write_http_response(
            &mut stream,
            "application/nostr+json",
            r#"{"supported_extensions":["buzz-workflow-timezone-v1"]}"#,
        )
        .await;
    });

    let source = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let source_addr = source.local_addr().unwrap();
    let source_server = tokio::spawn(async move {
        let (mut stream, _) = source.accept().await.unwrap();
        let mut request = [0_u8; 1024];
        let _ = stream.read(&mut request).await.unwrap();
        let response = format!(
            "HTTP/1.1 302 Found\r\nLocation: http://{destination_addr}/info\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
        );
        stream.write_all(response.as_bytes()).await.unwrap();
    });

    let error = build_workflow_definition_for_save(
        WF,
        CHAN,
        TIMEZONE_YAML,
        None,
        &format!("http://{source_addr}"),
        &reqwest::Client::new(),
    )
    .await
    .expect_err("a redirected capability must not authorize the source relay");
    assert_eq!(error, WORKFLOW_TIMEZONE_CHECK_ERROR);
    source_server.await.unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(2), destination_server)
        .await
        .expect("redirect-following test client did not reach the destination")
        .unwrap();
}

#[tokio::test]
async fn legacy_utc_and_interval_schedules_bypass_capability_probe() {
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();
    for yaml in [
        "name: UTC cron\ntrigger:\n  on: schedule\n  cron: '0 9 * * *'\nsteps: []\n",
        "name: Interval\ntrigger:\n  on: schedule\n  interval: 1h\nsteps: []\n",
    ] {
        build_workflow_definition_for_save(WF, CHAN, yaml, None, "http://127.0.0.1:9", &client)
            .await
            .expect("legacy schedules must not require a relay probe");
    }
}

#[test]
fn every_explicit_schedule_timezone_field_requires_the_gate() {
    for timezone in ["America/Chicago", "null", "42"] {
        let yaml = format!(
            "name: Zoned\ntrigger:\n  on: schedule\n  cron: '0 9 * * *'\n  timezone: {timezone}\nsteps: []\n"
        );
        assert!(workflow_uses_schedule_timezone(&yaml), "{timezone}");
    }
}
