use sha2::{Digest, Sha256};

use crate::client::{
    create_response_with_id_if_accepted, extract_d_tag, extract_relay_response_field,
    normalize_write_response, BuzzClient,
};
use crate::error::CliError;
use crate::validate::{parse_uuid, read_or_stdin, sdk_err, validate_uuid};

// TODO(phase-4): Replace raw nostr::EventBuilder usage with buzz-sdk builder functions

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

/// Fetch one page of workflow run history from `GET /workflows/{id}/runs`.
///
/// Returns the relay's `{runs, next}` body. Relay failures and malformed
/// bodies are errors, never an empty list.
async fn fetch_workflow_runs(
    client: &BuzzClient,
    workflow_id: &str,
    limit: Option<u32>,
) -> Result<String, CliError> {
    validate_uuid(workflow_id)?;
    let limit = limit.unwrap_or(20).clamp(1, 100);
    let body = client
        .get_authed(&format!("/workflows/{workflow_id}/runs?limit={limit}"))
        .await?;
    require_array_field(&body, "runs")
}

/// Fetch the approvals of one run from `GET /workflows/{id}/runs/{run}/approvals`.
async fn fetch_run_approvals(
    client: &BuzzClient,
    workflow_id: &str,
    run_id: &str,
) -> Result<String, CliError> {
    validate_uuid(workflow_id)?;
    validate_uuid(run_id)?;
    let body = client
        .get_authed(&format!("/workflows/{workflow_id}/runs/{run_id}/approvals"))
        .await?;
    require_array_field(&body, "approvals")
}

fn require_array_field(body: &str, field: &str) -> Result<String, CliError> {
    let value: serde_json::Value = serde_json::from_str(body)
        .map_err(|e| CliError::Other(format!("relay returned invalid JSON: {e}")))?;
    if !value.get(field).is_some_and(|v| v.is_array()) {
        return Err(CliError::Other(format!(
            "relay response is missing the `{field}` array"
        )));
    }
    Ok(value.to_string())
}

/// Get workflow run history.
pub async fn cmd_get_workflow_runs(
    client: &BuzzClient,
    workflow_id: &str,
    limit: Option<u32>,
) -> Result<(), CliError> {
    println!("{}", fetch_workflow_runs(client, workflow_id, limit).await?);
    Ok(())
}

/// List the approvals of a workflow run.
pub async fn cmd_get_run_approvals(
    client: &BuzzClient,
    workflow_id: &str,
    run_id: &str,
) -> Result<(), CliError> {
    println!(
        "{}",
        fetch_run_approvals(client, workflow_id, run_id).await?
    );
    Ok(())
}

/// Sign and submit a new workflow, returning the CLI output JSON.
///
/// When the relay accepts a webhook-triggered workflow, its one-time
/// `webhook_secret` is surfaced as a top-level field.
async fn create_workflow_output(
    client: &BuzzClient,
    channel_id: &str,
    yaml: &str,
) -> Result<String, CliError> {
    let channel_uuid = parse_uuid(channel_id)?;
    let yaml_definition = read_or_stdin(yaml)?;

    let workflow_id = uuid::Uuid::new_v4();
    let builder = buzz_sdk::build_workflow_def(channel_uuid, workflow_id, &yaml_definition)
        .map_err(sdk_err)?;
    let event = client.sign_event(builder)?;

    let resp = client.submit_event(event).await?;
    let final_workflow_id = extract_relay_response_field(&resp, "workflow_id")
        .unwrap_or_else(|| workflow_id.to_string());
    let mut out: serde_json::Value = serde_json::from_str(&create_response_with_id_if_accepted(
        &resp,
        "workflow_id",
        &final_workflow_id,
    ))
    .map_err(|e| CliError::Other(format!("relay returned invalid JSON: {e}")))?;
    if out.get("accepted").and_then(|a| a.as_bool()) == Some(true) {
        if let Some(secret) = extract_relay_response_field(&resp, "webhook_secret") {
            out["webhook_secret"] = serde_json::json!(secret);
        }
    }
    Ok(out.to_string())
}

/// Create a workflow — sign and submit a kind:30620 event.
pub async fn cmd_create_workflow(
    client: &BuzzClient,
    channel_id: &str,
    yaml: &str,
) -> Result<(), CliError> {
    println!(
        "{}",
        create_workflow_output(client, channel_id, yaml).await?
    );
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
        WorkflowsCmd::Approvals { workflow, run } => {
            cmd_get_run_approvals(client, &workflow, &run).await
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
    use std::sync::{Arc, Mutex};

    use axum::{
        extract::{Path, RawQuery, State},
        http::StatusCode,
        routing::{get, post},
        Json, Router,
    };
    use nostr::Keys;
    use serde_json::{json, Value};

    use super::*;

    const WF: &str = "11111111-1111-4111-8111-111111111111";
    const RUN: &str = "22222222-2222-4222-8222-222222222222";

    type Seen = Arc<Mutex<Vec<String>>>;

    #[derive(Clone)]
    struct Relay {
        seen: Seen,
        runs: (StatusCode, String),
        approvals: (StatusCode, String),
        submit: String,
    }

    async fn fake_relay(relay: Relay) -> BuzzClient {
        let app = Router::new()
            .route(
                "/workflows/{wf}/runs",
                get(
                    |State(r): State<Relay>, Path(wf): Path<String>, RawQuery(q): RawQuery| async move {
                        r.seen.lock().unwrap().push(format!(
                            "GET /workflows/{wf}/runs?{}",
                            q.unwrap_or_default()
                        ));
                        r.runs
                    },
                ),
            )
            .route(
                "/workflows/{wf}/runs/{run}/approvals",
                get(
                    |State(r): State<Relay>, Path((wf, run)): Path<(String, String)>| async move {
                        r.seen
                            .lock()
                            .unwrap()
                            .push(format!("GET /workflows/{wf}/runs/{run}/approvals"));
                        r.approvals
                    },
                ),
            )
            .route(
                "/events",
                post(|State(r): State<Relay>| async move {
                    ([("content-type", "application/json")], Json(json!(
                        serde_json::from_str::<Value>(&r.submit).unwrap()
                    )))
                }),
            )
            .with_state(relay);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        BuzzClient::new(format!("http://{addr}"), Keys::generate(), None, None).unwrap()
    }

    fn relay() -> Relay {
        Relay {
            seen: Seen::default(),
            runs: (
                StatusCode::OK,
                json!({"runs": [], "next": null}).to_string(),
            ),
            approvals: (StatusCode::OK, json!({"approvals": []}).to_string()),
            submit: json!({"event_id": "ab", "accepted": true, "message": ""}).to_string(),
        }
    }

    #[tokio::test]
    async fn runs_reads_the_relay_endpoint_and_never_hides_failures() {
        let mut r = relay();
        let body = json!({
            "runs": [{"id": RUN, "workflow_id": WF, "status": "completed", "created_at": 5}],
            "next": {"before": 5, "before_id": RUN},
        });
        r.runs = (StatusCode::OK, body.to_string());
        let seen = r.seen.clone();
        let client = fake_relay(r).await;

        let out = fetch_workflow_runs(&client, WF, Some(500)).await.unwrap();
        assert_eq!(serde_json::from_str::<Value>(&out).unwrap(), body);
        fetch_workflow_runs(&client, WF, None).await.unwrap();
        fetch_workflow_runs(&client, WF, Some(0)).await.unwrap();
        assert_eq!(
            *seen.lock().unwrap(),
            vec![
                format!("GET /workflows/{WF}/runs?limit=100"),
                format!("GET /workflows/{WF}/runs?limit=20"),
                format!("GET /workflows/{WF}/runs?limit=1"),
            ]
        );

        assert!(matches!(
            fetch_workflow_runs(&client, "not-a-uuid", None).await,
            Err(CliError::Usage(_))
        ));

        for (status, body, want_relay) in [
            (
                StatusCode::NOT_FOUND,
                json!({"error": "workflow not found"}).to_string(),
                true,
            ),
            (
                StatusCode::UNAUTHORIZED,
                json!({"error": "auth"}).to_string(),
                true,
            ),
            (StatusCode::OK, "not json".to_string(), false),
            (StatusCode::OK, "[]".to_string(), false),
        ] {
            let mut r = relay();
            r.runs = (status, body);
            let client = fake_relay(r).await;
            let err = fetch_workflow_runs(&client, WF, None).await.unwrap_err();
            assert_eq!(matches!(err, CliError::Relay { .. }), want_relay, "{err:?}");
        }
    }

    #[tokio::test]
    async fn approvals_reads_the_run_endpoint_and_passes_refs_through() {
        let mut r = relay();
        let body =
            json!({"approvals": [{"approval_ref": "ab12", "run_id": RUN, "status": "pending"}]});
        r.approvals = (StatusCode::OK, body.to_string());
        let seen = r.seen.clone();
        let client = fake_relay(r).await;

        let out = fetch_run_approvals(&client, WF, RUN).await.unwrap();
        assert_eq!(serde_json::from_str::<Value>(&out).unwrap(), body);
        assert_eq!(
            *seen.lock().unwrap(),
            vec![format!("GET /workflows/{WF}/runs/{RUN}/approvals")]
        );

        assert!(matches!(
            fetch_run_approvals(&client, WF, "nope").await,
            Err(CliError::Usage(_))
        ));

        let mut r = relay();
        r.approvals = (
            StatusCode::NOT_FOUND,
            json!({"error": "workflow run not found"}).to_string(),
        );
        let client = fake_relay(r).await;
        assert!(matches!(
            fetch_run_approvals(&client, WF, RUN).await,
            Err(CliError::Relay { status: 404, .. })
        ));
    }

    #[tokio::test]
    async fn create_surfaces_the_webhook_secret_only_when_accepted() {
        let mut r = relay();
        r.submit = json!({
            "event_id": "ab", "accepted": true,
            "message": "response:{\"workflow_id\":\"33333333-3333-4333-8333-333333333333\",\"webhook_secret\":\"s3cret\"}",
        })
        .to_string();
        let client = fake_relay(r).await;
        let out: Value = serde_json::from_str(
            &create_workflow_output(&client, WF, "name: x")
                .await
                .unwrap(),
        )
        .unwrap();
        assert_eq!(out["webhook_secret"], "s3cret");
        assert_eq!(out["workflow_id"], "33333333-3333-4333-8333-333333333333");
        assert_eq!(out["event_id"], "ab");
        assert_eq!(out["accepted"], true);
        assert!(out["message"].as_str().unwrap().starts_with("response:"));

        let client = fake_relay(relay()).await;
        let out: Value = serde_json::from_str(
            &create_workflow_output(&client, WF, "name: x")
                .await
                .unwrap(),
        )
        .unwrap();
        assert!(out.get("webhook_secret").is_none());
        assert!(out["workflow_id"].is_string());

        let mut r = relay();
        r.submit = json!({
            "event_id": "ab", "accepted": false,
            "message": "response:{\"webhook_secret\":\"s3cret\"}",
        })
        .to_string();
        let client = fake_relay(r).await;
        let out: Value = serde_json::from_str(
            &create_workflow_output(&client, WF, "name: x")
                .await
                .unwrap(),
        )
        .unwrap();
        assert!(out.get("webhook_secret").is_none());
        assert!(out.get("workflow_id").is_none());
    }
}
