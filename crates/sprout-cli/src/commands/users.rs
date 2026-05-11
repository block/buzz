use nostr::{EventBuilder, Kind, Tag};

use crate::client::SproutClient;
use crate::error::CliError;
use crate::validate::validate_hex64;

/// Get user profiles (kind:0 metadata events).
/// 0 pubkeys → query our own profile
/// 1 pubkey → query that user's profile
/// 2+ pubkeys → query batch
pub async fn cmd_get_users(client: &SproutClient, pubkeys: &[String]) -> Result<(), CliError> {
    for pk in pubkeys {
        validate_hex64(pk)?;
    }
    if pubkeys.len() > 200 {
        return Err(CliError::Usage("--pubkey: maximum 200 pubkeys".into()));
    }

    let my_pk = client.keys().public_key().to_hex();
    let authors: Vec<&str> = if pubkeys.is_empty() {
        vec![my_pk.as_str()]
    } else {
        pubkeys.iter().map(|s| s.as_str()).collect()
    };

    let filter = serde_json::json!({
        "kinds": [0],
        "authors": authors,
        "limit": authors.len()
    });
    let resp = client.query(&filter).await?;
    println!("{resp}");
    Ok(())
}

pub async fn cmd_set_profile(
    client: &SproutClient,
    display_name: Option<&str>,
    avatar_url: Option<&str>,
    about: Option<&str>,
    nip05_handle: Option<&str>,
) -> Result<(), CliError> {
    if display_name.is_none() && avatar_url.is_none() && about.is_none() && nip05_handle.is_none() {
        return Err(CliError::Usage(
            "at least one field required (--name, --avatar, --about, --nip05)".into(),
        ));
    }

    // Read-merge-write: fetch current profile, merge in the new fields, then sign.
    let current = fetch_current_profile(client).await?;

    // Merge: caller-supplied fields win; fall back to current profile values.
    let merged_name = display_name
        .map(|s| s.to_string())
        .or_else(|| {
            current
                .get("display_name")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
        })
        .or_else(|| {
            current
                .get("name")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
        });
    let merged_picture = avatar_url.map(|s| s.to_string()).or_else(|| {
        current
            .get("picture")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
    });
    let merged_about = about.map(|s| s.to_string()).or_else(|| {
        current
            .get("about")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
    });
    let merged_nip05 = nip05_handle.map(|s| s.to_string()).or_else(|| {
        current
            .get("nip05")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
    });

    let builder = sprout_sdk::build_profile(
        merged_name.as_deref(),
        None, // `name` field (username) — not exposed by CLI
        merged_picture.as_deref(),
        merged_about.as_deref(),
        merged_nip05.as_deref(),
    )
    .map_err(|e| CliError::Other(format!("build_profile failed: {e}")))?;

    let event = client.sign_event(builder)?;

    let resp = client.submit_event(event).await?;
    println!("{resp}");
    Ok(())
}

/// Fetch the current user's profile metadata via POST /query (kind:0).
/// Returns the parsed content JSON object, or an empty object if no profile exists.
async fn fetch_current_profile(
    client: &SproutClient,
) -> Result<serde_json::Map<String, serde_json::Value>, CliError> {
    let my_pk = client.keys().public_key().to_hex();
    let filter = serde_json::json!({
        "kinds": [0],
        "authors": [my_pk],
        "limit": 1
    });
    let raw = client.query(&filter).await?;
    let events: serde_json::Value = serde_json::from_str(&raw)
        .map_err(|e| CliError::Other(format!("failed to parse profile query: {e}")))?;

    let Some(arr) = events.as_array() else {
        return Ok(serde_json::Map::new());
    };
    let Some(event) = arr.first() else {
        return Ok(serde_json::Map::new());
    };
    // kind:0 content is a JSON string containing the profile fields
    let content_str = event
        .get("content")
        .and_then(|c| c.as_str())
        .unwrap_or("{}");
    let content: serde_json::Value = serde_json::from_str(content_str).unwrap_or_default();
    Ok(content.as_object().cloned().unwrap_or_default())
}

/// Get presence status for users — query kind:40902 presence snapshot events.
pub async fn cmd_get_presence(client: &SproutClient, pubkeys_csv: &str) -> Result<(), CliError> {
    let pubkeys: Vec<&str> = pubkeys_csv
        .split(',')
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .collect();
    for pk in &pubkeys {
        validate_hex64(pk)?;
    }

    let filter = serde_json::json!({
        "kinds": [40902],
        "authors": pubkeys,
        "limit": pubkeys.len()
    });
    let resp = client.query(&filter).await?;
    println!("{resp}");
    Ok(())
}

/// Set presence status — sign and submit a kind:20001 presence update event.
///
/// NOTE: Kind 20001 is ephemeral and only accepted via WebSocket connections.
/// The CLI uses the HTTP bridge (POST /events) which rejects ephemeral kinds.
/// This will fail until the CLI gains a WS publish path. The kind is correct
/// per the protocol spec (KIND_PRESENCE_UPDATE = 20001).
pub async fn cmd_set_presence(client: &SproutClient, status: &str) -> Result<(), CliError> {
    match status {
        "online" | "away" | "offline" => {}
        _ => {
            return Err(CliError::Usage(format!(
                "--status must be one of: online, away, offline (got: {status})"
            )))
        }
    }

    let tags =
        vec![Tag::parse(&["status", status])
            .map_err(|e| CliError::Other(format!("tag error: {e}")))?];

    // KIND_PRESENCE_UPDATE (20001) — ephemeral, WS-only. HTTP bridge will reject this
    // until the CLI gains a WebSocket publish path.
    let builder = EventBuilder::new(Kind::Custom(20001), "", tags);
    let event = client.sign_event(builder)?;

    let resp = client.submit_event(event).await?;
    println!("{resp}");
    Ok(())
}
