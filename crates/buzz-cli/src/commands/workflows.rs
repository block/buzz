use sha2::{Digest, Sha256};

use crate::client::{
    extract_d_tag, extract_relay_response_field, normalize_write_response, print_create_response,
    BuzzClient,
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

/// Read one authenticated page of relay-owned workflow runs.
pub async fn cmd_get_workflow_runs(
    client: &BuzzClient,
    workflow_id: &str,
    limit: Option<u32>,
    before: Option<&str>,
    before_id: Option<&str>,
) -> Result<(), CliError> {
    let path = workflow_runs_path(workflow_id, limit, before, before_id)?;
    let response = client.get_authed(&path).await?;
    let page = parse_workflow_runs_page(&response)?;
    println!("{page}");
    Ok(())
}

fn workflow_runs_path(
    workflow_id: &str,
    limit: Option<u32>,
    before: Option<&str>,
    before_id: Option<&str>,
) -> Result<String, CliError> {
    let workflow_id = parse_uuid(workflow_id)?;
    let limit = limit.unwrap_or(20).min(100);
    if limit == 0 {
        return Err(CliError::Usage("limit must be greater than zero".into()));
    }
    if before.is_some() != before_id.is_some() {
        return Err(CliError::Usage(
            "before and before-id must be supplied together".into(),
        ));
    }
    let mut query = url::form_urlencoded::Serializer::new(String::new());
    query.append_pair("limit", &limit.to_string());
    if let (Some(before), Some(before_id)) = (before, before_id) {
        chrono::DateTime::parse_from_rfc3339(before)
            .map_err(|_| CliError::Usage("before must be an RFC 3339 timestamp".into()))?;
        validate_uuid(before_id)?;
        query
            .append_pair("before", before)
            .append_pair("before_id", before_id);
    }
    Ok(format!("/workflows/{workflow_id}/runs?{}", query.finish()))
}

fn parse_workflow_runs_page(response: &str) -> Result<serde_json::Value, CliError> {
    let page: serde_json::Value = serde_json::from_str(response)
        .map_err(|e| CliError::Other(format!("invalid workflow run-history response: {e}")))?;
    if !page.get("runs").is_some_and(serde_json::Value::is_array)
        || !page.get("next").is_some_and(|next| {
            next.is_null()
                || (next.get("before").is_some_and(serde_json::Value::is_string)
                    && next
                        .get("before_id")
                        .is_some_and(serde_json::Value::is_string))
        })
    {
        return Err(CliError::Other(
            "invalid workflow run-history page: expected runs and next".into(),
        ));
    }
    Ok(page)
}

/// Create a workflow — sign and submit a kind:30620 event.
pub async fn cmd_create_workflow(
    client: &BuzzClient,
    channel_id: &str,
    yaml: &str,
) -> Result<(), CliError> {
    let channel_uuid = parse_uuid(channel_id)?;
    let yaml_definition = read_or_stdin(yaml)?;

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
        WorkflowsCmd::Runs {
            workflow,
            limit,
            before,
            before_id,
        } => {
            cmd_get_workflow_runs(
                client,
                &workflow,
                limit,
                before.as_deref(),
                before_id.as_deref(),
            )
            .await
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
    const ID: &str = "cc3598aa-30bc-4c13-8616-6064e87598d2";

    #[test]
    fn run_page_preserves_rows_cursor_and_rejects_legacy_empty_array() {
        let page = serde_json::json!({"runs": [{"id": ID, "status": "completed"}],
            "next": {"before": "2026-09-11T12:06:00Z", "before_id": ID}});
        assert_eq!(parse_workflow_runs_page(&page.to_string()).unwrap(), page);
        assert!(parse_workflow_runs_page("[]").is_err());
        assert!(parse_workflow_runs_page("not json").is_err());
        assert!(parse_workflow_runs_page(r#"{"runs":[]}"#).is_err());
        assert!(parse_workflow_runs_page(r#"{"runs":[],"next":null}"#).is_ok());
    }

    #[test]
    fn workflow_run_path_canonicalizes_uuid_before_signing() {
        let canonical = workflow_runs_path(ID, None, None, None).unwrap();
        assert_eq!(
            workflow_runs_path(&ID.to_uppercase(), None, None, None).unwrap(),
            canonical
        );
        assert_eq!(
            workflow_runs_path(&ID.replace('-', ""), None, None, None).unwrap(),
            canonical
        );
    }

    #[test]
    fn run_cursor_is_paired_validated_and_encoded() {
        assert!(workflow_runs_path(ID, Some(0), None, None).is_err());
        assert!(workflow_runs_path(ID, None, Some("bad"), None).is_err());
        assert!(workflow_runs_path(ID, None, Some("bad"), Some(ID)).is_err());
        let path =
            workflow_runs_path(ID, Some(200), Some("2026-09-11T08:06:00+00:00"), Some(ID)).unwrap();
        assert!(path.contains("limit=100"));
        assert!(path.contains("%2B00%3A00"));
        assert!(path.contains("before_id="));
    }

    #[tokio::test]
    async fn run_history_uses_authenticated_get_and_propagates_unavailable() {
        use axum::{
            extract::OriginalUri,
            http::{HeaderMap, StatusCode},
            routing::get,
            Router,
        };
        use base64::Engine;
        use nostr::JsonUtil;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let expected_base = base.clone();
        let app = Router::new().route(
            "/workflows/{id}/runs",
            get(move |headers: HeaderMap, uri: OriginalUri| {
                let expected_base = expected_base.clone();
                async move {
                    assert_eq!(headers.get("x-auth-tag").unwrap(), "fixture-delegation");
                    let encoded = headers
                        .get("authorization")
                        .unwrap()
                        .to_str()
                        .unwrap()
                        .strip_prefix("Nostr ")
                        .unwrap();
                    let bytes = base64::engine::general_purpose::STANDARD
                        .decode(encoded)
                        .unwrap();
                    let event = nostr::Event::from_json(bytes).unwrap();
                    event.verify().unwrap();
                    assert!(event
                        .tags
                        .iter()
                        .any(|tag| tag.as_slice() == ["method", "GET"]));
                    let expected_url = format!("{expected_base}{}", uri.0);
                    assert!(event
                        .tags
                        .iter()
                        .any(|tag| tag.as_slice() == ["u", expected_url.as_str()]));
                    (StatusCode::NOT_FOUND, "run-history endpoint unavailable")
                }
            }),
        );
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let client = BuzzClient::new(
            base,
            nostr::Keys::generate(),
            None,
            Some("fixture-delegation".into()),
        )
        .unwrap();
        let result = cmd_get_workflow_runs(&client, ID, None, None, None).await;
        assert!(matches!(result, Err(CliError::Relay { status: 404, .. })));
        server.abort();
    }
}
