use std::collections::HashMap;

use buzz_core_pkg::{kind::KIND_USER_TRUSTED_ASSERTION, PresenceStatus};
use serde_json::Value;
use tauri::State;

use crate::{
    app_state::AppState,
    commands::identity_archive::fetch_relay_self,
    events,
    managed_agents::persona_events::monotonic_created_at,
    models::{ProfileInfo, SearchUsersResponse, UserNotesResponse, UsersBatchResponse},
    nostr_convert,
    relay::{
        query_relay, query_relay_at_with_keys, relay_http_base_url, submit_event,
        submit_event_at_with_keys,
    },
};

async fn query_profiles_with_assertions(
    state: &AppState,
    pubkeys: &[String],
) -> Result<(Vec<nostr::Event>, Option<String>), String> {
    if pubkeys.is_empty() {
        return Ok((Vec::new(), None));
    }

    let relay_self = fetch_relay_self(state).await.unwrap_or(None);
    let mut filters = vec![serde_json::json!({
        "kinds": [0],
        "authors": pubkeys,
    })];
    if let Some(author) = relay_self.as_ref() {
        filters.push(serde_json::json!({
            "kinds": [KIND_USER_TRUSTED_ASSERTION],
            "authors": [author],
            "#d": pubkeys,
        }));
    }
    Ok((query_relay(state, &filters).await?, relay_self))
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct VerifiedIdentity {
    display_name: String,
    expires_at: u64,
}

fn verified_identities(
    events: &[nostr::Event],
    relay_self: Option<&str>,
) -> HashMap<String, VerifiedIdentity> {
    let Some(relay_self) = relay_self else {
        return HashMap::new();
    };
    let mut verified = HashMap::<String, (u64, String, Option<VerifiedIdentity>)>::new();
    let now = nostr::Timestamp::now().as_secs();
    for event in events {
        if event.kind.as_u16() as u32 != KIND_USER_TRUSTED_ASSERTION
            || !event.pubkey.to_hex().eq_ignore_ascii_case(relay_self)
            || !event.verify_id()
            || !event.verify_signature()
        {
            continue;
        }
        // The coordinate is recoverable even when the newer payload is
        // malformed (for example, an overlong `d` tag). That malformed head
        // must suppress the prior assertion rather than being skipped.
        let Some(subject) = event.tags.iter().find_map(|tag| {
            let parts = tag.as_slice();
            (parts.first().is_some_and(|part| part == "d"))
                .then(|| parts.get(1).map(|part| part.as_str()))
                .flatten()
        }) else {
            continue;
        };
        if subject.len() != 64 || !subject.chars().all(|value| value.is_ascii_hexdigit()) {
            continue;
        }
        let tag_value = |name: &str| {
            let mut matches = event.tags.iter().filter(|tag| {
                let parts = tag.as_slice();
                parts.first().is_some_and(|part| part == name)
            });
            let tag = matches.next()?;
            if matches.next().is_some() {
                return None;
            }
            let parts = tag.as_slice();
            (parts.len() == 2).then(|| parts[1].as_str())
        };
        // Select the signed replaceable-event head before validating its
        // payload. Otherwise a newer malformed assertion could be skipped and
        // silently resurrect the older active label returned alongside it.
        let identity = match (tag_value("d"), tag_value("verified"), tag_value("p")) {
            (Some(assertion_d), Some("relay"), Some(asserted_subject))
                if assertion_d == subject && asserted_subject == subject =>
            {
                match tag_value("active") {
                    Some("false") => None,
                    Some("true") => match (
                        tag_value("expiration")
                            .and_then(|value| value.parse::<u64>().ok())
                            .filter(|expiration| *expiration > now),
                        tag_value("display_name")
                            .map(str::trim)
                            .filter(|value| !value.is_empty()),
                    ) {
                        (Some(expires_at), Some(display_name)) => Some(VerifiedIdentity {
                            display_name: display_name.to_string(),
                            expires_at,
                        }),
                        _ => None,
                    },
                    _ => None,
                }
            }
            _ => None,
        };
        let created_at = event.created_at.as_secs();
        let event_id = event.id.to_hex();
        // NIP-01 replaceable-event ordering: greatest timestamp wins; equal
        // timestamps are resolved by the lowest event id. This stays stable
        // regardless of relay response order.
        match verified.entry(subject.to_ascii_lowercase()) {
            std::collections::hash_map::Entry::Vacant(entry) => {
                entry.insert((created_at, event_id, identity));
            }
            std::collections::hash_map::Entry::Occupied(mut entry)
                if created_at > entry.get().0
                    || (created_at == entry.get().0 && event_id < entry.get().1) =>
            {
                entry.insert((created_at, event_id, identity));
            }
            std::collections::hash_map::Entry::Occupied(_) => {}
        }
    }
    verified
        .into_iter()
        .filter_map(|(pubkey, (_, _, identity))| identity.map(|value| (pubkey, value)))
        .collect()
}

fn apply_verified_identity(profile: &mut ProfileInfo, identity: Option<VerifiedIdentity>) {
    profile.verified_name = identity
        .as_ref()
        .map(|value| value.display_name.to_string());
    profile.verified_name_expires_at = identity.map(|value| value.expires_at);
}

#[tauri::command]
pub async fn get_profile(state: State<'_, AppState>) -> Result<ProfileInfo, String> {
    let my_pubkey = current_pubkey_hex(&state)?;
    let (events, relay_self) =
        query_profiles_with_assertions(&state, std::slice::from_ref(&my_pubkey)).await?;

    let mut profile = events
        .iter()
        .find(|event| event.kind.as_u16() == 0 && event.pubkey.to_hex() == my_pubkey)
        .map(nostr_convert::profile_info_from_event)
        .transpose()?
        .unwrap_or_else(|| empty_profile_info(&current_pubkey_hex_unwrap(&state)));
    let identity = verified_identities(&events, relay_self.as_deref()).remove(&profile.pubkey);
    apply_verified_identity(&mut profile, identity);
    Ok(profile)
}

#[tauri::command]
pub async fn update_profile(
    display_name: Option<String>,
    avatar_url: Option<String>,
    about: Option<String>,
    nip05_handle: Option<String>,
    state: State<'_, AppState>,
) -> Result<ProfileInfo, String> {
    // Read-merge-write: kind 0 is a full profile snapshot.
    let my_pubkey = current_pubkey_hex(&state)?;
    let prior_events = query_relay(
        &state,
        &[serde_json::json!({
            "kinds": [0],
            "authors": [my_pubkey],
            "limit": 1
        })],
    )
    .await?;

    // Pull the current content as a JSON object so we can merge with
    // the caller's overrides.
    let current: Value = prior_events
        .first()
        .and_then(|ev| serde_json::from_str::<Value>(&ev.content).ok())
        .unwrap_or(Value::Null);

    let dn = display_name
        .as_deref()
        .or_else(|| current.get("display_name").and_then(Value::as_str));
    let name = current.get("name").and_then(Value::as_str);
    let picture = avatar_url
        .as_deref()
        .or_else(|| current.get("picture").and_then(Value::as_str));
    let ab = about
        .as_deref()
        .or_else(|| current.get("about").and_then(Value::as_str));
    let nip05 = nip05_handle
        .as_deref()
        .or_else(|| current.get("nip05").and_then(Value::as_str));

    let builder = events::build_profile(dn, name, picture, ab, nip05)?;
    submit_event(builder, &state).await?;

    // Re-fetch to return canonical profile.
    let events = query_relay(
        &state,
        &[serde_json::json!({
            "kinds": [0],
            "authors": [current_pubkey_hex(&state)?],
            "limit": 1
        })],
    )
    .await?;

    Ok(events
        .first()
        .map(nostr_convert::profile_info_from_event)
        .transpose()?
        .unwrap_or_else(|| empty_profile_info(&current_pubkey_hex_unwrap(&state))))
}

#[tauri::command]
pub async fn update_profile_at_relay(
    relay_url: String,
    expected_pubkey: String,
    expected_avatar_url: Option<String>,
    avatar_url: String,
    state: State<'_, AppState>,
) -> Result<ProfileInfo, String> {
    let signer = capture_expected_signer(&state, &expected_pubkey)?;

    let api_base_url = relay_http_base_url(&relay_url);
    let filter = serde_json::json!({
        "kinds": [0],
        "authors": [expected_pubkey],
        "limit": 1
    });
    let prior_events = query_relay_at_with_keys(
        &state,
        &api_base_url,
        std::slice::from_ref(&filter),
        &signer,
        None,
    )
    .await?;
    let prior_event = prior_events.first();
    let current: Value = prior_event
        .and_then(|event| serde_json::from_str::<Value>(&event.content).ok())
        .unwrap_or(Value::Null);
    let current_avatar_url = current
        .get("picture")
        .and_then(Value::as_str)
        .map(str::to_string);
    if normalized_avatar_url(current_avatar_url.as_deref())
        != normalized_avatar_url(expected_avatar_url.as_deref())
    {
        return Err("profile avatar changed before deferred save".to_string());
    }

    let builder = build_deferred_profile_event(&current, &avatar_url, prior_event)?;
    submit_event_at_with_keys(builder, &state, &api_base_url, &signer).await?;

    let events = query_relay_at_with_keys(&state, &api_base_url, &[filter], &signer, None).await?;
    Ok(events
        .first()
        .map(nostr_convert::profile_info_from_event)
        .transpose()?
        .unwrap_or_else(|| empty_profile_info(&expected_pubkey)))
}

fn build_deferred_profile_event(
    current: &Value,
    avatar_url: &str,
    prior_event: Option<&nostr::Event>,
) -> Result<nostr::EventBuilder, String> {
    let display_name = current.get("display_name").and_then(Value::as_str);
    let name = current.get("name").and_then(Value::as_str);
    let about = current.get("about").and_then(Value::as_str);
    let nip05 = current.get("nip05").and_then(Value::as_str);

    Ok(
        events::build_profile(display_name, name, Some(avatar_url), about, nip05)?
            .custom_created_at(monotonic_created_at(
                prior_event.map(|event| event.created_at.as_secs() as i64),
            )),
    )
}

fn capture_expected_signer(state: &AppState, expected_pubkey: &str) -> Result<nostr::Keys, String> {
    let signer = state.signing_keys()?;
    if signer.public_key().to_hex() != expected_pubkey {
        return Err("profile identity changed before avatar save".to_string());
    }
    Ok(signer)
}

fn normalized_avatar_url(avatar_url: Option<&str>) -> Option<&str> {
    avatar_url.map(str::trim).filter(|value| !value.is_empty())
}

#[tauri::command]
pub async fn get_user_profile(
    pubkey: Option<String>,
    state: State<'_, AppState>,
) -> Result<ProfileInfo, String> {
    let target = match pubkey {
        Some(pk) => pk,
        None => current_pubkey_hex(&state)?,
    };

    let (events, relay_self) =
        query_profiles_with_assertions(&state, std::slice::from_ref(&target)).await?;

    let mut profile = events
        .iter()
        .find(|event| event.kind.as_u16() == 0 && event.pubkey.to_hex() == target)
        .map(nostr_convert::profile_info_from_event)
        .transpose()?
        .unwrap_or_else(|| empty_profile_info(&target));
    let identity = verified_identities(&events, relay_self.as_deref()).remove(&profile.pubkey);
    apply_verified_identity(&mut profile, identity);
    Ok(profile)
}

#[tauri::command]
pub async fn get_users_batch(
    pubkeys: Vec<String>,
    state: State<'_, AppState>,
) -> Result<UsersBatchResponse, String> {
    if pubkeys.is_empty() {
        return Ok(UsersBatchResponse {
            profiles: HashMap::new(),
            missing: Vec::new(),
        });
    }
    let (events, relay_self) = query_profiles_with_assertions(&state, &pubkeys).await?;

    let mut response = nostr_convert::users_batch_from_events(&events, &pubkeys);
    let verified = verified_identities(&events, relay_self.as_deref());
    for (pubkey, profile) in &mut response.profiles {
        if let Some(identity) = verified.get(pubkey) {
            profile.verified_name = Some(identity.display_name.to_string());
            profile.verified_name_expires_at = Some(identity.expires_at);
        }
    }
    Ok(response)
}

#[tauri::command]
pub async fn get_user_notes(
    pubkey: String,
    limit: Option<u32>,
    before: Option<i64>,
    before_id: Option<String>,
    state: State<'_, AppState>,
) -> Result<UserNotesResponse, String> {
    let _ = before_id; // pure-nostr filter does not use the id-based cursor
    let mut filter = serde_json::Map::new();
    filter.insert("kinds".to_string(), serde_json::json!([1]));
    filter.insert("authors".to_string(), serde_json::json!([pubkey]));
    filter.insert(
        "limit".to_string(),
        serde_json::json!(limit.unwrap_or(20).min(100)),
    );
    if let Some(t) = before {
        filter.insert("until".to_string(), serde_json::json!(t));
    }

    let events = query_relay(&state, &[Value::Object(filter)]).await?;
    Ok(nostr_convert::user_notes_from_events(&events))
}

fn build_user_search_filter(query: &str, limit: usize, page: u32) -> serde_json::Value {
    serde_json::json!({
        "kinds": [0],
        "search": query,
        "search_mode": "prefix",
        "limit": limit,
        "page": page,
    })
}

#[tauri::command]
pub async fn search_users(
    query: String,
    limit: Option<u32>,
    cursor: Option<String>,
    state: State<'_, AppState>,
) -> Result<SearchUsersResponse, String> {
    let trimmed = query.trim();
    let max = limit.unwrap_or(8).min(500) as usize;
    let page = cursor
        .as_deref()
        .and_then(|value| value.parse::<u32>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(1);

    if max == 0 {
        return Ok(SearchUsersResponse {
            users: Vec::new(),
            next_cursor: None,
        });
    }

    if trimmed.is_empty() {
        let events = query_relay(
            &state,
            &[serde_json::json!({
                "kinds": [0],
                "limit": max,
                "page": page,
            })],
        )
        .await?;

        // Emit a real next page cursor when the relay returned a full page, so
        // the empty-query people directory can page past its first page (the
        // relay honors `page`→offset for this non-search kind:0 listing). The
        // raw `events.len()` is the correct fullness signal — `list_user_search_results`
        // dedupes/truncates, so its output length can undercount a full page.
        let mut response = nostr_convert::list_user_search_results(&events, max);
        if events.len() >= max {
            response.next_cursor = Some((page + 1).to_string());
        }
        return Ok(response);
    }

    // NIP-50 full-text search on kind:0 profiles. The relay's HTTP bridge
    // intercepts the `search` field on POST /query and routes to Postgres FTS
    // (see `crates/buzz-relay/src/api/bridge.rs::handle_bridge_search`),
    // so we get indexed, server-side search instead of fetching every kind:0
    // and scanning client-side. The old path was capped at 2000 kind:0 events
    // by the relay's HTTP bridge limit, which silently hid users on busy relays.
    //
    // We fetch one bounded page (bridge accepts up to 500) and re-rank that page
    // locally because the relay scores FTS rank against the whole kind:0 JSON
    // `content` blob, where a hit in `display_name` is not weighted any higher
    // than a substring hit in `about`. The caller can request later pages via the
    // cursor so the UI cap is only a page size, not a terminal directory ceiling.
    //
    // `search_mode: "prefix"` matters: every caller of this command is a
    // typeahead surface (member picker, @mention popup, DM recipient search,
    // topbar people results), so a partially typed name must match. Without it
    // the relay runs whole-word `websearch_to_tsquery` matching and "tyl"
    // returns zero results for "Tyler". Same bridge-only extension the topbar
    // message search uses (see `build_search_messages_filter`).
    let events = query_relay(&state, &[build_user_search_filter(trimmed, max, page)]).await?;

    let mut response = nostr_convert::rank_user_search_results(&events, trimmed, max);
    if events.len() >= max {
        response.next_cursor = Some((page + 1).to_string());
    }
    Ok(response)
}

#[tauri::command]
pub async fn get_presence(
    pubkeys: Vec<String>,
    state: State<'_, AppState>,
) -> Result<HashMap<String, PresenceStatus>, String> {
    if pubkeys.is_empty() {
        return Ok(HashMap::new());
    }

    // Presence is published as kind:20001 ephemeral events. Query the most
    // recent per author. Some relays don't retain ephemeral events — we
    // best-effort and return what we get.
    let events = query_relay(
        &state,
        &[serde_json::json!({
            "kinds": [20001],
            "authors": pubkeys,
        })],
    )
    .await
    .unwrap_or_default();

    let mut latest: HashMap<String, (u64, PresenceStatus)> = HashMap::new();
    for ev in &events {
        // Relay-synthesized presence events use a p-tag to identify the subject.
        // Self-signed presence events (live WS) use the event author directly.
        let pk = ev
            .tags
            .iter()
            .find_map(|t| {
                let s = t.as_slice();
                if s.len() >= 2 && s[0] == "p" {
                    Some(s[1].clone())
                } else {
                    None
                }
            })
            .unwrap_or_else(|| ev.pubkey.to_hex());
        let ts = ev.created_at.as_secs();
        let status = match ev.content.trim() {
            "online" => PresenceStatus::Online,
            "away" => PresenceStatus::Away,
            "offline" => PresenceStatus::Offline,
            _ => continue,
        };
        match latest.get(&pk) {
            Some((prev_ts, _)) if *prev_ts >= ts => {}
            _ => {
                latest.insert(pk, (ts, status));
            }
        }
    }

    Ok(latest
        .into_iter()
        .map(|(pk, (_, status))| (pk, status))
        .collect())
}

fn current_pubkey_hex(state: &AppState) -> Result<String, String> {
    let keys = state.keys.lock().map_err(|e| e.to_string())?;
    Ok(keys.public_key().to_hex())
}

fn current_pubkey_hex_unwrap(state: &AppState) -> String {
    current_pubkey_hex(state).unwrap_or_default()
}

fn empty_profile_info(pubkey: &str) -> ProfileInfo {
    ProfileInfo {
        pubkey: pubkey.to_string(),
        display_name: None,
        verified_name: None,
        verified_name_expires_at: None,
        avatar_url: None,
        about: None,
        nip05_handle: None,
        owner_pubkey: None,
        has_profile_event: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verified_identity_requires_relay_signed_nip85_assertion() {
        let relay = nostr::Keys::generate();
        let subject = nostr::Keys::generate().public_key().to_hex();
        let expires_at = nostr::Timestamp::now().as_secs() + 60;
        let event =
            nostr::EventBuilder::new(nostr::Kind::Custom(KIND_USER_TRUSTED_ASSERTION as u16), "")
                .tags([
                    nostr::Tag::parse(["d", subject.as_str()]).unwrap(),
                    nostr::Tag::parse(["p", subject.as_str()]).unwrap(),
                    nostr::Tag::parse(["verified", "relay"]).unwrap(),
                    nostr::Tag::parse(["active", "true"]).unwrap(),
                    nostr::Tag::parse(["expiration", &expires_at.to_string()]).unwrap(),
                    nostr::Tag::parse(["display_name", "Example User"]).unwrap(),
                ])
                .sign_with_keys(&relay)
                .unwrap();

        let verified = verified_identities(&[event], Some(&relay.public_key().to_hex()));
        assert_eq!(
            verified.get(&subject),
            Some(&VerifiedIdentity {
                display_name: "Example User".to_string(),
                expires_at,
            })
        );
    }

    #[test]
    fn expired_verified_identity_is_rejected() {
        let relay = nostr::Keys::generate();
        let subject = nostr::Keys::generate().public_key().to_hex();
        let created_at = nostr::Timestamp::now().as_secs();
        let expires_at = nostr::Timestamp::now().as_secs().saturating_sub(1);
        let prior_expiration = created_at + 120;
        let prior =
            nostr::EventBuilder::new(nostr::Kind::Custom(KIND_USER_TRUSTED_ASSERTION as u16), "")
                .tags([
                    nostr::Tag::parse(["d", subject.as_str()]).unwrap(),
                    nostr::Tag::parse(["p", subject.as_str()]).unwrap(),
                    nostr::Tag::parse(["verified", "relay"]).unwrap(),
                    nostr::Tag::parse(["active", "true"]).unwrap(),
                    nostr::Tag::parse(["expiration", &prior_expiration.to_string()]).unwrap(),
                    nostr::Tag::parse(["display_name", "Prior User"]).unwrap(),
                ])
                .custom_created_at(nostr::Timestamp::from(created_at))
                .sign_with_keys(&relay)
                .unwrap();
        let expired =
            nostr::EventBuilder::new(nostr::Kind::Custom(KIND_USER_TRUSTED_ASSERTION as u16), "")
                .tags([
                    nostr::Tag::parse(["d", subject.as_str()]).unwrap(),
                    nostr::Tag::parse(["p", subject.as_str()]).unwrap(),
                    nostr::Tag::parse(["verified", "relay"]).unwrap(),
                    nostr::Tag::parse(["active", "true"]).unwrap(),
                    nostr::Tag::parse(["expiration", &expires_at.to_string()]).unwrap(),
                    nostr::Tag::parse(["display_name", "Expired User"]).unwrap(),
                ])
                .custom_created_at(nostr::Timestamp::from(created_at + 1))
                .sign_with_keys(&relay)
                .unwrap();

        assert!(
            verified_identities(&[prior, expired], Some(&relay.public_key().to_hex())).is_empty()
        );
    }

    #[test]
    fn newer_inactive_assertion_removes_verified_identity() {
        let relay = nostr::Keys::generate();
        let subject = nostr::Keys::generate().public_key().to_hex();
        let created_at = nostr::Timestamp::now().as_secs();
        let expires_at = created_at + 60;
        let active =
            nostr::EventBuilder::new(nostr::Kind::Custom(KIND_USER_TRUSTED_ASSERTION as u16), "")
                .tags([
                    nostr::Tag::parse(["d", subject.as_str()]).unwrap(),
                    nostr::Tag::parse(["p", subject.as_str()]).unwrap(),
                    nostr::Tag::parse(["verified", "relay"]).unwrap(),
                    nostr::Tag::parse(["active", "true"]).unwrap(),
                    nostr::Tag::parse(["expiration", &expires_at.to_string()]).unwrap(),
                    nostr::Tag::parse(["display_name", "Example User"]).unwrap(),
                ])
                .custom_created_at(nostr::Timestamp::from(created_at))
                .sign_with_keys(&relay)
                .unwrap();
        let inactive =
            nostr::EventBuilder::new(nostr::Kind::Custom(KIND_USER_TRUSTED_ASSERTION as u16), "")
                .tags([
                    nostr::Tag::parse(["d", subject.as_str()]).unwrap(),
                    nostr::Tag::parse(["p", subject.as_str()]).unwrap(),
                    nostr::Tag::parse(["verified", "relay"]).unwrap(),
                    nostr::Tag::parse(["active", "false"]).unwrap(),
                ])
                .custom_created_at(nostr::Timestamp::from(created_at + 1))
                .sign_with_keys(&relay)
                .unwrap();

        assert!(
            verified_identities(&[active, inactive], Some(&relay.public_key().to_hex())).is_empty()
        );
    }

    #[test]
    fn newer_malformed_assertion_does_not_resurrect_older_identity() {
        let relay = nostr::Keys::generate();
        let subject = nostr::Keys::generate().public_key().to_hex();
        let wrong_subject = nostr::Keys::generate().public_key().to_hex();
        let created_at = nostr::Timestamp::now().as_secs();
        let expires_at = created_at + 60;
        let active =
            nostr::EventBuilder::new(nostr::Kind::Custom(KIND_USER_TRUSTED_ASSERTION as u16), "")
                .tags([
                    nostr::Tag::parse(["d", subject.as_str()]).unwrap(),
                    nostr::Tag::parse(["p", subject.as_str()]).unwrap(),
                    nostr::Tag::parse(["verified", "relay"]).unwrap(),
                    nostr::Tag::parse(["active", "true"]).unwrap(),
                    nostr::Tag::parse(["expiration", &expires_at.to_string()]).unwrap(),
                    nostr::Tag::parse(["display_name", "Example User"]).unwrap(),
                ])
                .custom_created_at(nostr::Timestamp::from(created_at))
                .sign_with_keys(&relay)
                .unwrap();
        let malformed =
            nostr::EventBuilder::new(nostr::Kind::Custom(KIND_USER_TRUSTED_ASSERTION as u16), "")
                .tags([
                    nostr::Tag::parse(["d", subject.as_str()]).unwrap(),
                    nostr::Tag::parse(["p", wrong_subject.as_str()]).unwrap(),
                    nostr::Tag::parse(["verified", "relay"]).unwrap(),
                    nostr::Tag::parse(["active", "true"]).unwrap(),
                    nostr::Tag::parse(["expiration", &expires_at.to_string()]).unwrap(),
                    nostr::Tag::parse(["display_name", "Malformed User"]).unwrap(),
                ])
                .custom_created_at(nostr::Timestamp::from(created_at + 1))
                .sign_with_keys(&relay)
                .unwrap();

        assert!(verified_identities(
            &[active.clone(), malformed],
            Some(&relay.public_key().to_hex())
        )
        .is_empty());

        let overlong_d =
            nostr::EventBuilder::new(nostr::Kind::Custom(KIND_USER_TRUSTED_ASSERTION as u16), "")
                .tags([
                    nostr::Tag::parse(["d", subject.as_str(), "unexpected"]).unwrap(),
                    nostr::Tag::parse(["p", subject.as_str()]).unwrap(),
                    nostr::Tag::parse(["verified", "relay"]).unwrap(),
                    nostr::Tag::parse(["active", "true"]).unwrap(),
                    nostr::Tag::parse(["expiration", &expires_at.to_string()]).unwrap(),
                    nostr::Tag::parse(["display_name", "Malformed User"]).unwrap(),
                ])
                .custom_created_at(nostr::Timestamp::from(created_at + 1))
                .sign_with_keys(&relay)
                .unwrap();
        assert!(
            verified_identities(&[active, overlong_d], Some(&relay.public_key().to_hex()))
                .is_empty()
        );
    }

    #[test]
    fn equal_timestamp_assertions_use_lowest_event_id_independent_of_response_order() {
        let relay = nostr::Keys::generate();
        let subject = nostr::Keys::generate().public_key().to_hex();
        let created_at = nostr::Timestamp::now().as_secs();
        let expires_at = created_at + 60;
        let active =
            nostr::EventBuilder::new(nostr::Kind::Custom(KIND_USER_TRUSTED_ASSERTION as u16), "")
                .tags([
                    nostr::Tag::parse(["d", subject.as_str()]).unwrap(),
                    nostr::Tag::parse(["p", subject.as_str()]).unwrap(),
                    nostr::Tag::parse(["verified", "relay"]).unwrap(),
                    nostr::Tag::parse(["active", "true"]).unwrap(),
                    nostr::Tag::parse(["expiration", &expires_at.to_string()]).unwrap(),
                    nostr::Tag::parse(["display_name", "Example User"]).unwrap(),
                ])
                .custom_created_at(nostr::Timestamp::from(created_at))
                .sign_with_keys(&relay)
                .unwrap();
        let inactive =
            nostr::EventBuilder::new(nostr::Kind::Custom(KIND_USER_TRUSTED_ASSERTION as u16), "")
                .tags([
                    nostr::Tag::parse(["d", subject.as_str()]).unwrap(),
                    nostr::Tag::parse(["p", subject.as_str()]).unwrap(),
                    nostr::Tag::parse(["verified", "relay"]).unwrap(),
                    nostr::Tag::parse(["active", "false"]).unwrap(),
                ])
                .custom_created_at(nostr::Timestamp::from(created_at))
                .sign_with_keys(&relay)
                .unwrap();
        let relay_pubkey = relay.public_key().to_hex();
        let expected_active = active.id.to_hex() < inactive.id.to_hex();

        for events in [
            vec![active.clone(), inactive.clone()],
            vec![inactive.clone(), active.clone()],
        ] {
            let actual = verified_identities(&events, Some(&relay_pubkey));
            assert_eq!(actual.contains_key(&subject), expected_active);
        }
    }

    #[test]
    fn deferred_profile_signer_is_captured_and_rejects_wrong_identity() {
        let state = crate::app_state::build_app_state();
        let original = state.signing_keys().expect("signable identity");
        let original_pubkey = original.public_key().to_hex();

        let captured = capture_expected_signer(&state, &original_pubkey)
            .expect("matching identity should be captured");
        *state.keys.lock().expect("lock keys") = nostr::Keys::generate();

        assert_eq!(captured.public_key().to_hex(), original_pubkey);
        assert_ne!(
            state.keys.lock().expect("lock keys").public_key().to_hex(),
            original_pubkey
        );
        assert_eq!(
            capture_expected_signer(&state, &original_pubkey).unwrap_err(),
            "profile identity changed before avatar save"
        );
    }

    #[test]
    fn deferred_profile_event_is_strictly_newer_than_prior_head() {
        let keys = nostr::Keys::generate();
        let prior_created_at = nostr::Timestamp::now().as_secs() + 60;
        let prior_event = nostr::EventBuilder::new(
            nostr::Kind::Metadata,
            serde_json::json!({"display_name": "Larry"}).to_string(),
        )
        .custom_created_at(nostr::Timestamp::from(prior_created_at))
        .sign_with_keys(&keys)
        .expect("sign prior profile");

        let builder = build_deferred_profile_event(
            &serde_json::json!({"display_name": "Larry"}),
            "https://example.com/avatar.png",
            Some(&prior_event),
        )
        .expect("build deferred profile");
        let event = builder
            .sign_with_keys(&keys)
            .expect("sign deferred profile");

        assert_eq!(event.created_at.as_secs(), prior_created_at + 1);
        assert_eq!(
            serde_json::from_str::<Value>(&event.content).unwrap()["picture"],
            "https://example.com/avatar.png"
        );
    }

    #[test]
    fn user_search_filter_requests_prefix_mode_for_typeahead() {
        // Every caller of `search_users` is a typeahead surface. Whole-word
        // FTS matching returns zero results for a partially typed name
        // ("tyl" for "Tyler"), which reads as "user doesn't exist" in the
        // member picker and @mention popup. Pin the mode so it can't drift.
        let filter = build_user_search_filter("tyl", 25, 1);

        assert_eq!(filter["search"], serde_json::json!("tyl"));
        assert_eq!(filter["search_mode"], serde_json::json!("prefix"));
        assert_eq!(filter["limit"], serde_json::json!(25));
        assert_eq!(filter["page"], serde_json::json!(1));
    }
}
