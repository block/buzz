//! Native owner/host validation and ordinary Stop; no keys cross IPC.
use super::desktop_profiles::{prepare, scope};
use crate::{
    app_state::AppState,
    managed_agents::{self, remote_stop, retention::open_retention_db},
};
use buzz_core_pkg::{
    desktop_profile::DesktopProfile,
    desktop_stop::{StopOutcome, StopResult, StopTarget},
};
use nostr::{Event, JsonUtil, PublicKey};
use serde_json::{json, Value};
use tauri::{AppHandle, Manager};

pub(crate) fn local_id(
    conn: &mut rusqlite::Connection,
    scope: &managed_agents::retention::RetentionScope,
) -> Result<String, String> {
    let saved = prepare(conn, scope)?;
    let event: Event = serde_json::from_value(saved["event"].clone()).map_err(|e| e.to_string())?;
    Ok(DesktopProfile::read(
        &event,
        &scope.owner_keys,
        scope.relay_url.trim_end_matches('/'),
    )?
    .id)
}

/// Persist exact signed bytes before the UI sends a new Stop. No boot replay.
#[tauri::command]
pub fn prepare_desktop_stop(
    app: AppHandle,
    owner: String,
    community: String,
    desktop: String,
    agent: String,
) -> Result<Event, String> {
    let state = app.state::<AppState>();
    let scope = scope(&app, &state, &owner, &community)?;
    let event = StopTarget {
        v: 1,
        community,
        desktop,
        agent,
    }
    .sign(&scope.owner_keys)?;
    let conn = open_retention_db(&scope.db_path)?;
    // Only the current UI operation needs retry bytes; retained receiver fences
    // and results are separate. Nothing automatically drains this slot.
    conn.execute_batch("CREATE TABLE IF NOT EXISTS desktop_stop_outgoing (slot INTEGER PRIMARY KEY CHECK(slot=1), raw TEXT NOT NULL)")
        .map_err(|e| e.to_string())?;
    conn.execute("INSERT INTO desktop_stop_outgoing VALUES (1, ?1) ON CONFLICT(slot) DO UPDATE SET raw=excluded.raw", [event.as_json()])
        .map_err(|e| e.to_string())?;
    Ok(event)
}

/// Called only for live owner-private delivery. Reopening never fetches commands.
#[tauri::command]
pub async fn receive_desktop_stop(
    app: AppHandle,
    owner: String,
    community: String,
    event: Event,
) -> Result<Option<Event>, String> {
    tokio::task::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let _transition = state
            .managed_agent_runtime_transition
            .lock()
            .map_err(|e| e.to_string())?;
        let scope = scope(&app, &state, &owner, &community)?;
        let target = StopTarget::read(&event, &scope.owner_keys, &community)?;
        let mut conn = open_retention_db(&scope.db_path)?;
        let desktop = local_id(&mut conn, &scope)?;
        managed_agents::placement::observe(&conn, &event, &scope.owner_keys, &community)?;
        if desktop != target.desktop {
            return Ok(None);
        }
        // Local possession alone is insufficient after an account switch:
        // verify the stored agent's owner delegation against the request author.
        if let Some(raw) = remote_stop::saved_result(&conn, &event.id.to_hex())? {
            let saved = Event::from_json(raw).map_err(|e| e.to_string())?;
            StopResult::read(&saved, &scope.owner_keys, &event, &community)?;
            return Ok(Some(saved));
        }
        if managed_agents::placement::desired(&conn, &target.agent)?
            .is_some_and(|(host, _)| host == desktop)
        {
            let result = StopResult {
                target,
                request: event.id.to_hex(),
                outcome: StopOutcome::Unknown,
            }
            .sign(&scope.owner_keys)?;
            remote_stop::save_result(&mut conn, &event.id.to_hex(), &result.as_json())?;
            return Ok(Some(result));
        }
        let owned = owned_local(&app, &state, &owner, &target.agent)?;
        remote_stop::receive(
            &mut conn,
            &event,
            &scope.owner_keys,
            &community,
            &desktop,
            owned,
            |target| {
                managed_agents::stop_pair_locked(
                    target.agent.clone(),
                    community.clone(),
                    app.clone(),
                )
                .map(|_| ())
            },
        )
    })
    .await
    .map_err(|e| format!("Desktop Stop task failed: {e}"))?
}

/// Result queries never dispatch/replay a request. Missing means Unknown.
#[tauri::command]
pub fn read_desktop_stop_results(
    app: AppHandle,
    owner: String,
    community: String,
    request: Event,
    events: Vec<Event>,
) -> Result<Value, String> {
    let state = app.state::<AppState>();
    let scope = scope(&app, &state, &owner, &community)?;
    StopTarget::read(&request, &scope.owner_keys, &community)?;
    if events.len() > 16 {
        return Err("too many Desktop Stop results".into());
    }
    let mut outcome = StopOutcome::Unknown;
    for event in events {
        let result = StopResult::read(&event, &scope.owner_keys, &request, &community)?;
        // Persisted terminal result beats a later Unknown after bounded eviction.
        if result.outcome != StopOutcome::Unknown {
            outcome = result.outcome;
        }
    }
    Ok(json!(outcome))
}

/// Local possession/profile alone never establishes owner authority.
pub(super) fn owned_local(
    app: &AppHandle,
    state: &AppState,
    owner: &str,
    agent: &str,
) -> Result<bool, String> {
    let _store = state
        .managed_agents_store_lock
        .lock()
        .map_err(|e| e.to_string())?;
    let records = managed_agents::load_managed_agents(app)?;
    Ok(records.iter().find(|r| r.pubkey == agent).is_some_and(|r| {
        r.backend == managed_agents::BackendKind::Local
            && r.auth_tag
                .as_deref()
                .and_then(|tag| {
                    let key = PublicKey::from_hex(agent).ok()?;
                    buzz_sdk_pkg::nip_oa::verify_auth_tag(tag, &key).ok()
                })
                .is_some_and(|key| key.to_hex() == owner)
    }))
}
