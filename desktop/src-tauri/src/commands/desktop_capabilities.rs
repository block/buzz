//! Private Desktop reports reuse the local catalog authority and retention scope.
use super::desktop_profiles::{prepare, scope};
use crate::{
    app_state::AppState,
    managed_agents::{
        retention::{open_retention_db, RetentionScope},
        AcpRuntimeCatalogEntry, HarnessSource,
    },
};
use buzz_core_pkg::{
    desktop_capabilities::{DesktopCapabilities, RuntimeFact},
    desktop_profile::DesktopProfile,
};
use nostr::{Event, JsonUtil};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior};
use serde_json::{json, Value};
use tauri::{AppHandle, State};

fn project(catalog: Vec<AcpRuntimeCatalogEntry>) -> Result<Vec<RuntimeFact>, String> {
    catalog
        .into_iter()
        .filter(|r| r.source == HarnessSource::Builtin)
        .map(|r| {
            Ok(RuntimeFact {
                id: r.id,
                availability: serde_json::from_value(
                    serde_json::to_value(r.availability).map_err(|e| e.to_string())?,
                )
                .map_err(|e| e.to_string())?,
                requires_external_cli: r.requires_external_cli,
                max_parallelism: r.max_parallelism,
            })
        })
        .collect()
}

/// Cached discovery only; Settings → Agents remains the local setup/check-again UI.
#[tauri::command]
pub async fn prepare_desktop_capabilities(
    app: AppHandle,
    state: State<'_, AppState>,
    owner: String,
    community: String,
) -> Result<Value, String> {
    // Serialize discovery + persistence so an older native completion cannot
    // overwrite a newer projection when observers cancel/restart.
    static SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
    let _guard = SERIAL.lock().await;
    scope(&app, &state, &owner, &community)?;
    let facts =
        project(super::agent_discovery::discover_acp_providers(app.clone(), Some(false)).await?)?;
    let scope = scope(&app, &state, &owner, &community)?;
    Ok(json!({ "event": prepare_report(&mut open_retention_db(&scope.db_path)?, &scope, facts)? }))
}

fn prepare_report(
    conn: &mut Connection,
    scope: &RetentionScope,
    facts: Vec<RuntimeFact>,
) -> Result<Event, String> {
    let saved = prepare(conn, scope)?;
    let profile: Event =
        serde_json::from_value(saved["event"].clone()).map_err(|e| e.to_string())?;
    let community = scope.relay_url.trim_end_matches('/');
    let report = DesktopCapabilities::new(
        DesktopProfile::read(&profile, &scope.owner_keys, community)?,
        facts,
    );
    let tx = conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|e| e.to_string())?;
    tx.execute_batch("CREATE TABLE IF NOT EXISTS desktop_capabilities (slot INTEGER PRIMARY KEY CHECK(slot = 1), raw TEXT NOT NULL);").map_err(|e| e.to_string())?;
    let raw: Option<String> = tx
        .query_row(
            "SELECT raw FROM desktop_capabilities WHERE slot = 1",
            [],
            |row| row.get(0),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    let previous = raw
        .map(|raw| Event::from_json(raw).map_err(|e| e.to_string()))
        .transpose()?;
    let unchanged = previous
        .as_ref()
        .map(|e| {
            DesktopCapabilities::read(e, &scope.owner_keys, community).map(|old| old == report)
        })
        .transpose()?
        .unwrap_or(false);
    let event = match previous {
        Some(event) if unchanged => event,
        _ => {
            let event = report.sign(&scope.owner_keys)?;
            tx.execute(
                "INSERT OR REPLACE INTO desktop_capabilities VALUES (1, ?1)",
                [event.as_json()],
            )
            .map_err(|e| e.to_string())?;
            event
        }
    };
    tx.commit().map_err(|e| e.to_string())?;
    Ok(event)
}

/// Read only verified owner/community reports, newest signed time then lower ID.
#[tauri::command]
pub fn read_desktop_capabilities(
    app: AppHandle,
    state: State<'_, AppState>,
    owner: String,
    community: String,
    events: Vec<Event>,
) -> Result<Value, String> {
    let scope = scope(&app, &state, &owner, &community)?;
    let rows: Vec<_> = DesktopCapabilities::read_latest(events, &scope.owner_keys, &community)?.into_iter()
        .map(|(report, reported)| json!({ "id": report.id, "reported": reported, "runtimes": report.runtimes })).collect();
    Ok(json!(rows))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unchanged_facts_reopen_exact_bytes_changed_facts_replace_atomically() {
        let dir = tempfile::tempdir().unwrap();
        let scope = RetentionScope {
            db_path: dir.path().join("report.db"),
            relay_url: "wss://one.example".into(),
            owner_keys: nostr::Keys::generate(),
        };
        let first = prepare_report(
            &mut open_retention_db(&scope.db_path).unwrap(),
            &scope,
            vec![],
        )
        .unwrap();
        let mut reopened = open_retention_db(&scope.db_path).unwrap();
        assert_eq!(
            prepare_report(&mut reopened, &scope, vec![]).unwrap(),
            first
        );
        assert_eq!(reopened.total_changes(), 0);
        let facts = vec![RuntimeFact {
            id: "goose".into(),
            availability: "available".into(),
            requires_external_cli: true,
            max_parallelism: None,
        }];
        let changed = prepare_report(&mut reopened, &scope, facts).unwrap();
        assert_ne!(changed.id, first.id);
        assert_eq!(changed.tags, first.tags);
        reopened
            .execute("UPDATE desktop_capabilities SET raw = 'corrupt'", [])
            .unwrap();
        assert!(prepare_report(&mut reopened, &scope, vec![]).is_err());
    }
}
