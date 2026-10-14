use buzz_core::WORKFLOW_TIMEZONE_EXTENSION;
use sha2::{Digest, Sha256};
use std::time::Duration;

use crate::client::{
    extract_d_tag, extract_relay_response_field, normalize_write_response, print_create_response,
    BuzzClient,
};
use crate::error::CliError;
use crate::validate::{parse_uuid, read_or_stdin, sdk_err, validate_uuid};

// TODO(phase-4): Replace raw nostr::EventBuilder usage with buzz-sdk builder functions

const MAX_RELAY_INFO_BYTES: usize = 64 * 1024;
const RELAY_INFO_CONNECT_TIMEOUT: Duration = Duration::from_secs(3);
const RELAY_INFO_TIMEOUT: Duration = Duration::from_secs(5);

fn workflow_schedule_has_timezone(yaml: &str) -> bool {
    let Ok(document) = serde_yaml::from_str::<serde_yaml::Value>(yaml) else {
        return false;
    };
    let Some(root) = document.as_mapping() else {
        return false;
    };
    let Some(trigger) = root
        .get(serde_yaml::Value::String("trigger".into()))
        .and_then(serde_yaml::Value::as_mapping)
    else {
        return false;
    };

    trigger
        .get(serde_yaml::Value::String("on".into()))
        .and_then(serde_yaml::Value::as_str)
        == Some("schedule")
        && trigger.contains_key(serde_yaml::Value::String("timezone".into()))
}

fn relay_supports_workflow_timezones(info: &serde_json::Value) -> bool {
    info.get("supported_extensions")
        .and_then(serde_json::Value::as_array)
        .is_some_and(|extensions| {
            extensions
                .iter()
                .any(|extension| extension.as_str() == Some(WORKFLOW_TIMEZONE_EXTENSION))
        })
}

async fn fetch_relay_info(client: &BuzzClient) -> Result<serde_json::Value, CliError> {
    let http = reqwest::Client::builder()
        .connect_timeout(RELAY_INFO_CONNECT_TIMEOUT)
        .redirect(reqwest::redirect::Policy::none())
        .timeout(RELAY_INFO_TIMEOUT)
        .build()
        .map_err(|error| CliError::Other(format!("failed to build relay info client: {error}")))?;
    let mut response = http
        .get(format!("{}/info", client.relay_url()))
        .header(reqwest::header::ACCEPT, "application/nostr+json")
        .send()
        .await?;
    let status = response.status();

    if response
        .content_length()
        .is_some_and(|length| length > MAX_RELAY_INFO_BYTES as u64)
    {
        return Err(CliError::Other(format!(
            "relay NIP-11 response exceeds {MAX_RELAY_INFO_BYTES} bytes"
        )));
    }

    let mut body = Vec::with_capacity(
        response
            .content_length()
            .unwrap_or_default()
            .min(MAX_RELAY_INFO_BYTES as u64) as usize,
    );
    while let Some(chunk) = response.chunk().await? {
        if body.len().saturating_add(chunk.len()) > MAX_RELAY_INFO_BYTES {
            return Err(CliError::Other(format!(
                "relay NIP-11 response exceeds {MAX_RELAY_INFO_BYTES} bytes"
            )));
        }
        body.extend_from_slice(&chunk);
    }

    if !status.is_success() {
        return Err(CliError::Relay {
            status: status.as_u16(),
            body: String::from_utf8_lossy(&body).into_owned(),
        });
    }

    serde_json::from_slice(&body)
        .map_err(|error| CliError::Other(format!("invalid NIP-11 response: {error}")))
}

async fn require_workflow_timezone_support(
    client: &BuzzClient,
    yaml: &str,
) -> Result<(), CliError> {
    if !workflow_schedule_has_timezone(yaml) {
        return Ok(());
    }

    let info = fetch_relay_info(client).await?;
    if relay_supports_workflow_timezones(&info) {
        return Ok(());
    }

    Err(CliError::Other(
        "this server cannot safely save schedules with a time zone; upgrade the server or remove trigger.timezone"
            .into(),
    ))
}

/// List workflows in a channel — query kind:30620 workflow definition events.
pub async fn cmd_list_workflows(client: &BuzzClient, channel_id: &str) -> Result<(), CliError> {
    validate_uuid(channel_id)?;
    let filter = serde_json::json!({
        "kinds": [30620],
        "#h": [channel_id]
    });
    let resp = client.query(&filter).await?;
    let events: Vec<serde_json::Value> = serde_json::from_str(&resp).unwrap_or_default();
    let workflows: Vec<serde_json::Value> = events
        .iter()
        .map(|e| {
            serde_json::json!({
                "workflow_id": extract_d_tag(e),
                "content": e.get("content").and_then(|v| v.as_str()).unwrap_or(""),
                "created_at": e.get("created_at").and_then(|v| v.as_u64()).unwrap_or(0),
                "pubkey": e.get("pubkey").and_then(|v| v.as_str()).unwrap_or(""),
            })
        })
        .collect();
    let output = serde_json::to_string(&workflows).unwrap_or_default();
    println!("{output}");
    Ok(())
}

/// Get a single workflow definition.
pub async fn cmd_get_workflow(client: &BuzzClient, workflow_id: &str) -> Result<(), CliError> {
    validate_uuid(workflow_id)?;
    let filter = serde_json::json!({
        "kinds": [30620],
        "#d": [workflow_id]
    });
    let resp = client.query(&filter).await?;
    let events: Vec<serde_json::Value> = serde_json::from_str(&resp).unwrap_or_default();
    if let Some(e) = events.first() {
        let normalized = serde_json::json!({
            "workflow_id": extract_d_tag(e),
            "content": e.get("content").and_then(|v| v.as_str()).unwrap_or(""),
            "created_at": e.get("created_at").and_then(|v| v.as_u64()).unwrap_or(0),
            "pubkey": e.get("pubkey").and_then(|v| v.as_str()).unwrap_or(""),
        });
        println!("{normalized}");
    } else {
        println!("null");
    }
    Ok(())
}

/// Get workflow run history — query kinds [46001, 46002, 46003].
///
/// NOTE: The relay does not currently emit workflow execution events (46001-46003).
/// Run history is stored in the workflow_runs DB table, not as Nostr events.
/// This command will return an empty array until the relay adds event emission
/// or a dedicated REST endpoint for run history.
pub async fn cmd_get_workflow_runs(
    client: &BuzzClient,
    workflow_id: &str,
    limit: Option<u32>,
) -> Result<(), CliError> {
    validate_uuid(workflow_id)?;
    let limit = limit.unwrap_or(20).min(100);
    let filter = serde_json::json!({
        "kinds": [46001, 46002, 46003],
        "#d": [workflow_id],
        "limit": limit
    });
    let resp = client.query(&filter).await?;
    let events: Vec<serde_json::Value> = serde_json::from_str(&resp).unwrap_or_default();
    let normalized: Vec<serde_json::Value> = events
        .iter()
        .map(|e| {
            serde_json::json!({
                "event_id": e.get("id").and_then(|v| v.as_str()).unwrap_or(""),
                "kind": e.get("kind").and_then(|v| v.as_u64()).unwrap_or(0),
                "content": e.get("content").and_then(|v| v.as_str()).unwrap_or(""),
                "created_at": e.get("created_at").and_then(|v| v.as_u64()).unwrap_or(0),
                "tags": e.get("tags").cloned().unwrap_or(serde_json::json!([])),
            })
        })
        .collect();
    let output = serde_json::to_string(&normalized).unwrap_or_default();
    println!("{output}");
    Ok(())
}

/// Create a workflow — sign and submit a kind:30620 event.
pub async fn cmd_create_workflow(
    client: &BuzzClient,
    channel_id: &str,
    yaml: &str,
) -> Result<(), CliError> {
    let channel_uuid = parse_uuid(channel_id)?;
    let yaml_definition = read_or_stdin(yaml)?;
    require_workflow_timezone_support(client, &yaml_definition).await?;

    let workflow_id = uuid::Uuid::new_v4();
    let builder = buzz_sdk::build_workflow_def(channel_uuid, workflow_id, &yaml_definition)
        .map_err(sdk_err)?;
    let event = client.sign_event(builder)?;

    let resp = client.submit_event(event).await?;
    let final_workflow_id = extract_relay_response_field(&resp, "workflow_id")
        .unwrap_or_else(|| workflow_id.to_string());
    print_create_response(&resp, "workflow_id", &final_workflow_id);
    Ok(())
}

/// Update a workflow — sign and submit an updated kind:30620 event with same d-tag.
pub async fn cmd_update_workflow(
    client: &BuzzClient,
    channel_id: &str,
    workflow_id: &str,
    yaml: &str,
) -> Result<(), CliError> {
    let channel_uuid = parse_uuid(channel_id)?;
    let wf_uuid = parse_uuid(workflow_id)?;
    let yaml_definition = read_or_stdin(yaml)?;
    require_workflow_timezone_support(client, &yaml_definition).await?;

    let filter = serde_json::json!({
        "kinds": [30620],
        "#d": [workflow_id]
    });
    let resp = client.query(&filter).await?;
    let events: Vec<serde_json::Value> = serde_json::from_str(&resp).unwrap_or_default();
    let expected_revision = events
        .first()
        .and_then(|event| event.get("id"))
        .and_then(|id| id.as_str())
        .ok_or_else(|| CliError::NotFound(format!("workflow {workflow_id} not found")))?;

    let builder =
        buzz_sdk::build_workflow_update(channel_uuid, wf_uuid, &yaml_definition, expected_revision)
            .map_err(sdk_err)?;
    let event = client.sign_event(builder)?;

    let resp = client.submit_event(event).await?;
    println!("{}", normalize_write_response(&resp));
    Ok(())
}

/// Delete a workflow — sign and submit a kind:5 deletion event.
pub async fn cmd_delete_workflow(client: &BuzzClient, workflow_id: &str) -> Result<(), CliError> {
    let wf_uuid = parse_uuid(workflow_id)?;
    let keys = client.keys();

    let builder =
        buzz_sdk::build_workflow_delete(&keys.public_key().to_hex(), wf_uuid).map_err(sdk_err)?;
    let event = client.sign_event(builder)?;

    let resp = client.submit_event(event).await?;
    println!("{}", normalize_write_response(&resp));
    Ok(())
}

/// Trigger a workflow — sign and submit a kind:46020 event.
///
/// When `inputs` is provided, it is parsed as a JSON object and used as the
/// event content (MCP parity). When omitted, the event content is `{}`.
pub async fn cmd_trigger_workflow(
    client: &BuzzClient,
    workflow_id: &str,
    inputs: Option<&str>,
) -> Result<(), CliError> {
    let wf_uuid = parse_uuid(workflow_id)?;

    if let Some(raw) = inputs {
        // Parse and validate it is a JSON object, then build the event manually
        // so we can embed the inputs as the event content.
        let parsed: serde_json::Value = serde_json::from_str(raw)
            .map_err(|e| CliError::Usage(format!("--inputs is not valid JSON: {e}")))?;
        if !parsed.is_object() {
            return Err(CliError::Usage("--inputs must be a JSON object".into()));
        }
        let content = serde_json::to_string(&parsed).unwrap_or_default();
        use nostr::{EventBuilder, Kind, Tag};
        let tags = vec![Tag::parse(["d", &wf_uuid.to_string()])
            .map_err(|e| CliError::Other(format!("tag error: {e}")))?];
        let builder = EventBuilder::new(
            Kind::Custom(buzz_sdk::kind::KIND_WORKFLOW_TRIGGER as u16),
            &content,
        )
        .tags(tags);
        let event = client.sign_event(builder)?;
        let resp = client.submit_event(event).await?;
        println!("{}", normalize_write_response(&resp));
    } else {
        let builder = buzz_sdk::build_workflow_trigger(wf_uuid).map_err(sdk_err)?;
        let event = client.sign_event(builder)?;
        let resp = client.submit_event(event).await?;
        println!("{}", normalize_write_response(&resp));
    }
    Ok(())
}

/// Approve or deny a workflow step — sign and submit a kind:46030 (grant) or 46031 (deny) event.
pub async fn cmd_approve_step(
    client: &BuzzClient,
    approval_token: &str,
    approved: bool,
    note: Option<&str>,
) -> Result<(), CliError> {
    validate_uuid(approval_token)?;

    let content = note.unwrap_or("");

    // The relay expects d-tag = hex(SHA256(token)), not the raw token UUID.
    let token_hash = hex::encode(Sha256::digest(approval_token.as_bytes()));
    let builder =
        buzz_sdk::build_workflow_approval(&token_hash, approved, content).map_err(sdk_err)?;
    let event = client.sign_event(builder)?;

    let resp = client.submit_event(event).await?;
    println!("{}", normalize_write_response(&resp));
    Ok(())
}

pub async fn dispatch(cmd: crate::WorkflowsCmd, client: &BuzzClient) -> Result<(), CliError> {
    use crate::WorkflowsCmd;
    match cmd {
        WorkflowsCmd::List { channel } => cmd_list_workflows(client, &channel).await,
        WorkflowsCmd::Get { workflow } => cmd_get_workflow(client, &workflow).await,
        WorkflowsCmd::Create { channel, yaml } => {
            cmd_create_workflow(client, &channel, &yaml).await
        }
        WorkflowsCmd::Update {
            channel,
            workflow,
            yaml,
        } => cmd_update_workflow(client, &channel, &workflow, &yaml).await,
        WorkflowsCmd::Delete { workflow } => cmd_delete_workflow(client, &workflow).await,
        WorkflowsCmd::Trigger { workflow, inputs } => {
            cmd_trigger_workflow(client, &workflow, inputs.as_deref()).await
        }
        WorkflowsCmd::Runs { workflow, limit } => {
            cmd_get_workflow_runs(client, &workflow, limit).await
        }
        WorkflowsCmd::Approve {
            token,
            approved,
            note,
        } => {
            // approved is already a bool — no parse_bool_flag needed
            cmd_approve_step(client, &token, approved, note.as_deref()).await
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::extract::State;
    use axum::http::StatusCode;
    use axum::routing::{get, post};
    use axum::Router;
    use nostr::Keys;
    use std::net::SocketAddr;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use tokio::net::TcpListener;

    const CHANNEL_ID: &str = "11111111-1111-4111-8111-111111111111";
    const WORKFLOW_ID: &str = "22222222-2222-4222-8222-222222222222";
    const ZONED_WORKFLOW: &str = r#"name: Zoned
trigger:
  on: schedule
  cron: 0 9 * * *
  timezone: America/Chicago
steps:
  - id: notify
    action: send_message
    text: hello
"#;

    #[derive(Clone)]
    struct FakeRelayState {
        event_requests: Arc<AtomicUsize>,
        info_body: Arc<str>,
        info_requests: Arc<AtomicUsize>,
    }

    async fn relay_info(State(state): State<FakeRelayState>) -> (StatusCode, String) {
        state.info_requests.fetch_add(1, Ordering::SeqCst);
        (StatusCode::OK, state.info_body.to_string())
    }

    async fn submit_event(State(state): State<FakeRelayState>) -> (StatusCode, &'static str) {
        state.event_requests.fetch_add(1, Ordering::SeqCst);
        (
            StatusCode::OK,
            r#"{"accepted":true,"event_id":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","message":"","workflow_id":"33333333-3333-4333-8333-333333333333"}"#,
        )
    }

    async fn fake_relay(info_body: &str) -> (BuzzClient, FakeRelayState) {
        let state = FakeRelayState {
            event_requests: Arc::new(AtomicUsize::new(0)),
            info_body: Arc::from(info_body),
            info_requests: Arc::new(AtomicUsize::new(0)),
        };
        let app = Router::new()
            .route("/info", get(relay_info))
            .route("/events", post(submit_event))
            .with_state(state.clone());
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr: SocketAddr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let client = BuzzClient::new(format!("http://{addr}"), Keys::generate(), None, None)
            .expect("test client");
        (client, state)
    }

    #[test]
    fn timezone_detection_is_limited_to_schedule_trigger_keys() {
        for yaml in [
            ZONED_WORKFLOW,
            "name: Zoned\ntrigger: { on: schedule, cron: '0 9 * * *', timezone: UTC }\n",
            "name: Invalid zone\ntrigger: { on: schedule, cron: '0 9 * * *', timezone: 5 }\n",
            "{\"name\":\"Zoned\",\"trigger\":{\"on\":\"schedule\",\"cron\":\"0 9 * * *\",\"timezone\":null}}",
        ] {
            assert!(workflow_schedule_has_timezone(yaml), "{yaml}");
        }

        for yaml in [
            "name: Legacy\ntrigger: { on: schedule, cron: '0 9 * * *' }\n",
            "name: Interval\ntrigger: { on: schedule, interval: 1h }\n",
            "name: Message\ntrigger: { on: message_posted, timezone: UTC }\n",
            "name: Text\ntrigger: { on: webhook }\nsteps: [{ id: s, action: send_message, text: 'timezone: UTC' }]\n",
        ] {
            assert!(!workflow_schedule_has_timezone(yaml), "{yaml}");
        }
    }

    #[test]
    fn extension_detection_requires_the_exact_advertised_token() {
        assert!(relay_supports_workflow_timezones(&serde_json::json!({
            "supported_extensions": [WORKFLOW_TIMEZONE_EXTENSION]
        })));
        for info in [
            serde_json::json!({}),
            serde_json::json!({"supported_extensions": []}),
            serde_json::json!({"supported_extensions": ["buzz-workflow-timezone-v2"]}),
            serde_json::json!({"supported_extensions": WORKFLOW_TIMEZONE_EXTENSION}),
        ] {
            assert!(!relay_supports_workflow_timezones(&info));
        }
    }

    #[tokio::test]
    async fn zoned_create_is_rejected_before_publish_when_support_is_absent() {
        let (client, state) = fake_relay(r#"{"supported_extensions":[]}"#).await;

        let error = cmd_create_workflow(&client, CHANNEL_ID, ZONED_WORKFLOW)
            .await
            .unwrap_err();

        assert!(error.to_string().contains("time zone"));
        assert_eq!(state.info_requests.load(Ordering::SeqCst), 1);
        assert_eq!(state.event_requests.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn zoned_update_is_rejected_before_query_or_publish_when_support_is_absent() {
        let (client, state) = fake_relay(r#"{"supported_extensions":[]}"#).await;

        let error = cmd_update_workflow(&client, CHANNEL_ID, WORKFLOW_ID, ZONED_WORKFLOW)
            .await
            .unwrap_err();

        assert!(error.to_string().contains("time zone"));
        assert_eq!(state.info_requests.load(Ordering::SeqCst), 1);
        assert_eq!(state.event_requests.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn oversized_relay_info_is_rejected_before_publish() {
        let oversized_info = "x".repeat(MAX_RELAY_INFO_BYTES + 1);
        let (client, state) = fake_relay(&oversized_info).await;

        let error = cmd_create_workflow(&client, CHANNEL_ID, ZONED_WORKFLOW)
            .await
            .unwrap_err();

        assert!(error.to_string().contains("exceeds"));
        assert_eq!(state.info_requests.load(Ordering::SeqCst), 1);
        assert_eq!(state.event_requests.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn legacy_schedules_bypass_the_capability_probe_and_publish() {
        let (client, state) = fake_relay(r#"{"supported_extensions":[]}"#).await;
        for yaml in [
            "name: Legacy\ntrigger: { on: schedule, cron: '0 9 * * *' }\nsteps: []\n",
            "name: Interval\ntrigger: { on: schedule, interval: 1h }\nsteps: []\n",
        ] {
            cmd_create_workflow(&client, CHANNEL_ID, yaml)
                .await
                .unwrap();
        }

        assert_eq!(state.info_requests.load(Ordering::SeqCst), 0);
        assert_eq!(state.event_requests.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn advertised_support_allows_zoned_workflow_publish() {
        let info = format!(r#"{{"supported_extensions":["{WORKFLOW_TIMEZONE_EXTENSION}"]}}"#);
        let (client, state) = fake_relay(&info).await;

        cmd_create_workflow(&client, CHANNEL_ID, ZONED_WORKFLOW)
            .await
            .unwrap();

        assert_eq!(state.info_requests.load(Ordering::SeqCst), 1);
        assert_eq!(state.event_requests.load(Ordering::SeqCst), 1);
    }
}
