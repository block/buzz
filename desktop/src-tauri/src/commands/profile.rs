use std::collections::HashMap;

use buzz_core_pkg::PresenceStatus;
use serde_json::Value;
use tauri::State;

use crate::{
    app_state::AppState,
    events,
    managed_agents::persona_events::monotonic_created_at,
    models::{ProfileInfo, SearchUsersResponse, UserNotesResponse, UsersBatchResponse},
    nostr_convert,
    relay::{
        query_relay, query_relay_at_with_keys, relay_http_base_url, submit_event,
        submit_event_at_with_keys,
    },
};

#[tauri::command]
pub async fn get_profile(state: State<'_, AppState>) -> Result<ProfileInfo, String> {
    let my_pubkey = current_pubkey_hex(&state)?;
    let events = query_relay(
        &state,
        &[serde_json::json!({
            "kinds": [0],
            "authors": [my_pubkey],
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
    let query_lease = crate::owner_identity_egress::EgressLease::OwnerIdentity(
        crate::owner_identity_egress::try_admit_owner_identity_egress().await?,
    );
    let prior_events = query_relay_at_with_keys(
        &state,
        &api_base_url,
        std::slice::from_ref(&filter),
        &signer,
        None,
        &query_lease,
    )
    .await?;
    drop(query_lease);
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

    // Admit BEFORE building the event: `build_deferred_profile_event` stamps
    // `created_at` at build time, so admission (which waits out the rate-limit
    // gate) must precede it to keep the kind:0 event fresh under a gate hold.
    let submit_lease = crate::owner_identity_egress::EgressLease::OwnerIdentity(
        crate::owner_identity_egress::try_admit_owner_identity_egress().await?,
    );
    let builder = build_deferred_profile_event(&current, &avatar_url, prior_event)?;
    submit_event_at_with_keys(builder, &state, &api_base_url, &signer, &submit_lease).await?;
    drop(submit_lease);

    let requery_lease = crate::owner_identity_egress::EgressLease::OwnerIdentity(
        crate::owner_identity_egress::try_admit_owner_identity_egress().await?,
    );
    let events = query_relay_at_with_keys(
        &state,
        &api_base_url,
        &[filter],
        &signer,
        None,
        &requery_lease,
    )
    .await?;
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

    let events = query_relay(
        &state,
        &[serde_json::json!({
            "kinds": [0],
            "authors": [target.clone()],
            "limit": 1
        })],
    )
    .await?;

    Ok(events
        .first()
        .map(nostr_convert::profile_info_from_event)
        .transpose()?
        .unwrap_or_else(|| empty_profile_info(&target)))
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
    let events = query_relay(
        &state,
        &[serde_json::json!({
            "kinds": [0],
            "authors": pubkeys,
        })],
    )
    .await?;

    Ok(nostr_convert::users_batch_from_events(&events, &pubkeys))
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
    Ok(state.current_pubkey()?.to_hex())
}

fn current_pubkey_hex_unwrap(state: &AppState) -> String {
    current_pubkey_hex(state).unwrap_or_default()
}

fn empty_profile_info(pubkey: &str) -> ProfileInfo {
    ProfileInfo {
        pubkey: pubkey.to_string(),
        display_name: None,
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
    fn deferred_profile_signer_is_captured_and_rejects_wrong_identity() {
        let state = crate::app_state::build_app_state();
        let original = state.signing_keys().expect("signable identity");
        let original_pubkey = original.public_key().to_hex();

        let captured = capture_expected_signer(&state, &original_pubkey)
            .expect("matching identity should be captured");
        *state.identity_lifecycle_keys_guard().expect("lock keys") = nostr::Keys::generate();

        assert_eq!(captured.public_key().to_hex(), original_pubkey);
        assert_ne!(
            state.current_pubkey().expect("pubkey").to_hex(),
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

    /// End-to-end drive of `update_profile_at_relay`'s three-lease path
    /// (query → submit → re-query) against a per-path counting loopback relay.
    ///
    /// Proves the P29-C1 posture Paul gated C1 close on: the deferred profile
    /// save admits exactly one owner-identity egress lease per network op and
    /// submits the kind:0 event exactly once. It also exercises the C1c
    /// accessor sweep on the live path — `capture_expected_signer` reads the
    /// identity through `signing_keys()` (the latch-gated accessor), so a
    /// regression that broke the accessor or the funnel witness threading would
    /// fail here rather than only in unit isolation.
    #[tokio::test]
    async fn update_profile_at_relay_submits_exactly_once_across_three_leases() {
        use std::io::{Read, Write};
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Arc;

        let _serial = crate::relay_admission::TEST_SERIAL.lock().await;
        crate::relay_admission::reset_rate_limit_gate();

        let query_count = Arc::new(AtomicUsize::new(0));
        let submit_count = Arc::new(AtomicUsize::new(0));
        let (qc, sc) = (Arc::clone(&query_count), Arc::clone(&submit_count));

        // Loopback relay: the `/query` path answers `[]` (empty profile
        // history), any other path (the submit endpoint) answers an accepted
        // `SubmitEventResponse`. Both are counted so the test can assert the
        // exactly-once submit + two queries of the deferred save flow.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            for _ in 0..3 {
                let Ok((mut stream, _)) = listener.accept() else {
                    break;
                };
                let mut buf = [0u8; 8192];
                let n = stream.read(&mut buf).unwrap_or(0);
                let request = String::from_utf8_lossy(&buf[..n]);
                let request_line = request.lines().next().unwrap_or("");
                let body = if request_line.contains("/query") {
                    qc.fetch_add(1, Ordering::SeqCst);
                    "[]".to_string()
                } else {
                    sc.fetch_add(1, Ordering::SeqCst);
                    r#"{"event_id":"deadbeef","accepted":true,"message":"ok"}"#.to_string()
                };
                let _ = stream.write_all(
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    )
                    .as_bytes(),
                );
                let _ = stream.flush();
            }
        });

        let app = tauri::test::mock_builder()
            .manage(crate::app_state::build_app_state())
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .expect("mock app");
        use tauri::Manager;
        let state = app.state::<crate::app_state::AppState>();
        let expected_pubkey = state.signing_keys().unwrap().public_key().to_hex();

        let result = update_profile_at_relay(
            format!("http://{addr}"),
            expected_pubkey.clone(),
            None,
            "https://example.com/avatar.png".to_string(),
            state,
        )
        .await
        .expect("deferred profile save must complete");

        // Empty re-query returns the empty canonical profile for the identity.
        assert_eq!(result.pubkey, expected_pubkey);
        // Exactly-once posture: one prior-query, one submit, one re-query.
        assert_eq!(
            submit_count.load(Ordering::SeqCst),
            1,
            "kind:0 submitted once"
        );
        assert_eq!(
            query_count.load(Ordering::SeqCst),
            2,
            "prior + re-query only"
        );

        server.join().unwrap();
        crate::relay_admission::reset_rate_limit_gate();
    }
}
