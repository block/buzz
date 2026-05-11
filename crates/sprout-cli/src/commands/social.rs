use nostr::EventId;
use serde::Deserialize;

use crate::client::SproutClient;
use crate::error::CliError;
use crate::validate::validate_hex64;

/// Per-module helper.
fn parse_event_id(hex: &str) -> Result<EventId, CliError> {
    EventId::parse(hex).map_err(|e| CliError::Usage(format!("invalid event ID: {e}")))
}

/// A single contact entry (CLI-local, not from sprout-sdk).
#[derive(Debug, Deserialize)]
pub struct ContactEntry {
    pub pubkey: String,
    #[serde(default)]
    pub relay_url: Option<String>,
    #[serde(default)]
    pub petname: Option<String>,
}

pub async fn cmd_publish_note(
    client: &SproutClient,
    content: &str,
    reply_to: Option<&str>,
) -> Result<(), CliError> {
    if let Some(r) = reply_to {
        validate_hex64(r)?;
    }

    let reply_id = reply_to.map(parse_event_id).transpose()?;

    let builder = sprout_sdk::build_note(content, reply_id)
        .map_err(|e| CliError::Other(format!("build error: {e}")))?;

    let event = client.sign_event(builder)?;

    let resp = client.submit_event(event).await?;
    println!("{resp}");
    Ok(())
}

pub async fn cmd_set_contact_list(
    client: &SproutClient,
    contacts_json: &str,
) -> Result<(), CliError> {
    let entries: Vec<ContactEntry> = serde_json::from_str(contacts_json)
        .map_err(|e| CliError::Usage(format!("invalid contacts JSON: {e}")))?;

    let contacts: Vec<(&str, Option<&str>, Option<&str>)> = entries
        .iter()
        .map(|c| {
            (
                c.pubkey.as_str(),
                c.relay_url.as_deref(),
                c.petname.as_deref(),
            )
        })
        .collect();

    let builder = sprout_sdk::build_contact_list(&contacts)
        .map_err(|e| CliError::Other(format!("build error: {e}")))?;

    let event = client.sign_event(builder)?;

    let resp = client.submit_event(event).await?;
    println!("{resp}");
    Ok(())
}

/// Get a single event by ID via POST /query.
pub async fn cmd_get_event(client: &SproutClient, event_id: &str) -> Result<(), CliError> {
    validate_hex64(event_id)?;
    let filter = serde_json::json!({
        "ids": [event_id]
    });
    let resp = client.query(&filter).await?;
    println!("{resp}");
    Ok(())
}

/// Get user notes (kind:1) by author pubkey.
pub async fn cmd_get_user_notes(
    client: &SproutClient,
    pubkey: &str,
    limit: Option<u32>,
    before: Option<i64>,
) -> Result<(), CliError> {
    validate_hex64(pubkey)?;
    let limit = limit.unwrap_or(50).min(100);

    let mut filter = serde_json::json!({
        "kinds": [1],
        "authors": [pubkey],
        "limit": limit
    });

    if let Some(b) = before {
        filter["until"] = serde_json::json!(b);
    }

    let resp = client.query(&filter).await?;
    println!("{resp}");
    Ok(())
}

/// Get a user's contact list (kind:3) by pubkey.
pub async fn cmd_get_contact_list(client: &SproutClient, pubkey: &str) -> Result<(), CliError> {
    validate_hex64(pubkey)?;
    let filter = serde_json::json!({
        "kinds": [3],
        "authors": [pubkey],
        "limit": 1
    });
    let resp = client.query(&filter).await?;
    println!("{resp}");
    Ok(())
}
