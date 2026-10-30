use buzz_core::kind::KIND_MANAGED_AGENT;
use nostr::PublicKey;

use crate::client::{extract_d_tag, normalize_write_response, BuzzClient};
use crate::error::CliError;
use crate::validate::validate_hex64;

// TODO(phase-4): Replace raw nostr::EventBuilder usage in cmd_set_presence with buzz-sdk builder

/// Get user profiles (kind:0 metadata events).
///
/// - 0 pubkeys, no name → query our own profile
/// - 1+ pubkeys → query those users' profiles
/// - --name "foo" → NIP-50 search on kind:0, then client-side filter
pub async fn cmd_get_users(
    client: &BuzzClient,
    pubkeys: &[String],
    name: Option<&str>,
    owner: Option<&str>,
    format: &crate::OutputFormat,
) -> Result<(), CliError> {
    if let Some(query) = name {
        if !pubkeys.is_empty() {
            return Err(CliError::Usage(
                "--name and --pubkey are mutually exclusive".into(),
            ));
        }
        return search_by_name(client, query, owner, format).await;
    }

    if owner.is_some() {
        return Err(CliError::Usage("--owner requires --name".into()));
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
    let events: Vec<serde_json::Value> = serde_json::from_str(&resp).unwrap_or_default();
    let profiles: Vec<serde_json::Value> = events
        .iter()
        .filter_map(|e| {
            let content_str = e.get("content")?.as_str()?;
            let mut profile: serde_json::Value = serde_json::from_str(content_str).ok()?;
            if let Some(obj) = profile.as_object_mut() {
                obj.insert(
                    "pubkey".to_string(),
                    serde_json::json!(e.get("pubkey").and_then(|v| v.as_str()).unwrap_or("")),
                );
            }
            Some(profile)
        })
        .collect();
    let output = match format {
        crate::OutputFormat::Compact => {
            let compact: Vec<serde_json::Value> = profiles
                .iter()
                .map(|p| serde_json::json!({
                    "pubkey": p.get("pubkey").cloned().unwrap_or_default(),
                    "display_name": p.get("display_name").or_else(|| p.get("name")).cloned().unwrap_or_default(),
                }))
                .collect();
            serde_json::to_string(&compact).unwrap_or_default()
        }
        crate::OutputFormat::Json | crate::OutputFormat::Agent => {
            serde_json::to_string(&profiles).unwrap_or_default()
        }
    };
    println!("{output}");
    Ok(())
}

fn effective_owner(client: &BuzzClient) -> String {
    client
        .auth_tag_owner_hex()
        .unwrap_or_else(|| client.keys().public_key().to_hex())
}

fn resolve_owner(client: &BuzzClient, owner: Option<&str>) -> Result<Option<String>, CliError> {
    owner
        .map(|owner| {
            if owner == "me" {
                Ok(effective_owner(client))
            } else {
                PublicKey::parse(owner)
                    .map(|pubkey| pubkey.to_hex())
                    .map_err(|e| {
                        CliError::Usage(format!("--owner must be `me`, a pubkey, or npub: {e}"))
                    })
            }
        })
        .transpose()
}

fn owned_agent_pubkeys_from_events(events: &[serde_json::Value], query: &str) -> Vec<String> {
    let mut pubkeys: Vec<String> = events
        .iter()
        .filter_map(|event| {
            let content: serde_json::Value =
                serde_json::from_str(event.get("content")?.as_str()?).ok()?;
            let name = content.get("name")?.as_str()?;
            if !name.eq_ignore_ascii_case(query) {
                return None;
            }
            let pubkey = extract_d_tag(event);
            (!pubkey.is_empty()).then_some(pubkey)
        })
        .collect();
    pubkeys.sort();
    pubkeys.dedup();
    pubkeys
}

async fn owned_agent_pubkeys_by_name(
    client: &BuzzClient,
    owner: &str,
    query: &str,
) -> Result<Vec<String>, CliError> {
    let filter = serde_json::json!({
        "kinds": [KIND_MANAGED_AGENT],
        "authors": [owner],
    });
    let events = client.query_all(filter).await?;
    Ok(owned_agent_pubkeys_from_events(&events, query))
}

fn profile_content(event: &serde_json::Value) -> serde_json::Map<String, serde_json::Value> {
    event
        .get("content")
        .and_then(|value| value.as_str())
        .and_then(|content| serde_json::from_str::<serde_json::Value>(content).ok())
        .and_then(|content| content.as_object().cloned())
        .unwrap_or_default()
}

fn name_search_profiles(events: &[serde_json::Value], query: &str) -> Vec<serde_json::Value> {
    let lower_query = query.to_ascii_lowercase();
    events
        .iter()
        .filter_map(|event| {
            let mut profile = profile_content(event);
            let display_name = profile
                .get("display_name")
                .and_then(|value| value.as_str())
                .unwrap_or("");
            let name = profile
                .get("name")
                .and_then(|value| value.as_str())
                .unwrap_or("");
            if !display_name.to_ascii_lowercase().contains(&lower_query)
                && !name.to_ascii_lowercase().contains(&lower_query)
            {
                return None;
            }
            profile.insert(
                "pubkey".to_string(),
                serde_json::json!(event
                    .get("pubkey")
                    .and_then(|value| value.as_str())
                    .unwrap_or("")),
            );
            Some(serde_json::Value::Object(profile))
        })
        .collect()
}

fn auth_tag_values(event: &serde_json::Value) -> Vec<&serde_json::Value> {
    event
        .get("tags")
        .and_then(|tags| tags.as_array())
        .into_iter()
        .flatten()
        .filter(|tag| {
            tag.as_array()
                .and_then(|values| values.first())
                .and_then(|value| value.as_str())
                == Some("auth")
        })
        .collect()
}

fn auth_conditions_apply(auth_tag: &serde_json::Value, event: &serde_json::Value) -> bool {
    let Some(conditions) = auth_tag
        .as_array()
        .and_then(|values| values.get(2))
        .and_then(|value| value.as_str())
    else {
        return false;
    };
    let Some(kind) = event.get("kind").and_then(|value| value.as_u64()) else {
        return false;
    };
    let Some(created_at) = event.get("created_at").and_then(|value| value.as_u64()) else {
        return false;
    };

    conditions.split('&').all(|clause| {
        if let Some(value) = clause.strip_prefix("kind=") {
            value.parse::<u64>() == Ok(kind)
        } else if let Some(value) = clause.strip_prefix("created_at<") {
            value.parse::<u64>().is_ok_and(|bound| created_at < bound)
        } else if let Some(value) = clause.strip_prefix("created_at>") {
            value.parse::<u64>().is_ok_and(|bound| created_at > bound)
        } else {
            clause.is_empty()
        }
    })
}

fn owner_verification(event: &serde_json::Value, expected_owner: &str) -> &'static str {
    let Some(agent_pubkey) = event
        .get("pubkey")
        .and_then(|value| value.as_str())
        .and_then(|value| PublicKey::parse(value).ok())
    else {
        return "invalid_agent_pubkey";
    };
    let auth_tags = auth_tag_values(event);
    let [auth_tag] = auth_tags.as_slice() else {
        return if auth_tags.is_empty() {
            "missing_auth"
        } else {
            "multiple_auth_tags"
        };
    };
    let Ok(auth_tag_json) = serde_json::to_string(auth_tag) else {
        return "invalid_auth";
    };
    match buzz_sdk::nip_oa::verify_auth_tag(&auth_tag_json, &agent_pubkey) {
        Ok(owner) if owner.to_hex() != expected_owner => "owner_mismatch",
        Ok(_) if !auth_conditions_apply(auth_tag, event) => "condition_mismatch",
        Ok(_) => "verified",
        Err(_) => "invalid_auth",
    }
}

fn owner_scoped_profiles(
    events: &[serde_json::Value],
    pubkeys: &[String],
    owner: &str,
    effective_owner: &str,
) -> Vec<serde_json::Value> {
    pubkeys
        .iter()
        .map(|pubkey| {
            let event = events.iter().find(|event| {
                event.get("pubkey").and_then(|value| value.as_str()) == Some(pubkey.as_str())
            });
            let mut profile = event.map(profile_content).unwrap_or_default();
            let verification = if PublicKey::parse(pubkey).is_err() {
                "invalid_agent_pubkey"
            } else {
                event
                    .map(|event| owner_verification(event, owner))
                    .unwrap_or("missing_profile")
            };
            profile.insert("pubkey".to_string(), serde_json::json!(pubkey));
            profile.insert("verification".to_string(), serde_json::json!(verification));
            profile.insert(
                "owned_by_me".to_string(),
                serde_json::json!(verification == "verified" && owner == effective_owner),
            );
            if verification == "verified" {
                profile.insert("owner_pubkey".to_string(), serde_json::json!(owner));
            }
            serde_json::Value::Object(profile)
        })
        .collect()
}

/// Search for users by display name. Owner-scoped searches resolve managed-agent records
/// and verify their profiles; unscoped searches use NIP-50 and return [] if unsupported.
async fn search_by_name(
    client: &BuzzClient,
    query: &str,
    owner: Option<&str>,
    format: &crate::OutputFormat,
) -> Result<(), CliError> {
    if query.trim().is_empty() {
        return Err(CliError::Usage("--name cannot be empty".into()));
    }

    let owner = resolve_owner(client, owner)?;
    let profiles = if let Some(owner) = owner {
        let pubkeys = owned_agent_pubkeys_by_name(client, &owner, query).await?;
        if pubkeys.is_empty() {
            println!("[]");
            return Ok(());
        }
        let valid_pubkeys: Vec<&String> = pubkeys
            .iter()
            .filter(|pubkey| PublicKey::parse(pubkey.as_str()).is_ok())
            .collect();
        let events = if valid_pubkeys.is_empty() {
            Vec::new()
        } else {
            let filter = serde_json::json!({
                "kinds": [0],
                "authors": valid_pubkeys,
                "limit": valid_pubkeys.len(),
            });
            let raw = client.query(&filter).await?;
            serde_json::from_str(&raw)
                .map_err(|e| CliError::Other(format!("failed to parse response: {e}")))?
        };
        owner_scoped_profiles(&events, &pubkeys, &owner, &effective_owner(client))
    } else {
        let filter = serde_json::json!({
            "kinds": [0],
            "search": query,
            "limit": 100
        });
        let raw = client.query(&filter).await?;
        let events: Vec<serde_json::Value> = serde_json::from_str(&raw)
            .map_err(|e| CliError::Other(format!("failed to parse response: {e}")))?;
        name_search_profiles(&events, query)
    };
    let output = match format {
        crate::OutputFormat::Compact => {
            let compact: Vec<serde_json::Value> = profiles
                .iter()
                .map(|p| {
                    let mut value = serde_json::json!({
                        "pubkey": p.get("pubkey").cloned().unwrap_or_default(),
                        "display_name": p.get("display_name").or_else(|| p.get("name")).cloned().unwrap_or_default(),
                    });
                    if let Some(obj) = value.as_object_mut() {
                        for field in ["owner_pubkey", "owned_by_me", "verification"] {
                            if let Some(field_value) = p.get(field) {
                                obj.insert(field.to_string(), field_value.clone());
                            }
                        }
                    }
                    value
                })
                .collect();
            serde_json::to_string(&compact).unwrap_or_default()
        }
        crate::OutputFormat::Json | crate::OutputFormat::Agent => {
            serde_json::to_string(&profiles).unwrap_or_default()
        }
    };
    println!("{output}");
    Ok(())
}

pub async fn cmd_set_profile(
    client: &BuzzClient,
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

    let builder = buzz_sdk::build_profile(
        merged_name.as_deref(),
        None, // `name` field (username) — not exposed by CLI
        merged_picture.as_deref(),
        merged_about.as_deref(),
        merged_nip05.as_deref(),
    )
    .map_err(|e| CliError::Other(format!("build_profile failed: {e}")))?;

    let event = client.sign_event(builder)?;

    let resp = client.submit_event(event).await?;
    println!("{}", normalize_write_response(&resp));
    Ok(())
}

/// Fetch the current user's profile metadata via POST /query (kind:0).
/// Returns the parsed content JSON object, or an empty object if no profile exists.
async fn fetch_current_profile(
    client: &BuzzClient,
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
pub async fn cmd_get_presence(client: &BuzzClient, pubkeys_csv: &str) -> Result<(), CliError> {
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
    let events: Vec<serde_json::Value> = serde_json::from_str(&resp).unwrap_or_default();
    let presence: Vec<serde_json::Value> = events
        .iter()
        .map(|e| {
            serde_json::json!({
                "pubkey": presence_subject(e),
                "status": e.get("content").and_then(|v| v.as_str()).unwrap_or(""),
                "updated_at": e.get("created_at").and_then(|v| v.as_u64()).unwrap_or(0),
            })
        })
        .collect();
    let output = serde_json::to_string(&presence).unwrap_or_default();
    println!("{output}");
    Ok(())
}

pub(crate) fn presence_subject(event: &serde_json::Value) -> &str {
    event
        .get("tags")
        .and_then(|tags| tags.as_array())
        .and_then(|tags| {
            tags.iter()
                .find_map(|tag| match tag.as_array()?.as_slice() {
                    [name, subject, ..] if name == "p" => subject.as_str(),
                    _ => None,
                })
        })
        .unwrap_or_else(|| event.get("pubkey").and_then(|v| v.as_str()).unwrap_or(""))
}

/// Set presence status — sign and submit a kind:20001 presence update event via WebSocket.
///
/// Kind 20001 is ephemeral and only accepted via WebSocket connections. This
/// method connects to the relay over WS, performs NIP-42 authentication, and
/// publishes the event directly — bypassing the HTTP bridge.
pub async fn cmd_set_presence(client: &BuzzClient, status: &str) -> Result<(), CliError> {
    let builder = buzz_sdk::build_presence_update(status).map_err(crate::validate::sdk_err)?;
    let event = client.sign_event(builder)?;

    let resp = client.publish_ephemeral_event(event).await?;
    println!("{}", normalize_write_response(&resp));
    Ok(())
}

/// Seconds in each `--expires` duration unit.
fn expiry_unit_secs(unit: char) -> Option<u64> {
    match unit {
        's' => Some(1),
        'm' => Some(60),
        'h' => Some(3_600),
        'd' => Some(86_400),
        'w' => Some(604_800),
        _ => None,
    }
}

/// Resolve `--expires` to an absolute unix time that is after `now`.
///
/// A bare integer is a unix timestamp; an integer with an `s`, `m`, `h`, `d`,
/// or `w` suffix is a duration from `now`.
fn parse_expiry(value: &str, now: u64) -> Result<u64, CliError> {
    let value = value.trim();
    let bad = || {
        CliError::Usage(format!(
            "--expires must be a unix timestamp or a duration like 30m, 2h, 1d (got: {value})"
        ))
    };
    let split = value
        .char_indices()
        .find(|(_, c)| !c.is_ascii_digit())
        .map_or(value.len(), |(i, _)| i);
    let (digits, unit) = value.split_at(split);
    let number: u64 = digits.parse().map_err(|_| bad())?;
    let expires_at = match unit {
        "" => number,
        _ => {
            let mut chars = unit.chars();
            let secs = chars.next().and_then(expiry_unit_secs).ok_or_else(bad)?;
            if chars.next().is_some() {
                return Err(bad());
            }
            number
                .checked_mul(secs)
                .and_then(|delta| now.checked_add(delta))
                .ok_or_else(bad)?
        }
    };
    if expires_at <= now {
        return Err(CliError::Usage(format!(
            "--expires must be in the future (got {expires_at}, now is {now}); a bare number is a unix timestamp, so add a unit such as 1h for a duration"
        )));
    }
    Ok(expires_at)
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// Sign and submit a NIP-38 kind:30315 user status event.
///
/// Uses the `d:general` coordinate that the desktop client reads for the
/// profile status line. A blank `text` with no `emoji` clears the status.
/// `expires` is validated before anything is signed.
async fn publish_status(
    client: &BuzzClient,
    text: &str,
    emoji: Option<&str>,
    expires: Option<&str>,
    now: u64,
) -> Result<String, CliError> {
    let expires_at = expires.map(|value| parse_expiry(value, now)).transpose()?;
    let builder = buzz_sdk::build_user_status_expiring(text, emoji, expires_at)
        .map_err(crate::validate::sdk_err)?;
    let event = client.sign_event(builder)?;
    let resp = client.submit_event(event).await?;
    Ok(normalize_write_response(&resp))
}

/// Set user status and print the write response.
pub async fn cmd_set_status(
    client: &BuzzClient,
    text: &str,
    emoji: Option<&str>,
    expires: Option<&str>,
) -> Result<(), CliError> {
    println!(
        "{}",
        publish_status(client, text, emoji, expires, now_secs()).await?
    );
    Ok(())
}

/// Most pubkeys one `users status` call reads; each name costs a profile search.
const MAX_STATUS_PUBKEYS: usize = 100;

/// Desktop's `parseUserStatusEvent` reads `expiration` with `parseInt`: an
/// optional sign and leading digits, ignoring any trailing text.
fn parse_expiration_tag(value: &str) -> Option<i64> {
    let value = value.trim_start();
    let unsigned = value.strip_prefix(['+', '-']).unwrap_or(value);
    let digits = unsigned
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(unsigned.len());
    let end = value.len() - unsigned.len() + digits;
    value[..end].parse().ok()
}

fn tag_value<'a>(event: &'a serde_json::Value, name: &str) -> Option<&'a str> {
    event
        .get("tags")?
        .as_array()?
        .iter()
        .filter_map(|tag| tag.as_array())
        .find(|tag| tag.len() >= 2 && tag[0].as_str() == Some(name))
        .and_then(|tag| tag[1].as_str())
}

/// The visible status per requested author, in request order. The newest
/// event per author wins, as on desktop (higher `created_at`, then lower id);
/// a newest event that is empty or expired hides the author, and an older
/// status never resurfaces behind it.
fn visible_statuses(
    events: &[serde_json::Value],
    authors: &[String],
    now: u64,
) -> Vec<serde_json::Value> {
    fn key(event: &serde_json::Value) -> (u64, &str) {
        (
            event
                .get("created_at")
                .and_then(|v| v.as_u64())
                .unwrap_or(0),
            event.get("id").and_then(|v| v.as_str()).unwrap_or(""),
        )
    }
    authors
        .iter()
        .filter_map(|author| {
            let event = events
                .iter()
                .filter(|e| {
                    e.get("pubkey").and_then(|v| v.as_str()) == Some(author.as_str())
                        && tag_value(e, "d") == Some("general")
                })
                .min_by(|a, b| {
                    let (a_at, a_id) = key(a);
                    let (b_at, b_id) = key(b);
                    b_at.cmp(&a_at).then_with(|| a_id.cmp(b_id))
                })?;
            let text = event.get("content").and_then(|v| v.as_str()).unwrap_or("");
            let emoji = tag_value(event, "emoji").unwrap_or("");
            let expires_at = tag_value(event, "expiration").and_then(parse_expiration_tag);
            let expired = expires_at.is_some_and(|at| at <= now as i64);
            if (text.is_empty() && emoji.is_empty()) || expired {
                return None;
            }
            Some(serde_json::json!({
                "pubkey": author,
                "text": text,
                "emoji": emoji,
                "expires_at": expires_at,
                "updated_at": key(event).0,
            }))
        })
        .collect()
}

/// Read the visible `d:general` statuses for `pubkeys` (default: you).
async fn read_statuses(
    client: &BuzzClient,
    pubkeys: &[String],
    now: u64,
) -> Result<Vec<serde_json::Value>, CliError> {
    if pubkeys.len() > MAX_STATUS_PUBKEYS {
        return Err(CliError::Usage(format!(
            "--pubkey: maximum {MAX_STATUS_PUBKEYS} pubkeys"
        )));
    }
    let mut authors: Vec<String> = Vec::new();
    for pubkey in pubkeys {
        let resolved = crate::commands::messages::resolve_author(client, pubkey).await?;
        if !authors.contains(&resolved) {
            authors.push(resolved);
        }
    }
    if authors.is_empty() {
        authors.push(client.keys().public_key().to_hex());
    }
    let filter = serde_json::json!({
        "kinds": [buzz_core::kind::KIND_USER_STATUS],
        "authors": authors,
        "#d": ["general"],
        "limit": authors.len(),
    });
    let raw = client.query(&filter).await?;
    let events: Vec<serde_json::Value> = serde_json::from_str(&raw)
        .map_err(|e| CliError::Other(format!("failed to parse status query: {e}")))?;
    Ok(visible_statuses(&events, &authors, now))
}

/// Print the visible statuses for `pubkeys` as a JSON array.
pub async fn cmd_get_status(client: &BuzzClient, pubkeys: &[String]) -> Result<(), CliError> {
    let statuses = read_statuses(client, pubkeys, now_secs()).await?;
    println!("{}", serde_json::Value::Array(statuses));
    Ok(())
}

pub async fn dispatch(
    cmd: crate::UsersCmd,
    client: &BuzzClient,
    format: &crate::OutputFormat,
) -> Result<(), CliError> {
    use crate::UsersCmd;
    match cmd {
        UsersCmd::Get {
            pubkeys,
            name,
            owner,
        } => cmd_get_users(client, &pubkeys, name.as_deref(), owner.as_deref(), format).await,
        UsersCmd::SetProfile {
            name,
            avatar,
            about,
            nip05,
        } => {
            cmd_set_profile(
                client,
                name.as_deref(),
                avatar.as_deref(),
                about.as_deref(),
                nip05.as_deref(),
            )
            .await
        }
        UsersCmd::Presence { pubkeys } => cmd_get_presence(client, &pubkeys).await,
        UsersCmd::SetPresence { status } => cmd_set_presence(client, &status.to_string()).await,
        UsersCmd::SetStatus {
            text,
            emoji,
            clear,
            expires,
        } => {
            // `--clear` is mutually exclusive with `--text`/`--emoji`: publish the
            // empty `d:general` event that clients read as "no status".
            let (text, emoji) = if clear {
                ("", None)
            } else {
                (text.as_deref().unwrap_or_default(), emoji.as_deref())
            };
            cmd_set_status(client, text, emoji, expires.as_deref()).await
        }
        UsersCmd::Status { pubkeys } => cmd_get_status(client, &pubkeys).await,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        owned_agent_pubkeys_from_events, owner_scoped_profiles, owner_verification,
        parse_expiration_tag, parse_expiry, presence_subject, publish_status, read_statuses,
    };
    use crate::client::BuzzClient;
    use crate::error::CliError;
    use axum::{body::Bytes, extract::State, routing::post, Json, Router};
    use nostr::{EventBuilder, Keys, Kind, Tag, Timestamp};
    use serde_json::{json, Value};
    use std::sync::{Arc, Mutex};
    use tokio::net::TcpListener;

    const NOW: u64 = 1_800_000_000;

    #[derive(Default)]
    struct Relay {
        events: Vec<Value>,
        queries: Vec<Value>,
        raw_query_response: Option<String>,
    }
    type Shared = Arc<Mutex<Relay>>;

    /// Fake relay: `/query` records the filter and returns stored events of the
    /// requested kind and authors (kind 0 also honors `search`); `/events`
    /// stores one.
    async fn fake_relay(relay: Shared) -> String {
        let app = Router::new()
            .route(
                "/query",
                post(|State(relay): State<Shared>, body: Bytes| async move {
                    let filters: Vec<Value> = serde_json::from_slice(&body).unwrap();
                    let [filter] = filters.as_slice() else {
                        panic!("one filter expected: {filters:?}")
                    };
                    let mut relay = relay.lock().unwrap();
                    relay.queries.push(filter.clone());
                    if let Some(raw) = relay.raw_query_response.clone() {
                        return raw;
                    }
                    let kind = &filter["kinds"][0];
                    let matching: Vec<Value> = relay
                        .events
                        .iter()
                        .filter(|e| &e["kind"] == kind)
                        .filter(|e| match filter.get("authors") {
                            Some(authors) => authors.as_array().unwrap().contains(&e["pubkey"]),
                            None => true,
                        })
                        .filter(|e| match filter.get("search").and_then(|s| s.as_str()) {
                            Some(q) => e["content"]
                                .as_str()
                                .unwrap()
                                .to_lowercase()
                                .contains(&q.to_lowercase()),
                            None => true,
                        })
                        .cloned()
                        .collect();
                    serde_json::to_string(&matching).unwrap()
                }),
            )
            .route(
                "/events",
                post(|State(relay): State<Shared>, body: Bytes| async move {
                    let event: Value = serde_json::from_slice(&body).unwrap();
                    relay.lock().unwrap().events.push(event.clone());
                    Json(json!({"event_id": event["id"], "accepted": true, "message": ""}))
                }),
            )
            .with_state(relay);
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        format!("http://{addr}")
    }

    async fn setup() -> (BuzzClient, Shared) {
        let relay = Shared::default();
        let url = fake_relay(relay.clone()).await;
        let client = BuzzClient::new(url, Keys::generate(), None, None).unwrap();
        (client, relay)
    }

    fn seed(relay: &Shared, keys: &Keys, kind: u16, content: &str, tags: Vec<Vec<&str>>, at: u64) {
        let tags: Vec<Tag> = tags.into_iter().map(|t| Tag::parse(t).unwrap()).collect();
        let event = EventBuilder::new(Kind::Custom(kind), content)
            .tags(tags)
            .custom_created_at(Timestamp::from(at))
            .sign_with_keys(keys)
            .unwrap();
        relay
            .lock()
            .unwrap()
            .events
            .push(serde_json::to_value(event).unwrap());
    }

    fn seed_status(relay: &Shared, keys: &Keys, text: &str, extra: Vec<Vec<&str>>, at: u64) {
        let mut tags = vec![vec!["d", "general"]];
        tags.extend(extra);
        seed(relay, keys, 30315, text, tags, at);
    }

    fn event_count(relay: &Shared) -> usize {
        relay.lock().unwrap().events.len()
    }

    fn last_tags(relay: &Shared) -> Vec<Vec<String>> {
        let event = relay.lock().unwrap().events.last().cloned().unwrap();
        serde_json::from_value(event["tags"].clone()).unwrap()
    }

    #[tokio::test]
    async fn set_status_publishes_one_expiring_event_and_rejects_bad_expiry_before_signing() {
        let (client, relay) = setup().await;
        let resp = publish_status(&client, "lunch", Some("🥪"), Some("30m"), NOW)
            .await
            .unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(&resp).unwrap()["accepted"],
            true
        );
        assert_eq!(event_count(&relay), 1);
        assert_eq!(
            last_tags(&relay),
            vec![
                vec!["d", "general"],
                vec!["emoji", "🥪"],
                vec!["expiration", &(NOW + 1_800).to_string()],
            ]
        );

        publish_status(&client, "later", None, Some(&(NOW + 5).to_string()), NOW)
            .await
            .unwrap();
        assert_eq!(
            last_tags(&relay).last().unwrap(),
            &vec!["expiration".to_string(), (NOW + 5).to_string()]
        );

        publish_status(&client, "no expiry", None, None, NOW)
            .await
            .unwrap();
        assert_eq!(last_tags(&relay), vec![vec!["d", "general"]]);

        let published = event_count(&relay);
        for expires in [
            &NOW.to_string(),
            "0",
            "1700000000",
            "0m",
            "soon",
            "",
            "1x",
            "1hh",
            "-5m",
            "99999999999999999999d",
            "999999999999999d",
        ] {
            let err = publish_status(&client, "x", None, Some(expires), NOW)
                .await
                .unwrap_err();
            assert!(matches!(err, CliError::Usage(_)), "{expires}: {err:?}");
        }
        let err = publish_status(&client, "", None, Some("1h"), NOW)
            .await
            .unwrap_err();
        assert!(matches!(err, CliError::Usage(_)), "{err:?}");
        assert_eq!(event_count(&relay), published);
    }

    #[test]
    fn expiry_parses_timestamps_and_durations() {
        for (input, expected) in [
            ("30s", NOW + 30),
            ("30m", NOW + 1_800),
            ("2h", NOW + 7_200),
            ("1d", NOW + 86_400),
            (" 1w ", NOW + 604_800),
            ("1800000001", 1_800_000_001),
        ] {
            assert_eq!(parse_expiry(input, NOW).unwrap(), expected, "{input}");
        }
    }

    #[test]
    fn expiration_tag_reads_like_desktop_parse_int() {
        for (input, expected) in [
            ("1800000060", Some(1_800_000_060)),
            (" 42", Some(42)),
            ("+42", Some(42)),
            ("-5", Some(-5)),
            ("90abc", Some(90)),
            ("abc", None),
            ("", None),
            ("-", None),
        ] {
            assert_eq!(parse_expiration_tag(input), expected, "{input:?}");
        }
    }

    #[tokio::test]
    async fn status_read_hides_expired_and_cleared_and_reports_the_rest() {
        let (client, relay) = setup().await;
        let me = client.keys().clone();
        let (expired, cleared, emoji_only, odd, timed) = (
            Keys::generate(),
            Keys::generate(),
            Keys::generate(),
            Keys::generate(),
            Keys::generate(),
        );
        let silent = Keys::generate();
        seed_status(&relay, &me, "writing docs", vec![vec!["emoji", "📝"]], 100);
        seed_status(
            &relay,
            &expired,
            "gone",
            vec![vec!["expiration", &NOW.to_string()]],
            100,
        );
        seed_status(&relay, &cleared, "old text", vec![], 100);
        seed_status(&relay, &cleared, "", vec![], 200);
        seed_status(&relay, &emoji_only, "", vec![vec!["emoji", "🎧"]], 100);
        seed_status(&relay, &odd, "odd", vec![vec!["expiration", "soon"]], 100);
        seed_status(
            &relay,
            &timed,
            "back soon",
            vec![vec!["expiration", &format!("{}xyz", NOW + 60)]],
            100,
        );
        // Wrong coordinate and wrong kind are not statuses.
        seed(
            &relay,
            &silent,
            30315,
            "music",
            vec![vec!["d", "music"]],
            100,
        );
        seed(&relay, &silent, 1, "note", vec![vec!["d", "general"]], 100);

        let pubkeys: Vec<String> = [&expired, &cleared, &emoji_only, &odd, &timed, &silent]
            .iter()
            .map(|k| k.public_key().to_hex())
            .collect();
        let statuses = read_statuses(&client, &pubkeys, NOW).await.unwrap();
        assert_eq!(
            statuses,
            vec![
                json!({"pubkey": pubkeys[2], "text": "", "emoji": "🎧", "expires_at": null, "updated_at": 100}),
                json!({"pubkey": pubkeys[3], "text": "odd", "emoji": "", "expires_at": null, "updated_at": 100}),
                json!({"pubkey": pubkeys[4], "text": "back soon", "emoji": "", "expires_at": NOW + 60, "updated_at": 100}),
            ]
        );

        let filter = relay.lock().unwrap().queries.last().cloned().unwrap();
        assert_eq!(filter["kinds"], json!([30315]));
        assert_eq!(filter["#d"], json!(["general"]));
        assert_eq!(filter["limit"], json!(6));

        // Default author is the caller.
        let own = read_statuses(&client, &[], NOW).await.unwrap();
        assert_eq!(
            own,
            vec![json!({
                "pubkey": me.public_key().to_hex(),
                "text": "writing docs",
                "emoji": "📝",
                "expires_at": null,
                "updated_at": 100,
            })]
        );
        // The same status is hidden once its expiry passes.
        seed_status(
            &relay,
            &me,
            "meeting",
            vec![vec!["expiration", &(NOW + 10).to_string()]],
            300,
        );
        let before = read_statuses(&client, &[], NOW).await.unwrap();
        assert_eq!(before[0]["text"], "meeting");
        assert_eq!(before[0]["expires_at"], json!(NOW + 10));
        assert!(read_statuses(&client, &[], NOW + 10)
            .await
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    async fn status_read_picks_the_newest_event_and_breaks_ties_by_lower_id() {
        let (client, relay) = setup().await;
        let keys = Keys::generate();
        seed_status(&relay, &keys, "older", vec![], 100);
        seed_status(&relay, &keys, "tie a", vec![], 200);
        seed_status(&relay, &keys, "tie b", vec![], 200);
        let events = relay.lock().unwrap().events.clone();
        let winner = events[1..]
            .iter()
            .min_by_key(|e| e["id"].as_str().unwrap().to_string())
            .unwrap()["content"]
            .clone();
        let statuses = read_statuses(&client, &[keys.public_key().to_hex()], NOW)
            .await
            .unwrap();
        assert_eq!(statuses.len(), 1);
        assert_eq!(statuses[0]["text"], winner);
        assert_eq!(statuses[0]["updated_at"], 200);
    }

    #[tokio::test]
    async fn status_read_resolves_npub_and_name_and_bounds_input() {
        let (client, relay) = setup().await;
        let by_npub = Keys::generate();
        let by_name = Keys::generate();
        seed_status(&relay, &by_npub, "via npub", vec![], 100);
        seed_status(&relay, &by_name, "via name", vec![], 100);
        seed(
            &relay,
            &by_name,
            0,
            r#"{"display_name":"Honey"}"#,
            vec![],
            50,
        );
        let npub = nostr::ToBech32::to_bech32(&by_npub.public_key()).unwrap();
        let statuses = read_statuses(
            &client,
            &[
                npub,
                "honey".to_string(),
                by_name.public_key().to_hex().to_uppercase(),
            ],
            NOW,
        )
        .await
        .unwrap();
        let texts: Vec<&str> = statuses
            .iter()
            .map(|s| s["text"].as_str().unwrap())
            .collect();
        assert_eq!(texts, vec!["via npub", "via name"]);

        let err = read_statuses(&client, &["nobody".to_string()], NOW)
            .await
            .unwrap_err();
        assert!(matches!(err, CliError::Usage(_)), "{err:?}");

        let queries = relay.lock().unwrap().queries.len();
        let too_many: Vec<String> = (0..101).map(|_| by_npub.public_key().to_hex()).collect();
        let err = read_statuses(&client, &too_many, NOW).await.unwrap_err();
        assert!(matches!(err, CliError::Usage(_)), "{err:?}");
        assert_eq!(relay.lock().unwrap().queries.len(), queries);
    }

    #[tokio::test]
    async fn status_read_fails_on_an_unreadable_relay_response() {
        let (client, relay) = setup().await;
        relay.lock().unwrap().raw_query_response = Some("not json".into());
        let err = read_statuses(&client, &[], NOW).await.unwrap_err();
        assert!(matches!(err, CliError::Other(_)), "{err:?}");
    }

    #[tokio::test]
    async fn a_published_status_reads_back_with_its_expiry() {
        let (client, _relay) = setup().await;
        publish_status(&client, "lunch", Some("🥪"), Some("1h"), NOW)
            .await
            .unwrap();
        let statuses = read_statuses(&client, &[], NOW + 60).await.unwrap();
        assert_eq!(statuses.len(), 1);
        assert_eq!(statuses[0]["text"], "lunch");
        assert_eq!(statuses[0]["emoji"], "🥪");
        assert_eq!(statuses[0]["expires_at"], json!(NOW + 3_600));
        assert!(read_statuses(&client, &[], NOW + 3_600)
            .await
            .unwrap()
            .is_empty());
    }

    #[test]
    fn owned_agent_lookup_matches_exact_name_case_insensitively() {
        let events = vec![
            json!({"content": r#"{"name":"Honey"}"#, "tags": [["d", "b"]]}),
            json!({"content": r#"{"name":"Honeybee"}"#, "tags": [["d", "c"]]}),
            json!({"content": r#"{"name":"honey"}"#, "tags": [["d", "a"]]}),
        ];
        assert_eq!(
            owned_agent_pubkeys_from_events(&events, "Honey"),
            vec!["a", "b"]
        );
    }

    #[test]
    fn owned_agent_lookup_ignores_malformed_events() {
        let events = vec![
            json!({"content": "not json", "tags": [["d", "a"]]}),
            json!({"content": r#"{"name":"Honey"}"#, "tags": [["p", "b"]]}),
        ];
        assert!(owned_agent_pubkeys_from_events(&events, "Honey").is_empty());
    }

    fn profile_event(agent_keys: &Keys, auth_tags: Vec<serde_json::Value>) -> serde_json::Value {
        json!({
            "pubkey": agent_keys.public_key().to_hex(),
            "kind": 0,
            "created_at": 100,
            "content": r#"{"display_name":"Renamed Honey"}"#,
            "tags": auth_tags,
        })
    }

    #[test]
    fn owner_verification_requires_one_valid_auth_tag_for_requested_owner() {
        let owner_keys = Keys::generate();
        let agent_keys = Keys::generate();
        let foreign_owner_keys = Keys::generate();
        let valid_tag: serde_json::Value = serde_json::from_str(
            &buzz_sdk::nip_oa::compute_auth_tag(&owner_keys, &agent_keys.public_key(), "kind=0")
                .unwrap(),
        )
        .unwrap();
        let foreign_tag: serde_json::Value = serde_json::from_str(
            &buzz_sdk::nip_oa::compute_auth_tag(
                &foreign_owner_keys,
                &agent_keys.public_key(),
                "kind=9",
            )
            .unwrap(),
        )
        .unwrap();

        assert_eq!(
            owner_verification(
                &profile_event(&agent_keys, vec![valid_tag.clone()]),
                &owner_keys.public_key().to_hex(),
            ),
            "verified"
        );
        assert_eq!(
            owner_verification(
                &profile_event(&agent_keys, vec![foreign_tag]),
                &owner_keys.public_key().to_hex(),
            ),
            "owner_mismatch"
        );
        assert_eq!(
            owner_verification(
                &profile_event(&agent_keys, vec![]),
                &owner_keys.public_key().to_hex()
            ),
            "missing_auth"
        );
        assert_eq!(
            owner_verification(
                &profile_event(&agent_keys, vec![valid_tag.clone(), valid_tag]),
                &owner_keys.public_key().to_hex(),
            ),
            "multiple_auth_tags"
        );
        assert_eq!(
            owner_verification(
                &profile_event(
                    &agent_keys,
                    vec![json!([
                        "auth",
                        owner_keys.public_key().to_hex(),
                        "kind=9",
                        "0".repeat(128)
                    ])],
                ),
                &owner_keys.public_key().to_hex(),
            ),
            "invalid_auth"
        );
    }

    #[test]
    fn owner_verification_requires_conditions_to_apply_to_profile_event() {
        let owner_keys = Keys::generate();
        let agent_keys = Keys::generate();
        let verification = |conditions: &str| {
            let auth_tag: serde_json::Value = serde_json::from_str(
                &buzz_sdk::nip_oa::compute_auth_tag(
                    &owner_keys,
                    &agent_keys.public_key(),
                    conditions,
                )
                .unwrap(),
            )
            .unwrap();
            owner_verification(
                &profile_event(&agent_keys, vec![auth_tag]),
                &owner_keys.public_key().to_hex(),
            )
        };

        assert_eq!(verification("kind=9"), "condition_mismatch");
        assert_eq!(verification("created_at<100"), "condition_mismatch");
        assert_eq!(verification("created_at>100"), "condition_mismatch");
        assert_eq!(
            verification("kind=0&created_at>99&created_at<101"),
            "verified"
        );
    }

    #[test]
    fn owner_scoped_profiles_keep_drifted_and_missing_profiles_without_claiming_ownership() {
        let owner_keys = Keys::generate();
        let agent_keys = Keys::generate();
        let missing_keys = Keys::generate();
        let auth_tag: serde_json::Value = serde_json::from_str(
            &buzz_sdk::nip_oa::compute_auth_tag(&owner_keys, &agent_keys.public_key(), "kind=0")
                .unwrap(),
        )
        .unwrap();
        let events = vec![profile_event(&agent_keys, vec![auth_tag])];
        let pubkeys = vec![
            agent_keys.public_key().to_hex(),
            missing_keys.public_key().to_hex(),
            "malformed".to_string(),
        ];

        let profiles = owner_scoped_profiles(
            &events,
            &pubkeys,
            &owner_keys.public_key().to_hex(),
            &owner_keys.public_key().to_hex(),
        );

        assert_eq!(profiles[0]["display_name"], "Renamed Honey");
        assert_eq!(profiles[0]["verification"], "verified");
        assert_eq!(profiles[0]["owned_by_me"], true);
        assert_eq!(
            profiles[0]["owner_pubkey"],
            owner_keys.public_key().to_hex()
        );
        assert_eq!(profiles[1]["verification"], "missing_profile");
        assert_eq!(profiles[1]["owned_by_me"], false);
        assert!(profiles[1].get("owner_pubkey").is_none());
        assert_eq!(profiles[2]["verification"], "invalid_agent_pubkey");
        assert_eq!(profiles[2]["owned_by_me"], false);
        assert!(profiles[2].get("owner_pubkey").is_none());
    }

    #[test]
    fn presence_subject_uses_p_tag() {
        let event = json!({"pubkey": "relay", "tags": [["p", "user"]]});
        assert_eq!(presence_subject(&event), "user");
    }

    #[test]
    fn presence_subject_falls_back_to_author_without_p_tag() {
        let event = json!({"pubkey": "user", "tags": [["status", "online"]]});
        assert_eq!(presence_subject(&event), "user");
    }

    #[test]
    fn presence_subject_falls_back_to_author_for_malformed_p_tag() {
        let event = json!({"pubkey": "user", "tags": [["p"]]});
        assert_eq!(presence_subject(&event), "user");
    }
}
