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
    Ok(
        json!({ "event": prepare_report(&mut open_retention_db(&scope.db_path)?, &scope, facts, nostr::Timestamp::now)? }),
    )
}

fn prepare_report(
    conn: &mut Connection,
    scope: &RetentionScope,
    facts: Vec<RuntimeFact>,
    clock: impl FnOnce() -> nostr::Timestamp,
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
        previous => {
            let now = clock();
            // Keep the prior retry record until real time advances. Signing tied
            // ciphertext can lose NIP-33's lower-ID tie; never cache that loss or
            // future-date a replacement. The existing pulse/reconnect/Refresh
            // retries discovery, not a captured projection, without waiting here.
            if previous.as_ref().is_some_and(|e| now <= e.created_at) {
                return Err("Desktop capability facts deferred until the clock advances".into());
            }
            let event = report.sign_at(&scope.owner_keys, now)?;
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
    fn changed_facts_defer_until_real_clock_advances_then_win_signed_order() {
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
            || nostr::Timestamp::from(1000),
        )
        .unwrap();
        let mut reopened = open_retention_db(&scope.db_path).unwrap();
        assert_eq!(
            prepare_report(&mut reopened, &scope, vec![], || panic!(
                "unchanged must not sign"
            ))
            .unwrap(),
            first
        );
        assert_eq!(reopened.total_changes(), 0);
        let mut facts = vec![RuntimeFact {
            id: "goose".into(),
            availability: "available".into(),
            requires_external_cli: true,
            max_parallelism: None,
        }];
        for now in [1000, 990, 999, 1000] {
            let error = prepare_report(&mut reopened, &scope, facts.clone(), || {
                nostr::Timestamp::from(now)
            })
            .unwrap_err();
            assert!(error.contains("clock advances"));
            assert_eq!(reopened.total_changes(), 0, "deferral must not persist");
            // Returning to old facts cancels the proposed change, even after a
            // restart/rollback: no deferred payload or timestamp renewal survives.
            assert_eq!(
                prepare_report(&mut reopened, &scope, vec![], || panic!("exact retry")).unwrap(),
                first
            );
            reopened = open_retention_db(&scope.db_path).unwrap();
        }
        // The retry observes today's facts, not the projection first deferred.
        facts[0].availability = "cli_missing".into();
        let changed = prepare_report(&mut reopened, &scope, facts.clone(), || {
            nostr::Timestamp::from(1001)
        })
        .unwrap();
        first.verify().unwrap();
        changed.verify().unwrap();
        assert_eq!(changed.created_at.as_secs(), 1001, "no future timestamp");
        assert!(changed.created_at > first.created_at);
        assert_eq!(changed.tags, first.tags);
        for events in [
            vec![first.clone(), changed.clone()],
            vec![changed.clone(), first],
        ] {
            let rows =
                DesktopCapabilities::read_latest(events, &scope.owner_keys, &scope.relay_url)
                    .unwrap();
            assert_eq!(rows.len(), 1);
            assert_eq!(rows[0].0.runtimes, facts);
            assert_eq!(rows[0].1, 1001);
        }
        let mut reopened = open_retention_db(&scope.db_path).unwrap();
        let mut invalid = facts.clone();
        invalid[0].max_parallelism = Some(0);
        assert!(prepare_report(&mut reopened, &scope, invalid, || {
            nostr::Timestamp::from(1002)
        })
        .is_err());
        assert_eq!(
            prepare_report(&mut reopened, &scope, facts, || panic!("exact retry")).unwrap(),
            changed
        );
        assert_eq!(
            reopened.total_changes(),
            0,
            "failed signing must not persist"
        );
        reopened
            .execute("UPDATE desktop_capabilities SET raw = 'corrupt'", [])
            .unwrap();
        assert!(prepare_report(&mut reopened, &scope, vec![], nostr::Timestamp::now).is_err());
    }
}
