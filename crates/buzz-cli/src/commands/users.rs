use buzz_core::kind::KIND_MANAGED_AGENT;
use nostr::PublicKey;

use crate::client::{extract_d_tag, normalize_write_response, BuzzClient};
use crate::error::CliError;
use crate::validate::validate_hex64;

use super::agents::fetch_archived_snapshot;
use super::verified_profiles::{
    discover_authority, fetch_verified_profiles, verified_profiles_from_events, VerifiedProfile,
};
use std::collections::{HashMap, HashSet};

/// Longest query `POST /profiles/search` accepts, in characters.
const PROFILE_SEARCH_MAX_CHARS: usize = 100;

// TODO(phase-4): Replace raw nostr::EventBuilder usage in cmd_set_presence with buzz-sdk builder

/// Get user profiles (kind:0 metadata events).
///
/// - 0 pubkeys, no name → query our own profile
/// - 1+ pubkeys → query those users' profiles
/// - --name "foo" → [`search_profiles`]
///
/// Where the community advertises NIP-FI verified profiles, each person with a
/// valid relay-signed kind:30382 also carries a `verified` object.
///
/// A person on the relay's NIP-IA archive list is marked `"archived": true`.
/// A name search leaves them out unless `include_archived`; a pubkey lookup
/// always returns them, since whoever has the key is not discovering anyone.
pub async fn cmd_get_users(
    client: &BuzzClient,
    pubkeys: &[String],
    name: Option<&str>,
    owner: Option<&str>,
    include_archived: bool,
    format: &crate::OutputFormat,
) -> Result<(), CliError> {
    let output = get_users_output(client, pubkeys, name, owner, include_archived, format).await?;
    println!("{output}");
    Ok(())
}

/// What [`cmd_get_users`] prints.
async fn get_users_output(
    client: &BuzzClient,
    pubkeys: &[String],
    name: Option<&str>,
    owner: Option<&str>,
    include_archived: bool,
    format: &crate::OutputFormat,
) -> Result<String, CliError> {
    if let Some(query) = name {
        if !pubkeys.is_empty() {
            return Err(CliError::Usage(
                "--name and --pubkey are mutually exclusive".into(),
            ));
        }
        return search_by_name(client, query, owner, include_archived, format).await;
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

    let Some((profiles, archived)) = alongside_archive(client, async {
        get_profiles(client, pubkeys).await.map(Some)
    })
    .await?
    else {
        unreachable!("pubkey lookups always return profiles")
    };
    let profiles = with_archived(profiles, archived.as_ref(), true);
    let output = match format {
        crate::OutputFormat::Compact => {
            let compact: Vec<serde_json::Value> = profiles
                .iter()
                .map(|p| compact_profile(p, &["archived"]))
                .collect();
            serde_json::to_string(&compact).unwrap_or_default()
        }
        crate::OutputFormat::Json | crate::OutputFormat::Agent => {
            serde_json::to_string(&profiles).unwrap_or_default()
        }
    };
    Ok(output)
}

/// Runs `profiles` while the archive list is read. A profile error returns at
/// once, and `None` (nothing to show) returns without waiting for the list.
/// Otherwise it waits for the list, and when the list can't be read it warns
/// on stderr and gives `None`: archive filtering fails open, never failing
/// the lookup.
async fn alongside_archive(
    client: &BuzzClient,
    profiles: impl std::future::Future<Output = Result<Option<Vec<serde_json::Value>>, CliError>>,
) -> Result<Option<(Vec<serde_json::Value>, Option<HashSet<String>>)>, CliError> {
    let archive = archived_pubkeys(client);
    tokio::pin!(archive, profiles);
    let mut archived = None;
    let profiles = loop {
        tokio::select! {
            profiles = &mut profiles => break profiles?,
            read = &mut archive, if archived.is_none() => archived = Some(read),
        }
    };
    let Some(profiles) = profiles else {
        return Ok(None);
    };
    let archived = match archived {
        Some(read) => read,
        None => archive.await,
    };
    let archived = archived
        .map_err(|warning| eprintln!("{}", serde_json::json!({ "warning": warning })))
        .ok();
    Ok(Some((profiles, archived)))
}

/// The relay's archived pubkeys, or the warning to show when the list can't
/// be read or verified.
async fn archived_pubkeys(client: &BuzzClient) -> Result<HashSet<String>, String> {
    archived_pubkeys_within(client, ARCHIVE_READ_DEADLINE).await
}

/// How long a lookup waits for the archive list before giving up on filtering.
const ARCHIVE_READ_DEADLINE: std::time::Duration = std::time::Duration::from_secs(5);

async fn archived_pubkeys_within(
    client: &BuzzClient,
    deadline: std::time::Duration,
) -> Result<HashSet<String>, String> {
    let read = tokio::time::timeout(deadline, fetch_archived_snapshot(client))
        .await
        .unwrap_or_else(|_| Err(CliError::Other(format!("timed out after {deadline:?}"))));
    read.map(|archived| archived.into_iter().collect())
        .map_err(|e| format!("archived-identities list unavailable, results not filtered: {e}"))
}

/// Marks each profile on the `archived` list `"archived": true`, dropping it
/// unless `keep_archived`. With no list, profiles pass through unmarked. The
/// profile parsers strip any self-authored `archived`, so the marker only
/// ever reflects the relay's signed list.
fn with_archived(
    profiles: Vec<serde_json::Value>,
    archived: Option<&HashSet<String>>,
    keep_archived: bool,
) -> Vec<serde_json::Value> {
    profiles
        .into_iter()
        .filter_map(|mut profile| {
            let Some(archived) = archived else {
                return Some(profile);
            };
            let Some(pubkey) = profile.get("pubkey").and_then(|v| v.as_str()) else {
                return Some(profile);
            };
            let pubkey = pubkey.to_ascii_lowercase();
            if !archived.contains(&pubkey) {
                return Some(profile);
            }
            if !keep_archived {
                return None;
            }
            if let Some(obj) = profile.as_object_mut() {
                obj.insert("archived".to_string(), serde_json::json!(true));
            }
            Some(profile)
        })
        .collect()
}

/// kind:0 profiles for `pubkeys` (our own when empty), each with its verified
/// profile where one exists, then any requested person who has a verified
/// profile but no kind:0.
///
/// A failed verified-profile check never drops the kind:0 output; it is
/// reported as a warning on stderr so it is never read as "not verified".
async fn get_profiles(
    client: &BuzzClient,
    pubkeys: &[String],
) -> Result<Vec<serde_json::Value>, CliError> {
    let (profiles, warning) = lookup_profiles(client, pubkeys).await?;
    if let Some(warning) = warning {
        eprintln!("{warning}");
    }
    Ok(profiles)
}

/// [`get_profiles`] with its warning returned instead of printed.
async fn lookup_profiles(
    client: &BuzzClient,
    pubkeys: &[String],
) -> Result<(Vec<serde_json::Value>, Option<serde_json::Value>), CliError> {
    let my_pk = client.public_key().to_hex();
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
                obj.remove("verified");
                obj.remove("archived");
                obj.insert(
                    "pubkey".to_string(),
                    serde_json::json!(e.get("pubkey").and_then(|v| v.as_str()).unwrap_or("")),
                );
            }
            Some(profile)
        })
        .collect();
    let requested: Vec<String> = authors.iter().map(|a| a.to_string()).collect();
    let (verified, warning) = match verified_for(client, &requested).await {
        Ok(verified) => (verified, None),
        Err(e) => (HashMap::new(), Some(verified_unavailable(&e))),
    };
    let mut profiles = with_verified(profiles, &verified);
    // A requested person with a verified profile but no kind:0 is still found,
    // as a name search would find them.
    let mut found: Vec<String> = profiles
        .iter()
        .filter_map(|p| p.get("pubkey").and_then(|v| v.as_str()))
        .map(str::to_ascii_lowercase)
        .collect();
    for pubkey in requested.iter().map(|p| p.to_ascii_lowercase()) {
        if let (false, Some(v)) = (found.contains(&pubkey), verified.get(&pubkey)) {
            profiles.push(verified_only_profile(&pubkey, v));
            found.push(pubkey);
        }
    }
    Ok((profiles, warning))
}

/// The stderr warning for a verified-profile check that could not run.
fn verified_unavailable(error: &CliError) -> serde_json::Value {
    serde_json::json!({
        "warning": "verified_profiles_unavailable",
        "message": error.to_string(),
    })
}

/// Verified profiles for `pubkeys`; empty where the community advertises none.
async fn verified_for(
    client: &BuzzClient,
    pubkeys: &[String],
) -> Result<HashMap<String, VerifiedProfile>, CliError> {
    match discover_authority(client).await? {
        Some(authority) => fetch_verified_profiles(client, &authority.pubkey, pubkeys).await,
        None => Ok(HashMap::new()),
    }
}

/// Attach each profile's `verified` object where a relay-signed one exists.
fn with_verified(
    mut profiles: Vec<serde_json::Value>,
    verified: &HashMap<String, VerifiedProfile>,
) -> Vec<serde_json::Value> {
    for profile in &mut profiles {
        let pubkey = profile
            .get("pubkey")
            .and_then(|v| v.as_str())
            .map(str::to_ascii_lowercase);
        if let (Some(found), Some(obj)) = (
            pubkey.as_deref().and_then(|p| verified.get(p)),
            profile.as_object_mut(),
        ) {
            obj.insert("verified".to_string(), found.to_json());
        }
    }
    profiles
}

/// A person with a verified profile but no kind:0.
fn verified_only_profile(pubkey: &str, verified: &VerifiedProfile) -> serde_json::Value {
    serde_json::json!({
        "pubkey": pubkey,
        "display_name": verified.display_name,
        "verified": verified.to_json(),
    })
}

/// People from a `/profiles/search` page: each person's kind:0 and, when
/// `authority` signed one, their verified profile, in the relay's order. A
/// person who matched only by verified name has no kind:0.
fn search_page_profiles(events: &[serde_json::Value], authority: &str) -> Vec<serde_json::Value> {
    let verified = verified_profiles_from_events(events, authority);
    let mut order: Vec<String> = Vec::new();
    let mut profiles: HashMap<String, serde_json::Map<String, serde_json::Value>> = HashMap::new();
    for event in events {
        let kind = event.get("kind").and_then(|v| v.as_u64());
        let pubkey = if kind == Some(0) {
            event
                .get("pubkey")
                .and_then(|v| v.as_str())
                .map(str::to_ascii_lowercase)
        } else if kind == Some(u64::from(buzz_core::kind::KIND_VERIFIED_PROFILE)) {
            // Only a valid projection by the advertised key finds anybody.
            Some(extract_d_tag(event)).filter(|subject| verified.contains_key(subject))
        } else {
            None
        };
        let Some(pubkey) = pubkey else {
            continue;
        };
        if !order.contains(&pubkey) {
            order.push(pubkey.clone());
        }
        if kind == Some(0) {
            profiles.insert(pubkey, profile_content(event));
        }
    }
    order
        .into_iter()
        .filter_map(|pubkey| {
            let found = verified.get(&pubkey);
            match profiles.remove(&pubkey) {
                Some(mut content) => {
                    content.insert("pubkey".to_string(), serde_json::json!(pubkey));
                    if let Some(found) = found {
                        content.insert("verified".to_string(), found.to_json());
                    }
                    Some(serde_json::Value::Object(content))
                }
                None => found.map(|found| verified_only_profile(&pubkey, found)),
            }
        })
        .collect()
}

/// The compact form of a profile: pubkey, name, any `verified` object, and the
/// listed `extra` fields when present.
fn compact_profile(profile: &serde_json::Value, extra: &[&str]) -> serde_json::Value {
    let mut value = serde_json::json!({
        "pubkey": profile.get("pubkey").cloned().unwrap_or_default(),
        "display_name": profile
            .get("display_name")
            .or_else(|| profile.get("name"))
            .cloned()
            .unwrap_or_default(),
    });
    if let Some(obj) = value.as_object_mut() {
        for field in extra.iter().chain(&["verified"]) {
            if let Some(field_value) = profile.get(*field) {
                obj.insert(field.to_string(), field_value.clone());
            }
        }
    }
    value
}

fn effective_owner(client: &BuzzClient) -> String {
    client
        .auth_tag_owner_hex()
        .unwrap_or_else(|| client.public_key().to_hex())
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

/// The object in a kind:0 event's content. kind:0 is user-controlled, so any
/// `verified` key in it is dropped; only a relay-signed kind:30382 sets one.
fn profile_content(event: &serde_json::Value) -> serde_json::Map<String, serde_json::Value> {
    let mut profile = event
        .get("content")
        .and_then(|value| value.as_str())
        .and_then(|content| serde_json::from_str::<serde_json::Value>(content).ok())
        .and_then(|content| content.as_object().cloned())
        .unwrap_or_default();
    profile.remove("verified");
    profile.remove("archived");
    profile
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

/// Unscoped name search. Where the community advertises searchable verified
/// profiles, `POST /profiles/search` matches profile names and verified names
/// (word prefixes), so a person is found by their verified username too.
/// Otherwise, or for a query longer than that endpoint accepts, NIP-50 search
/// over kind:0, filtered to name substring matches (returns [] if
/// unsupported), with verified profiles attached where advertised. As for
/// [`get_profiles`], a failed verified check is a stderr warning, not an error.
async fn search_profiles(
    client: &BuzzClient,
    query: &str,
) -> Result<Vec<serde_json::Value>, CliError> {
    let (profiles, warning) = find_profiles(client, query).await?;
    if let Some(warning) = warning {
        eprintln!("{warning}");
    }
    Ok(profiles)
}

/// [`search_profiles`] with its warning returned instead of printed.
async fn find_profiles(
    client: &BuzzClient,
    query: &str,
) -> Result<(Vec<serde_json::Value>, Option<serde_json::Value>), CliError> {
    let parse = |raw: String| -> Result<Vec<serde_json::Value>, CliError> {
        serde_json::from_str(&raw)
            .map_err(|e| CliError::Other(format!("failed to parse response: {e}")))
    };
    let (authority, warning) = match discover_authority(client).await {
        Ok(authority) => (authority, None),
        Err(e) => (None, Some(verified_unavailable(&e))),
    };
    let fits = query.trim().chars().count() <= PROFILE_SEARCH_MAX_CHARS;
    if let Some(authority) = authority.as_ref().filter(|a| a.searchable && fits) {
        let body = serde_json::json!({"query": query.trim(), "limit": 100});
        let raw = client.post_json_authed("/profiles/search", &body).await?;
        return Ok((search_page_profiles(&parse(raw)?, &authority.pubkey), None));
    }
    let filter = serde_json::json!({
        "kinds": [0],
        "search": query,
        "limit": 100
    });
    let events = parse(client.query(&filter).await?)?;
    let profiles = name_search_profiles(&events, query);
    let Some(authority) = authority else {
        return Ok((profiles, warning));
    };
    let pubkeys: Vec<String> = profiles
        .iter()
        .filter_map(|p| p.get("pubkey").and_then(|v| v.as_str()).map(str::to_owned))
        .collect();
    match fetch_verified_profiles(client, &authority.pubkey, &pubkeys).await {
        Ok(verified) => Ok((with_verified(profiles, &verified), None)),
        Err(e) => Ok((profiles, Some(verified_unavailable(&e)))),
    }
}

/// Search for users by name. Owner-scoped searches resolve managed-agent records
/// and verify their profiles; unscoped searches go through [`search_profiles`].
async fn search_by_name(
    client: &BuzzClient,
    query: &str,
    owner: Option<&str>,
    include_archived: bool,
    format: &crate::OutputFormat,
) -> Result<String, CliError> {
    if query.trim().is_empty() {
        return Err(CliError::Usage("--name cannot be empty".into()));
    }

    let owner = resolve_owner(client, owner)?;
    let profiles = async {
        let Some(owner) = owner else {
            return search_profiles(client, query).await.map(Some);
        };
        let pubkeys = owned_agent_pubkeys_by_name(client, &owner, query).await?;
        if pubkeys.is_empty() {
            return Ok(None);
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
        Ok(Some(owner_scoped_profiles(
            &events,
            &pubkeys,
            &owner,
            &effective_owner(client),
        )))
    };
    let Some((profiles, archived)) = alongside_archive(client, profiles).await? else {
        return Ok("[]".to_string());
    };
    let profiles = with_archived(profiles, archived.as_ref(), include_archived);
    let output = match format {
        crate::OutputFormat::Compact => {
            let compact: Vec<serde_json::Value> = profiles
                .iter()
                .map(|p| {
                    compact_profile(
                        p,
                        &["owner_pubkey", "owned_by_me", "verification", "archived"],
                    )
                })
                .collect();
            serde_json::to_string(&compact).unwrap_or_default()
        }
        crate::OutputFormat::Json | crate::OutputFormat::Agent => {
            serde_json::to_string(&profiles).unwrap_or_default()
        }
    };
    Ok(output)
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

    let event = client.sign_event(builder).await?;

    let resp = client.submit_event(event).await?;
    println!("{}", normalize_write_response(&resp));
    Ok(())
}

/// Fetch the current user's profile metadata via POST /query (kind:0).
/// Returns the parsed content JSON object, or an empty object if no profile exists.
async fn fetch_current_profile(
    client: &BuzzClient,
) -> Result<serde_json::Map<String, serde_json::Value>, CliError> {
    let my_pk = client.public_key().to_hex();
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
    let event = client.sign_event(builder).await?;

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
    let event = client.sign_event(builder).await?;
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
        authors.push(client.public_key().to_hex());
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
            include_archived,
        } => {
            cmd_get_users(
                client,
                &pubkeys,
                name.as_deref(),
                owner.as_deref(),
                include_archived,
                format,
            )
            .await
        }
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
        archived_pubkeys, archived_pubkeys_within, compact_profile, find_profiles, get_profiles,
        get_users_output, lookup_profiles, owned_agent_pubkeys_from_events, owner_scoped_profiles,
        owner_verification, parse_expiration_tag, parse_expiry, presence_subject, publish_status,
        read_statuses, search_profiles, with_archived, ARCHIVE_READ_DEADLINE,
    };
    use crate::client::BuzzClient;
    use crate::error::CliError;
    use axum::{
        body::Bytes, extract::State, http::StatusCode, routing::get, routing::post, Json, Router,
    };
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
        /// Never answer queries for this kind.
        held_kind: Option<u64>,
        /// Answer queries for this kind with invalid JSON.
        broken_kind: Option<u64>,
        /// Answer queries for this kind with HTTP 500.
        failing_kind: Option<u64>,
        /// Hold kind:0 queries until the archive-list query has arrived.
        profiles_after_archive: bool,
        /// NIP-11 document served at `/`.
        info: Value,
        /// Body of `POST /profiles/search`; `None` answers 404 like an older relay.
        profile_search: Option<Value>,
        profile_searches: Vec<Value>,
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
                    let held = relay.lock().unwrap().held_kind;
                    if held.is_some() && filters[0]["kinds"][0].as_u64() == held {
                        std::future::pending::<()>().await;
                    }
                    if filters[0]["kinds"][0] == 0 {
                        while relay.lock().unwrap().profiles_after_archive
                            && !relay
                                .lock()
                                .unwrap()
                                .queries
                                .iter()
                                .any(|q| q["kinds"][0] == 13535)
                        {
                            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
                        }
                    }
                    let [filter] = filters.as_slice() else {
                        panic!("one filter expected: {filters:?}")
                    };
                    let mut relay = relay.lock().unwrap();
                    relay.queries.push(filter.clone());
                    if let Some(raw) = relay.raw_query_response.clone() {
                        return (StatusCode::OK, raw);
                    }
                    if relay.failing_kind.is_some()
                        && filter["kinds"][0].as_u64() == relay.failing_kind
                    {
                        return (StatusCode::INTERNAL_SERVER_ERROR, "boom".to_string());
                    }
                    if relay.broken_kind.is_some()
                        && filter["kinds"][0].as_u64() == relay.broken_kind
                    {
                        return (StatusCode::OK, "not json".to_string());
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
                        .filter(|e| match filter.get("#d") {
                            Some(d) => d
                                .as_array()
                                .unwrap()
                                .contains(&json!(e["tags"][0][1].as_str().unwrap_or_default())),
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
                    (StatusCode::OK, serde_json::to_string(&matching).unwrap())
                }),
            )
            .route(
                "/",
                get(|State(relay): State<Shared>| async move {
                    Json(relay.lock().unwrap().info.clone())
                }),
            )
            .route(
                "/profiles/search",
                post(|State(relay): State<Shared>, body: Bytes| async move {
                    let mut relay = relay.lock().unwrap();
                    relay
                        .profile_searches
                        .push(serde_json::from_slice(&body).unwrap());
                    match relay.profile_search.clone() {
                        Some(events) => (StatusCode::OK, Json(events)),
                        None => (StatusCode::NOT_FOUND, Json(json!({"error": "not found"}))),
                    }
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
        setup_as(Keys::generate()).await
    }

    async fn setup_as(keys: Keys) -> (BuzzClient, Shared) {
        let relay = Shared::default();
        let url = fake_relay(relay.clone()).await;
        let client =
            BuzzClient::new(url, crate::credentials::Credentials::from_keys(keys)).unwrap();
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
        let me = client.credentials().test_keys();
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

    /// A NIP-11 document advertising verified profiles signed by `relay`.
    fn advertising(relay: &Keys) -> Value {
        json!({
            "self": relay.public_key().to_hex(),
            "federated_identity": {
                "public_profile": {"kind": 30382, "multiple_issuers": false, "searchable": true}
            },
        })
    }

    /// Seed a relay-signed verified profile for `subject`.
    fn seed_verified(relay: &Shared, signer: &Keys, subject: &Keys, username: &str, at: u64) {
        let subject = subject.public_key().to_hex();
        seed(
            relay,
            signer,
            30382,
            "",
            vec![
                vec!["d", &subject],
                vec!["p", &subject],
                vec!["active", "true"],
                vec!["display_name", "Alice"],
                vec!["username", username],
                vec!["full_name", "Alice Example"],
                vec!["issuer_label", "Example"],
                vec!["verified_at", "100"],
            ],
            at,
        );
    }

    fn seed_profile(relay: &Shared, keys: &Keys, content: &str) {
        seed(relay, keys, 0, content, vec![], 50);
    }

    fn verified_json(username: &str) -> Value {
        json!({
            "username": username,
            "display_name": "Alice",
            "full_name": "Alice Example",
            "issuer_label": "Example",
            "verified_at": 100,
        })
    }

    /// Seeds the relay's kind:13535 archive list, signed by `signer`.
    fn seed_archive_list(relay: &Shared, signer: &Keys, archived: &[&Keys]) {
        let hexes: Vec<String> = archived.iter().map(|k| k.public_key().to_hex()).collect();
        let mut tags = vec![vec!["-"]];
        tags.extend(hexes.iter().map(|h| vec!["p", h.as_str()]));
        seed(relay, signer, 13535, "", tags, 100);
    }

    fn rows(keys: &[&Keys]) -> Vec<Value> {
        keys.iter()
            .map(|k| json!({"pubkey": k.public_key().to_hex(), "name": "Alice Example"}))
            .collect()
    }

    #[tokio::test]
    async fn archive_filter_reads_the_relay_signed_list() {
        let (client, relay) = setup().await;
        let relay_keys = Keys::generate();
        let (old, new) = (Keys::generate(), Keys::generate());
        relay.lock().unwrap().info = json!({"self": relay_keys.public_key().to_hex()});
        seed_archive_list(&relay, &relay_keys, &[&old]);

        let archived = archived_pubkeys(&client).await.expect("list readable");
        let profiles = rows(&[&old, &new]);

        // Name search: excluded by default.
        let found = with_archived(profiles.clone(), Some(&archived), false);
        assert_eq!(found, rows(&[&new]));

        // --include-archived (and --pubkey): kept and marked.
        let found = with_archived(profiles, Some(&archived), true);
        assert_eq!(found.len(), 2);
        assert_eq!(found[0]["archived"], true);
        assert!(found[1].get("archived").is_none());
    }

    /// `buzz users get` output, parsed, in both normal and compact format.
    async fn users_get(
        client: &BuzzClient,
        pubkeys: &[String],
        name: Option<&str>,
        include: bool,
    ) -> [Vec<Value>; 2] {
        let mut out = Vec::new();
        for format in [crate::OutputFormat::Json, crate::OutputFormat::Compact] {
            let raw = get_users_output(client, pubkeys, name, None, include, &format)
                .await
                .unwrap();
            out.push(serde_json::from_str(&raw).unwrap());
        }
        out.try_into().unwrap()
    }

    fn archived_flags(rows: &[Value]) -> Vec<Option<bool>> {
        rows.iter()
            .map(|p| p.get("archived").map(|v| v.as_bool().unwrap()))
            .collect()
    }

    #[tokio::test]
    async fn users_get_marks_archived_only_from_the_signed_list() {
        let (client, relay) = setup().await;
        let relay_keys = Keys::generate();
        let (listed, claims) = (Keys::generate(), Keys::generate());
        relay.lock().unwrap().info = json!({"self": relay_keys.public_key().to_hex()});
        // Both set their own `archived: true`; only `listed` is on the relay's list.
        seed_profile(
            &relay,
            &listed,
            r#"{"name":"Alice Example","archived":true}"#,
        );
        seed_profile(&relay, &claims, r#"{"name":"Alice Other","archived":true}"#);
        seed_archive_list(&relay, &relay_keys, &[&listed]);
        let pubkeys = vec![listed.public_key().to_hex(), claims.public_key().to_hex()];

        for rows in users_get(&client, &[], Some("alice"), false).await {
            assert_eq!(rows.len(), 1, "{rows:?}");
            assert_eq!(rows[0]["pubkey"], claims.public_key().to_hex());
            assert_eq!(archived_flags(&rows), [None]);
        }
        for rows in users_get(&client, &[], Some("alice"), true).await {
            assert_eq!(archived_flags(&rows), [Some(true), None], "{rows:?}");
        }
        for rows in users_get(&client, &pubkeys, None, false).await {
            assert_eq!(archived_flags(&rows), [Some(true), None], "{rows:?}");
        }
        // A non-object kind:0 still comes back with a healthy list.
        let odd = Keys::generate();
        seed_profile(&relay, &odd, "null");
        for rows in users_get(&client, &[odd.public_key().to_hex()], None, false).await {
            assert_eq!(rows.len(), 1, "{rows:?}");
        }

        // Unreadable list: nothing filtered or marked.
        relay.lock().unwrap().broken_kind = Some(13535);
        for rows in users_get(&client, &[], Some("alice"), false).await {
            assert_eq!(archived_flags(&rows), [None, None], "{rows:?}");
        }
        for rows in users_get(&client, &pubkeys, None, false).await {
            assert_eq!(archived_flags(&rows), [None, None], "{rows:?}");
        }
    }

    #[tokio::test]
    async fn archive_filter_fails_open_when_the_list_read_hangs() {
        let (client, relay) = setup().await;
        let relay_keys = Keys::generate();
        relay.lock().unwrap().info = json!({"self": relay_keys.public_key().to_hex()});
        relay.lock().unwrap().held_kind = Some(13535);
        let deadline = std::time::Duration::from_millis(100);
        assert!(archived_pubkeys_within(&client, deadline).await.is_err());
    }

    #[tokio::test]
    async fn profile_errors_return_without_waiting_for_the_archive_read() {
        // Rerun in a child process to read its stderr: no archive warning.
        if std::env::var_os("BUZZ_USERS_STDERR_CHILD").is_none() {
            let out = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "commands::users::tests::profile_errors_return_without_waiting_for_the_archive_read",
                    "--nocapture",
                ])
                .env("BUZZ_USERS_STDERR_CHILD", "1")
                .output()
                .unwrap();
            let stderr = String::from_utf8_lossy(&out.stderr);
            let stdout = String::from_utf8_lossy(&out.stdout);
            assert!(out.status.success(), "{stdout}{stderr}");
            assert!(stdout.contains("1 passed"), "{stdout}");
            assert!(!stderr.contains("archived-identities"), "{stderr}");
            return;
        }
        let (client, relay) = setup().await;
        let relay_keys = Keys::generate();
        relay.lock().unwrap().info = json!({"self": relay_keys.public_key().to_hex()});
        relay.lock().unwrap().held_kind = Some(13535);
        relay.lock().unwrap().failing_kind = Some(0);
        let pubkey = Keys::generate().public_key().to_hex();
        for (pubkeys, name) in [(vec![pubkey], None), (vec![], Some("alice"))] {
            let started = std::time::Instant::now();
            let result = get_users_output(
                &client,
                &pubkeys,
                name,
                None,
                false,
                &crate::OutputFormat::Json,
            )
            .await;
            assert!(result.is_err(), "{name:?}: {result:?}");
            assert!(started.elapsed() < ARCHIVE_READ_DEADLINE / 2, "{name:?}");
        }
    }

    #[tokio::test]
    async fn name_search_reads_the_archive_list_alongside_the_profiles() {
        let (client, relay) = setup().await;
        let relay_keys = Keys::generate();
        let (old, new) = (Keys::generate(), Keys::generate());
        relay.lock().unwrap().info = json!({"self": relay_keys.public_key().to_hex()});
        seed_profile(&relay, &old, r#"{"name":"Alice Old"}"#);
        seed_profile(&relay, &new, r#"{"name":"Alice New"}"#);
        seed_archive_list(&relay, &relay_keys, &[&old]);
        // The profile search answers only once the archive read has started.
        relay.lock().unwrap().profiles_after_archive = true;
        let raw = tokio::time::timeout(
            std::time::Duration::from_secs(2),
            get_users_output(
                &client,
                &[],
                Some("alice"),
                None,
                false,
                &crate::OutputFormat::Json,
            ),
        )
        .await
        .expect("archive read starts before the profile search finishes")
        .unwrap();
        let rows: Vec<Value> = serde_json::from_str(&raw).unwrap();
        assert_eq!(rows.len(), 1, "{rows:?}");
        assert_eq!(rows[0]["pubkey"], new.public_key().to_hex());
    }

    #[tokio::test]
    async fn archive_filter_fails_open_when_the_list_is_unreadable() {
        // No relay `self`, and a list signed by someone other than the relay.
        for forged in [false, true] {
            let (client, relay) = setup().await;
            let old = Keys::generate();
            if forged {
                let relay_keys = Keys::generate();
                relay.lock().unwrap().info = json!({"self": relay_keys.public_key().to_hex()});
                let impostor = Keys::generate();
                seed_archive_list(&relay, &impostor, &[&old]);
                // The query asks for the relay's author; make the impostor's event answer it.
                relay.lock().unwrap().events[0]["pubkey"] = json!(relay_keys.public_key().to_hex());
            }
            let archived = archived_pubkeys(&client).await.ok();
            assert!(archived.is_none(), "forged={forged}");
            let profiles = rows(&[&old]);
            assert_eq!(
                with_archived(profiles.clone(), archived.as_ref(), false),
                profiles
            );
        }
    }

    #[tokio::test]
    async fn get_by_pubkey_shows_the_verified_identity_the_relay_signed() {
        let (client, relay) = setup().await;
        let authority = Keys::generate();
        relay.lock().unwrap().info = advertising(&authority);
        let alice = Keys::generate();
        let plain = Keys::generate();
        seed_profile(&relay, &alice, r#"{"name":"al"}"#);
        seed_profile(&relay, &plain, r#"{"name":"plain"}"#);
        seed_verified(&relay, &authority, &alice, "alice", 100);

        let pubkeys = vec![alice.public_key().to_hex(), plain.public_key().to_hex()];
        let profiles = get_profiles(&client, &pubkeys).await.unwrap();
        assert_eq!(profiles.len(), 2);
        assert_eq!(profiles[0]["name"], "al");
        assert_eq!(profiles[0]["verified"], verified_json("alice"));
        assert!(profiles[1].get("verified").is_none());

        let filter = relay.lock().unwrap().queries.last().cloned().unwrap();
        assert_eq!(filter["kinds"], json!([30382]));
        assert_eq!(filter["authors"], json!([authority.public_key().to_hex()]));

        // Compact output keeps the verified object.
        let compact = compact_profile(&profiles[0], &[]);
        assert_eq!(compact["verified"]["username"], "alice");
    }

    #[tokio::test]
    async fn get_by_pubkey_ignores_verified_profiles_the_community_does_not_advertise() {
        let (client, relay) = setup().await;
        let signer = Keys::generate();
        relay.lock().unwrap().info = json!({"self": signer.public_key().to_hex()});
        let alice = Keys::generate();
        seed_profile(&relay, &alice, r#"{"name":"al"}"#);
        seed_verified(&relay, &signer, &alice, "alice", 100);

        let profiles = get_profiles(&client, &[alice.public_key().to_hex()])
            .await
            .unwrap();
        assert!(profiles[0].get("verified").is_none());
        assert!(relay
            .lock()
            .unwrap()
            .queries
            .iter()
            .all(|q| q["kinds"] != json!([30382])));
    }

    #[tokio::test]
    async fn name_search_finds_a_person_by_verified_username() {
        let (client, relay) = setup().await;
        let authority = Keys::generate();
        relay.lock().unwrap().info = advertising(&authority);
        let alice = Keys::generate();
        let bob = Keys::generate();
        // The kind:0 name does not contain the username; the relay matched the
        // verified one and returned both events, plus an impostor's.
        seed_profile(&relay, &alice, r#"{"name":"al","display_name":"Al"}"#);
        seed_verified(&relay, &authority, &alice, "alice", 100);
        seed_verified(&relay, &Keys::generate(), &bob, "alice", 100);
        let events = relay.lock().unwrap().events.clone();
        relay.lock().unwrap().profile_search = Some(json!(events));

        let found = search_profiles(&client, " alice ").await.unwrap();
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!(found[0]["pubkey"], alice.public_key().to_hex());
        assert_eq!(found[0]["name"], "al");
        assert_eq!(found[0]["verified"], verified_json("alice"));
        assert_eq!(
            relay.lock().unwrap().profile_searches,
            vec![json!({"query": "alice", "limit": 100})]
        );
    }

    #[tokio::test]
    async fn name_search_returns_a_person_matched_only_by_verified_name() {
        let (client, relay) = setup().await;
        let authority = Keys::generate();
        relay.lock().unwrap().info = advertising(&authority);
        let alice = Keys::generate();
        seed_verified(&relay, &authority, &alice, "alice", 100);
        let events = relay.lock().unwrap().events.clone();
        relay.lock().unwrap().profile_search = Some(json!(events));

        let found = search_profiles(&client, "alice").await.unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0]["pubkey"], alice.public_key().to_hex());
        assert_eq!(found[0]["verified"]["username"], "alice");
    }

    #[tokio::test]
    async fn name_search_without_advertised_profiles_uses_nip50() {
        let (client, relay) = setup().await;
        let alice = Keys::generate();
        seed_profile(&relay, &alice, r#"{"name":"alice"}"#);
        seed_profile(&relay, &Keys::generate(), r#"{"name":"bob"}"#);

        let found = search_profiles(&client, "alice").await.unwrap();
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!(found[0]["pubkey"], alice.public_key().to_hex());
        assert!(found[0].get("verified").is_none());
        assert!(relay.lock().unwrap().profile_searches.is_empty());
    }

    #[tokio::test]
    async fn get_by_pubkey_keeps_profiles_and_warns_when_the_verified_check_fails() {
        let alice = Keys::generate();
        let pubkeys = vec![alice.public_key().to_hex()];
        // Discovery fails (malformed NIP-11), or the kind:30382 query does.
        for broken_lookup in [false, true] {
            let (client, relay) = setup().await;
            relay.lock().unwrap().info = if broken_lookup {
                advertising(&Keys::generate())
            } else {
                json!({"federated_identity": "malformed"})
            };
            relay.lock().unwrap().broken_kind = broken_lookup.then_some(30382);
            seed_profile(&relay, &alice, r#"{"name":"al"}"#);

            let (profiles, warning) = lookup_profiles(&client, &pubkeys).await.unwrap();
            assert_eq!(profiles.len(), 1, "{broken_lookup}");
            assert_eq!(profiles[0]["name"], "al");
            assert!(profiles[0].get("verified").is_none());
            let warning = warning.expect("a failed check is reported");
            assert_eq!(warning["warning"], "verified_profiles_unavailable");
            assert!(warning["message"].as_str().is_some_and(|m| !m.is_empty()));
        }

        // A working check reports nothing.
        let (client, relay) = setup().await;
        relay.lock().unwrap().info = advertising(&Keys::generate());
        seed_profile(&relay, &alice, r#"{"name":"al"}"#);
        let (_, warning) = lookup_profiles(&client, &pubkeys).await.unwrap();
        assert!(warning.is_none());
    }

    #[tokio::test]
    async fn name_search_keeps_kind0_matches_and_warns_when_the_verified_check_fails() {
        // Discovery fails (malformed NIP-11), or the kind:30382 query does on
        // a relay whose profile search does not match verified names.
        for broken_lookup in [false, true] {
            let (client, relay) = setup().await;
            relay.lock().unwrap().info = if broken_lookup {
                let mut info = advertising(&Keys::generate());
                info["federated_identity"]["public_profile"]["searchable"] = json!(false);
                info
            } else {
                json!({"federated_identity": "malformed"})
            };
            relay.lock().unwrap().broken_kind = broken_lookup.then_some(30382);
            let alice = Keys::generate();
            seed_profile(&relay, &alice, r#"{"name":"alice"}"#);

            let (found, warning) = find_profiles(&client, "alice").await.unwrap();
            assert_eq!(found.len(), 1, "{broken_lookup}: {found:?}");
            assert_eq!(found[0]["pubkey"], alice.public_key().to_hex());
            assert!(found[0].get("verified").is_none());
            let warning = warning.expect("a failed check is reported");
            assert_eq!(warning["warning"], "verified_profiles_unavailable");
            assert!(relay.lock().unwrap().profile_searches.is_empty());
        }

        // A working check reports nothing.
        let (client, relay) = setup().await;
        seed_profile(&relay, &Keys::generate(), r#"{"name":"alice"}"#);
        let (_, warning) = find_profiles(&client, "alice").await.unwrap();
        assert!(warning.is_none());
    }

    #[tokio::test]
    async fn name_search_uses_profile_search_up_to_its_character_limit() {
        // Counted in characters after trimming, as the relay counts them.
        let within = [
            "a".repeat(100),
            "é".repeat(100),
            format!("  {}  ", "a".repeat(100)),
        ];
        for query in within {
            let (client, relay) = setup().await;
            relay.lock().unwrap().info = advertising(&Keys::generate());
            relay.lock().unwrap().profile_search = Some(json!([]));
            search_profiles(&client, &query).await.unwrap();
            let searches = relay.lock().unwrap().profile_searches.clone();
            assert_eq!(searches, vec![json!({"query": query.trim(), "limit": 100})]);
        }

        let (client, relay) = setup().await;
        relay.lock().unwrap().info = advertising(&Keys::generate());
        let long = "é".repeat(101);
        seed_profile(
            &relay,
            &Keys::generate(),
            &json!({"name": long}).to_string(),
        );
        let found = search_profiles(&client, &long).await.unwrap();
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(relay.lock().unwrap().profile_searches.is_empty());
    }

    #[tokio::test]
    async fn get_by_pubkey_returns_people_with_only_a_verified_profile() {
        let me = Keys::generate();
        let (client, relay) = setup_as(me.clone()).await;
        let authority = Keys::generate();
        relay.lock().unwrap().info = advertising(&authority);
        let alice = Keys::generate();
        let bob = Keys::generate();
        seed_profile(&relay, &bob, r#"{"name":"bob"}"#);
        seed_verified(&relay, &authority, &alice, "alice", 100);
        let alice_hex = alice.public_key().to_hex();

        // A single key.
        let profiles = get_profiles(&client, std::slice::from_ref(&alice_hex))
            .await
            .unwrap();
        assert_eq!(profiles.len(), 1, "{profiles:?}");
        assert_eq!(profiles[0]["pubkey"], alice_hex);
        assert_eq!(profiles[0]["verified"], verified_json("alice"));

        // A batch: kind:0 people first, then verified-only ones, once each.
        let batch = vec![
            alice_hex.clone(),
            bob.public_key().to_hex(),
            alice_hex.to_uppercase(),
        ];
        let profiles = get_profiles(&client, &batch).await.unwrap();
        let pubkeys: Vec<&str> = profiles
            .iter()
            .map(|p| p["pubkey"].as_str().unwrap())
            .collect();
        assert_eq!(pubkeys, [bob.public_key().to_hex(), alice_hex.clone()]);

        // The default lookup of your own profile.
        seed_verified(&relay, &authority, &me, "me", 100);
        let profiles = get_profiles(&client, &[]).await.unwrap();
        assert_eq!(profiles.len(), 1, "{profiles:?}");
        assert_eq!(profiles[0]["verified"]["username"], "me");
    }

    #[tokio::test]
    async fn name_search_on_a_relay_without_searchable_profiles_uses_nip50() {
        let (client, relay) = setup().await;
        let authority = Keys::generate();
        let mut info = advertising(&authority);
        info["federated_identity"]["public_profile"]
            .as_object_mut()
            .unwrap()
            .remove("searchable");
        relay.lock().unwrap().info = info;
        let alice = Keys::generate();
        seed_profile(&relay, &alice, r#"{"name":"alice"}"#);
        seed_verified(&relay, &authority, &alice, "alice", 100);
        relay.lock().unwrap().profile_search = Some(json!([]));

        let found = search_profiles(&client, "alice").await.unwrap();
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!(found[0]["pubkey"], alice.public_key().to_hex());
        assert_eq!(found[0]["verified"], verified_json("alice"));
        assert!(relay.lock().unwrap().profile_searches.is_empty());
    }

    const FORGED: &str =
        r#"{"name":"alice","verified":{"username":"forged","issuer_label":"Fake"}}"#;

    #[tokio::test]
    async fn a_verified_object_in_kind0_content_is_never_output() {
        // (a) The community does not advertise verified profiles.
        let (client, relay) = setup().await;
        let alice = Keys::generate();
        seed_profile(&relay, &alice, FORGED);
        let pubkeys = vec![alice.public_key().to_hex()];
        let profiles = get_profiles(&client, &pubkeys).await.unwrap();
        assert_eq!(profiles[0]["name"], "alice");
        assert!(profiles[0].get("verified").is_none());
        let found = search_profiles(&client, "alice").await.unwrap();
        assert_eq!(found.len(), 1);
        assert!(found[0].get("verified").is_none());

        // (b) Advertised, but no valid projection exists for the subject.
        let authority = Keys::generate();
        relay.lock().unwrap().info = advertising(&authority);
        seed_verified(&relay, &Keys::generate(), &alice, "alice", 100);
        let profiles = get_profiles(&client, &pubkeys).await.unwrap();
        assert!(profiles[0].get("verified").is_none());
        let events = relay.lock().unwrap().events.clone();
        relay.lock().unwrap().profile_search = Some(json!(events));
        let found = search_profiles(&client, "alice").await.unwrap();
        assert_eq!(found.len(), 1);
        assert!(found[0].get("verified").is_none());
        let compact = compact_profile(&profiles[0], &[]);
        assert!(compact.get("verified").is_none());

        // (c) A valid projection replaces the forgery.
        seed_verified(&relay, &authority, &alice, "alice", 100);
        let profiles = get_profiles(&client, &pubkeys).await.unwrap();
        assert_eq!(profiles[0]["verified"], verified_json("alice"));
        let events = relay.lock().unwrap().events.clone();
        relay.lock().unwrap().profile_search = Some(json!(events));
        let found = search_profiles(&client, "alice").await.unwrap();
        assert_eq!(found[0]["verified"], verified_json("alice"));

        // The owner-scoped path takes kind:0 content too.
        let event = json!({"pubkey": "aa", "content": FORGED, "tags": []});
        let owned = owner_scoped_profiles(&[event], &["aa".to_string()], "bb", "bb");
        assert!(owned[0].get("verified").is_none());
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
