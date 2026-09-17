//! Durable read-only Desktop profiles, separate from persona publication queues.
use buzz_core_pkg::desktop_profile::DesktopProfile;
use nostr::{Event, JsonUtil};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior};
use serde_json::{json, Value};
use tauri::{AppHandle, State};

use crate::app_state::AppState;
use crate::managed_agents::retention::{active_retention_scope, open_retention_db, RetentionScope};

fn scope(
    app: &AppHandle,
    state: &AppState,
    owner: &str,
    community: &str,
) -> Result<RetentionScope, String> {
    let scope = active_retention_scope(app, state)?;
    if scope.owner_keys.public_key().to_hex() != owner
        || scope.relay_url.trim_end_matches('/') != community
    {
        return Err("Desktop profile scope changed".into());
    }
    Ok(scope)
}

fn prepare(conn: &mut Connection, scope: &RetentionScope) -> Result<Value, String> {
    // SQLite serializes concurrent startup/open requests across processes. The ID
    // and exact ciphertext/signature commit together, before any network write.
    let tx = conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|e| e.to_string())?;
    tx.execute_batch(
        "CREATE TABLE IF NOT EXISTS desktop_profile (
        slot INTEGER PRIMARY KEY CHECK(slot = 1), raw TEXT NOT NULL);",
    )
    .map_err(|e| e.to_string())?;
    let saved: Option<String> = tx
        .query_row(
            "SELECT raw FROM desktop_profile WHERE slot = 1",
            [],
            |row| row.get(0),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    let raw = match saved {
        Some(saved) => saved,
        None => {
            let profile = DesktopProfile::new(
                scope.relay_url.trim_end_matches('/').to_owned(),
                uuid::Uuid::new_v4().simple().to_string(),
            )?;
            let raw = profile.sign(&scope.owner_keys)?.as_json();
            tx.execute("INSERT INTO desktop_profile VALUES (1, ?1)", [&raw])
                .map_err(|e| e.to_string())?;
            raw
        }
    };
    let event = Event::from_json(&raw).map_err(|_| "invalid saved Desktop profile")?;
    DesktopProfile::read(
        &event,
        &scope.owner_keys,
        scope.relay_url.trim_end_matches('/'),
    )?;
    tx.commit().map_err(|e| e.to_string())?;
    Ok(json!({ "event": event }))
}

/// Prepare or reload the identical owner/community-local installation profile.
#[tauri::command]
pub fn prepare_desktop_profile(
    app: AppHandle,
    state: State<'_, AppState>,
    owner: String,
    community: String,
) -> Result<Value, String> {
    let scope = scope(&app, &state, &owner, &community)?;
    prepare(&mut open_retention_db(&scope.db_path)?, &scope)
}

/// Authenticate and decrypt a bounded relay result before exposing any row to UI.
#[tauri::command]
pub fn read_desktop_profiles(
    app: AppHandle,
    state: State<'_, AppState>,
    owner: String,
    community: String,
    events: Vec<Event>,
) -> Result<Value, String> {
    let scope = scope(&app, &state, &owner, &community)?;
    if events.len() > 100 {
        return Err("too many Desktop profiles".into());
    }
    let rows: Result<Vec<_>, String> = events.iter().map(|event| {
        let profile = DesktopProfile::read(event, &scope.owner_keys, &community)?;
        Ok(json!({ "id": profile.id, "name": profile.name, "updated": event.created_at.as_secs() }))
    }).collect();
    Ok(json!(rows?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::managed_agents::retention::scoped_retention_db_path;

    #[test]
    fn durable_identity_exact_retry_without_rewrites() {
        let dir = tempfile::tempdir().unwrap();
        let owner_keys = nostr::Keys::generate();
        let scope = RetentionScope {
            db_path: dir.path().join("one.db"),
            relay_url: "wss://one.example".into(),
            owner_keys,
        };
        let first = prepare(&mut open_retention_db(&scope.db_path).unwrap(), &scope).unwrap();
        let mut reopened = open_retention_db(&scope.db_path).unwrap();
        assert_eq!(prepare(&mut reopened, &scope).unwrap(), first);
        // No mutable ACK state: a confirmed or failed publish leaves the same
        // signed record available for exact retry, without another native write.
        let accepted = prepare(&mut reopened, &scope).unwrap();
        assert_eq!(accepted, first);
        assert_eq!(reopened.total_changes(), 0, "no repeated native writes");
        let other = RetentionScope {
            db_path: dir.path().join("two.db"),
            relay_url: scope.relay_url.clone(),
            owner_keys: scope.owner_keys.clone(),
        };
        let second = prepare(&mut open_retention_db(&other.db_path).unwrap(), &other).unwrap();
        assert_ne!(first["event"]["tags"], second["event"]["tags"]);
        let a = scoped_retention_db_path(
            dir.path(),
            &scope.relay_url,
            &scope.owner_keys.public_key().to_hex(),
        );
        assert_ne!(
            a,
            scoped_retention_db_path(
                dir.path(),
                "wss://two.example",
                &scope.owner_keys.public_key().to_hex()
            )
        );
        assert_ne!(
            a,
            scoped_retention_db_path(
                dir.path(),
                &scope.relay_url,
                &nostr::Keys::generate().public_key().to_hex()
            )
        );
        reopened
            .execute("UPDATE desktop_profile SET raw = 'corrupt'", [])
            .unwrap();
        assert!(prepare(&mut reopened, &scope).is_err());
    }
}
