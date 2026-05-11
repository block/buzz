use nostr::{EventBuilder, Kind, Tag};
use sha2::{Digest, Sha256};

use crate::client::SproutClient;
use crate::error::CliError;
use crate::validate::{read_or_stdin, validate_uuid};

// ---------------------------------------------------------------------------
// Read commands — POST /query
// ---------------------------------------------------------------------------

/// List workflows in a channel — query kind:30620 workflow definition events.
pub async fn cmd_list_workflows(client: &SproutClient, channel_id: &str) -> Result<(), CliError> {
    validate_uuid(channel_id)?;
    let filter = serde_json::json!({
        "kinds": [30620],
        "#h": [channel_id]
    });
    let resp = client.query(&filter).await?;
    println!("{resp}");
    Ok(())
}

/// Get a single workflow definition.
pub async fn cmd_get_workflow(client: &SproutClient, workflow_id: &str) -> Result<(), CliError> {
    validate_uuid(workflow_id)?;
    let filter = serde_json::json!({
        "kinds": [30620],
        "#d": [workflow_id]
    });
    let resp = client.query(&filter).await?;
    println!("{resp}");
    Ok(())
}

/// Get workflow run history — query kind:46020 trigger events for this workflow.
pub async fn cmd_get_workflow_runs(
    client: &SproutClient,
    workflow_id: &str,
    limit: Option<u32>,
) -> Result<(), CliError> {
    validate_uuid(workflow_id)?;
    let limit = limit.unwrap_or(20).min(100);
    let filter = serde_json::json!({
        "kinds": [46020],
        "#d": [workflow_id],
        "limit": limit
    });
    let resp = client.query(&filter).await?;
    println!("{resp}");
    Ok(())
}

// ---------------------------------------------------------------------------
// Write commands — signed events via POST /events
// ---------------------------------------------------------------------------

/// Create a workflow — sign and submit a kind:30620 event.
pub async fn cmd_create_workflow(
    client: &SproutClient,
    channel_id: &str,
    yaml: &str,
) -> Result<(), CliError> {
    validate_uuid(channel_id)?;
    let yaml_definition = read_or_stdin(yaml)?;

    // Generate a unique d-tag for this workflow
    let workflow_id = uuid::Uuid::new_v4().to_string();
    let tags = vec![
        Tag::parse(&["d", &workflow_id]).map_err(|e| CliError::Other(format!("tag error: {e}")))?,
        Tag::parse(&["h", channel_id]).map_err(|e| CliError::Other(format!("tag error: {e}")))?,
    ];

    let builder = EventBuilder::new(Kind::Custom(30620), &yaml_definition, tags);
    let event = client.sign_event(builder)?;

    let resp = client.submit_event(event).await?;
    println!("{resp}");
    Ok(())
}

/// Update a workflow — sign and submit an updated kind:30620 event with same d-tag.
pub async fn cmd_update_workflow(
    client: &SproutClient,
    workflow_id: &str,
    yaml: &str,
) -> Result<(), CliError> {
    validate_uuid(workflow_id)?;
    let yaml_definition = read_or_stdin(yaml)?;

    let tags =
        vec![Tag::parse(&["d", workflow_id])
            .map_err(|e| CliError::Other(format!("tag error: {e}")))?];

    let builder = EventBuilder::new(Kind::Custom(30620), &yaml_definition, tags);
    let event = client.sign_event(builder)?;

    let resp = client.submit_event(event).await?;
    println!("{resp}");
    Ok(())
}

/// Delete a workflow — sign and submit a kind:5 deletion event.
pub async fn cmd_delete_workflow(client: &SproutClient, workflow_id: &str) -> Result<(), CliError> {
    validate_uuid(workflow_id)?;
    let keys = client.keys();

    // NIP-09 deletion targeting the parameterized replaceable event
    let tags = vec![Tag::parse(&[
        "a",
        &format!("30620:{}:{}", keys.public_key().to_hex(), workflow_id),
    ])
    .map_err(|e| CliError::Other(format!("tag error: {e}")))?];

    let builder = EventBuilder::new(Kind::Custom(5), "", tags);
    let event = client.sign_event(builder)?;

    let resp = client.submit_event(event).await?;
    println!("{resp}");
    Ok(())
}

/// Trigger a workflow — sign and submit a kind:46020 event.
pub async fn cmd_trigger_workflow(
    client: &SproutClient,
    workflow_id: &str,
) -> Result<(), CliError> {
    validate_uuid(workflow_id)?;

    let tags =
        vec![Tag::parse(&["d", workflow_id])
            .map_err(|e| CliError::Other(format!("tag error: {e}")))?];

    let builder = EventBuilder::new(Kind::Custom(46020), "", tags);
    let event = client.sign_event(builder)?;

    let resp = client.submit_event(event).await?;
    println!("{resp}");
    Ok(())
}

/// Approve or deny a workflow step — sign and submit a kind:46030 (grant) or 46031 (deny) event.
pub async fn cmd_approve_step(
    client: &SproutClient,
    approval_token: &str,
    approved: bool,
    note: Option<&str>,
) -> Result<(), CliError> {
    validate_uuid(approval_token)?;

    let kind = if approved { 46030 } else { 46031 };
    let content = note.unwrap_or("");

    // The relay expects d-tag = hex(SHA256(token)), not the raw token UUID.
    let token_hash = hex::encode(Sha256::digest(approval_token.as_bytes()));
    let tags =
        vec![Tag::parse(&["d", &token_hash])
            .map_err(|e| CliError::Other(format!("tag error: {e}")))?];

    let builder = EventBuilder::new(Kind::Custom(kind), content, tags);
    let event = client.sign_event(builder)?;

    let resp = client.submit_event(event).await?;
    println!("{resp}");
    Ok(())
}
