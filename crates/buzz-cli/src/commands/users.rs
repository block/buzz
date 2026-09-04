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
        crate::OutputFormat::Json => serde_json::to_string(&profiles).unwrap_or_default(),
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
        crate::OutputFormat::Json => serde_json::to_string(&profiles).unwrap_or_default(),
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
    let fetched = fetch_current_profile(client).await?;
    let current = fetched.content;

    // Refuse a republish that would silently strip an existing owner attestation.
    //
    // kind:0 is replaceable, so a profile write REPLACES the stored event whole.
    // Content is merged above, but the NIP-OA `auth` tag is not a content field:
    // it is re-attached only by sign_event from BUZZ_AUTH_TAG, and sign_event
    // deliberately forbids callers supplying one (it is a signed attestation, not
    // a value to copy forward). So running this command without BUZZ_AUTH_TAG
    // against a profile that HAS the tag destroys it.
    //
    // That is not hypothetical: on 2026-08-12 an avatar re-host republished eight
    // agent profiles this way. The tag vanished, same-owner sibling verification
    // returned false fleet-wide, and every agent-to-agent message was dropped for
    // ~23 hours. Nothing failed loudly — the writes succeeded, the agents stayed
    // healthy, and only the owner could still reach them (matched by pubkey, which
    // never consults the profile).
    //
    // Fail closed instead: the operator either supplies the attestation or is told
    // exactly what they are about to destroy.
    if fetched.has_auth_tag && client.auth_tag_owner_hex().is_none() {
        return Err(CliError::Usage(
            "refusing to republish: your current profile carries a NIP-OA owner \
             attestation (auth tag) and BUZZ_AUTH_TAG is not set, so this write \
             would silently drop it and break same-owner verification. Set \
             BUZZ_AUTH_TAG to the attestation JSON and retry."
                .into(),
        ));
    }

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

/// Does this event carry a NIP-OA `auth` tag (`["auth", owner, cond, sig]`)?
///
/// Used to detect a profile republish that would strip an existing owner
/// attestation. Kept pure and separate from the network fetch so the decision
/// is testable — an untested guard here is how the 2026-08-12 fleet-wide
/// mention drop went unnoticed for ~23 hours.
fn event_has_auth_tag(event: &serde_json::Value) -> bool {
    event
        .get("tags")
        .and_then(|t| t.as_array())
        .is_some_and(|tags| {
            tags.iter().any(|t| {
                t.as_array()
                    .and_then(|a| a.first())
                    .and_then(|v| v.as_str())
                    == Some("auth")
            })
        })
}

/// Fetched profile state: the parsed content fields, plus whether the live
/// event carries a NIP-OA `auth` tag.
///
/// The tag matters separately from content because it is a signed owner
/// attestation, not a profile field — it can only be re-attached by
/// `sign_event` from `BUZZ_AUTH_TAG`, never copied forward by a caller.
struct CurrentProfile {
    content: serde_json::Map<String, serde_json::Value>,
    has_auth_tag: bool,
}

/// Fetch the current user's profile via POST /query (kind:0).
///
/// Returns the parsed content fields plus whether the stored event carries an
/// owner attestation. Empty content and `has_auth_tag: false` when no profile
/// exists yet.
async fn fetch_current_profile(client: &BuzzClient) -> Result<CurrentProfile, CliError> {
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
        return Ok(CurrentProfile {
            content: serde_json::Map::new(),
            has_auth_tag: false,
        });
    };
    let Some(event) = arr.first() else {
        return Ok(CurrentProfile {
            content: serde_json::Map::new(),
            has_auth_tag: false,
        });
    };
    let has_auth_tag = event_has_auth_tag(event);
    // kind:0 content is a JSON string containing the profile fields
    let content_str = event
        .get("content")
        .and_then(|c| c.as_str())
        .unwrap_or("{}");
    let content: serde_json::Value = serde_json::from_str(content_str).unwrap_or_default();
    Ok(CurrentProfile {
        content: content.as_object().cloned().unwrap_or_default(),
        has_auth_tag,
    })
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

/// Set user status — sign and submit a NIP-38 kind:30315 user status event.
///
/// Uses the `d:general` coordinate that the desktop client reads for the
/// profile status line. A blank `text` with no `emoji` clears the status.
pub async fn cmd_set_status(
    client: &BuzzClient,
    text: &str,
    emoji: Option<&str>,
) -> Result<(), CliError> {
    let builder = buzz_sdk::build_user_status(text, emoji).map_err(crate::validate::sdk_err)?;
    let event = client.sign_event(builder)?;
    let resp = client.submit_event(event).await?;
    println!("{}", normalize_write_response(&resp));
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
        UsersCmd::SetStatus { text, emoji, clear } => {
            // `--clear` is mutually exclusive with `--text`/`--emoji`: publish the
            // empty `d:general` event that clients read as "no status".
            let (text, emoji) = if clear {
                ("", None)
            } else {
                (text.as_deref().unwrap_or_default(), emoji.as_deref())
            };
            cmd_set_status(client, text, emoji).await
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        owned_agent_pubkeys_from_events, owner_scoped_profiles, owner_verification,
        presence_subject,
    };
    use nostr::Keys;
    use serde_json::json;

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

    #[test]
    fn event_has_auth_tag_detects_the_owner_attestation() {
        let event = json!({"tags": [["auth", "owner-pk", "", "sig"], ["p", "x"]]});
        assert!(super::event_has_auth_tag(&event));
    }

    #[test]
    fn event_has_auth_tag_is_false_when_the_attestation_was_stripped() {
        // The exact shape ENG-3879 left behind: content preserved, tags emptied.
        let event = json!({"content": "{\"display_name\":\"Sol-3\"}", "tags": []});
        assert!(!super::event_has_auth_tag(&event));
    }

    #[test]
    fn event_has_auth_tag_ignores_other_tags_and_malformed_entries() {
        let event = json!({"tags": [["p", "auth"], ["auth"], [], "auth"]});
        // ["auth"] alone is a real auth tag position — first element matches.
        assert!(super::event_has_auth_tag(&event));
        let event = json!({"tags": [["p", "auth"], [], "auth"]});
        assert!(!super::event_has_auth_tag(&event));
    }

    #[test]
    fn event_has_auth_tag_is_false_without_a_tags_field() {
        assert!(!super::event_has_auth_tag(&json!({"content": "{}"})));
    }
}

#[cfg(test)]
mod set_profile_seam_tests {
    //! Drive the real `cmd_set_profile` against a fake relay.
    //!
    //! `event_has_auth_tag` tests prove the predicate; they cannot prove the
    //! guard is wired into the write path, that the refusal happens BEFORE any
    //! event is submitted, or that an attested write still round-trips. Those
    //! are the properties the 2026-08-12 incident actually needed, so they are
    //! asserted here against the live query → guard → sign → submit chain, with
    //! the relay's received events as the evidence.
    use std::net::SocketAddr;
    use std::sync::{Arc, Mutex};

    use axum::routing::post;
    use axum::Router;
    use nostr::{Keys, Tag};
    use tokio::net::TcpListener;

    use super::cmd_set_profile;
    use crate::client::BuzzClient;
    use crate::error::CliError;

    type Submitted = Arc<Mutex<Vec<serde_json::Value>>>;

    const OWNER: &str = "95b5d50f247a714a2b1ff68798f4cc6a50effd8046638d9e6339a096e6d6b2ad";
    const SIG: &str = "a4c05492fb3df820037ef55617cc2563233df2e5a07ed27559d5aad23d07d5d9\
74f385095d15c0af2bff62ded61a8d2030caaa0c8c7edc3c3cf3e47820b87da9";

    /// A stored kind:0 exactly as the relay serves it: content carries the
    /// profile fields, the attestation lives in `tags`.
    fn stored_profile(with_auth_tag: bool) -> serde_json::Value {
        let tags = if with_auth_tag {
            serde_json::json!([["auth", OWNER, "", SIG]])
        } else {
            serde_json::json!([])
        };
        serde_json::json!({
            "kind": 0,
            "content": r#"{"display_name":"Neo","picture":"https://relay/avatar.png"}"#,
            "tags": tags,
        })
    }

    async fn relay(profile: serde_json::Value) -> (String, Submitted) {
        let submitted: Submitted = Arc::new(Mutex::new(Vec::new()));
        let captured = submitted.clone();
        let app = Router::new()
            .route(
                "/query",
                post(move || {
                    let profile = profile.clone();
                    async move { axum::Json(serde_json::json!([profile])) }
                }),
            )
            .route(
                "/events",
                post(move |body: String| {
                    let captured = captured.clone();
                    async move {
                        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&body) {
                            captured.lock().unwrap().push(v);
                        }
                        axum::Json(serde_json::json!({
                            "event_id": "0".repeat(64), "accepted": true, "message": ""
                        }))
                    }
                }),
            );
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr: SocketAddr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        (format!("http://{addr}"), submitted)
    }

    fn client(base_url: &str, attested: bool) -> BuzzClient {
        let auth_tag = attested.then(|| Tag::parse(["auth", OWNER, "", SIG]).unwrap());
        BuzzClient::new(base_url.to_string(), Keys::generate(), auth_tag, None).unwrap()
    }

    fn has_auth_tag(event: &serde_json::Value) -> bool {
        event["tags"]
            .as_array()
            .is_some_and(|tags| tags.iter().any(|t| t[0] == "auth"))
    }

    #[tokio::test]
    async fn attested_profile_without_configured_attestation_submits_nothing() {
        // The incident itself: the write succeeded and took the tag with it.
        // The refusal must land before anything reaches the relay — an error
        // returned after submitting would still have destroyed the attestation.
        let (url, submitted) = relay(stored_profile(true)).await;
        let err = cmd_set_profile(&client(&url, false), Some("Neo"), None, None, None)
            .await
            .unwrap_err();

        assert!(
            matches!(err, CliError::Usage(ref m) if m.contains("refusing to republish")),
            "expected a usage refusal naming the attestation, got {err:?}"
        );
        assert!(
            submitted.lock().unwrap().is_empty(),
            "guard refused but still wrote to the relay — the attestation is gone anyway"
        );
    }

    #[tokio::test]
    async fn attested_profile_with_attestation_publishes_one_replacement_carrying_the_tag() {
        let (url, submitted) = relay(stored_profile(true)).await;
        cmd_set_profile(&client(&url, true), Some("Neo Prime"), None, None, None)
            .await
            .expect("attested republish must be allowed");

        let events = submitted.lock().unwrap();
        assert_eq!(events.len(), 1, "expected exactly one replacement write");
        assert!(
            has_auth_tag(&events[0]),
            "replacement lost the owner attestation: {}",
            events[0]
        );
        // kind:0 is replaceable, so unspecified fields must survive the merge —
        // the incident began as an avatar rewrite that blanked `picture`.
        let content: serde_json::Value =
            serde_json::from_str(events[0]["content"].as_str().unwrap()).unwrap();
        assert_eq!(content["display_name"], "Neo Prime");
        assert_eq!(content["picture"], "https://relay/avatar.png");
    }

    #[tokio::test]
    async fn unattested_profile_remains_editable_without_an_attestation() {
        // The guard must not become a blanket block: a profile that never had
        // an attestation has none to lose.
        let (url, submitted) = relay(stored_profile(false)).await;
        cmd_set_profile(&client(&url, false), Some("Scout"), None, None, None)
            .await
            .expect("unattested profile must stay editable");

        let events = submitted.lock().unwrap();
        assert_eq!(events.len(), 1, "expected the profile write to go through");
        assert!(!has_auth_tag(&events[0]));
    }
}
