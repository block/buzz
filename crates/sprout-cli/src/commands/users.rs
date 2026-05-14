use crate::client::SproutClient;
use crate::error::CliError;
use crate::validate::validate_hex64;

/// Get user profiles (kind:0 metadata events).
///
/// - 0 pubkeys, no name → query our own profile
/// - 1+ pubkeys → query those users' profiles
/// - --name "foo" → NIP-50 search on kind:0, then client-side filter
pub async fn cmd_get_users(
    client: &SproutClient,
    pubkeys: &[String],
    name: Option<&str>,
) -> Result<(), CliError> {
    if let Some(query) = name {
        if !pubkeys.is_empty() {
            return Err(CliError::Usage(
                "--name and --pubkey are mutually exclusive".into(),
            ));
        }
        return search_by_name(client, query).await;
    }

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

/// Search for users by display name via NIP-50 full-text search on kind:0 profiles.
/// Returns [] if the relay does not implement NIP-50 search.
async fn search_by_name(client: &SproutClient, query: &str) -> Result<(), CliError> {
    if query.trim().is_empty() {
        return Err(CliError::Usage("--name cannot be empty".into()));
    }

    let filter = serde_json::json!({
        "kinds": [0],
        "search": query,
        "limit": 100
    });
    let raw = client.query(&filter).await?;

    // Parse and filter client-side for case-insensitive substring match
    // on display_name or name fields (NIP-50 may return broader matches).
    let events: serde_json::Value = serde_json::from_str(&raw)
        .map_err(|e| CliError::Other(format!("failed to parse response: {e}")))?;

    let Some(arr) = events.as_array() else {
        println!("[]");
        return Ok(());
    };

    let lower_query = query.to_ascii_lowercase();
    let matches: Vec<&serde_json::Value> = arr
        .iter()
        .filter(|event| {
            let Some(content_str) = event.get("content").and_then(|v| v.as_str()) else {
                return false;
            };
            let Ok(content) = serde_json::from_str::<serde_json::Value>(content_str) else {
                return false;
            };
            let display_name = content
                .get("display_name")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let name = content.get("name").and_then(|v| v.as_str()).unwrap_or("");
            display_name.to_ascii_lowercase().contains(&lower_query)
                || name.to_ascii_lowercase().contains(&lower_query)
        })
        .collect();

    let output = serde_json::to_string(&matches).expect("serializing parsed JSON values");
    println!("{output}");
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
    let builder = sprout_sdk::build_presence_update(status).map_err(crate::validate::sdk_err)?;
    let event = client.sign_event(builder)?;

    let resp = client.submit_event(event).await?;
    println!("{resp}");
    Ok(())
}
