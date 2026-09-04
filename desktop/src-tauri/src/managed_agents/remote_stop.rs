//! Durable no-auto-start fence shared by ordinary Desktop launch paths.
use super::retention::{open_retention_db, scoped_retention_db_path};
use super::ManagedAgentRuntimeKey;
use rusqlite::{Connection, OptionalExtension};
use tauri::{AppHandle, Manager};

fn schema(conn: &Connection) -> Result<(), String> {
    conn.execute_batch("CREATE TABLE IF NOT EXISTS desktop_stop_fence (
        agent TEXT PRIMARY KEY, stamp INTEGER NOT NULL, event_id TEXT NOT NULL, blocked INTEGER NOT NULL);")
        .map_err(|e| e.to_string())
}

/// Explicit local Start captures the Stop fence before its asynchronous preflight.
/// Automatic starts and Restart continuations never receive this permission.
pub(crate) struct ResumeTicket {
    previous: Option<String>,
}

pub(crate) fn capture_resume(
    app: &AppHandle,
    key: &ManagedAgentRuntimeKey,
    owner: &str,
) -> Result<ResumeTicket, String> {
    let conn = connection(app, key, owner)?;
    schema(&conn)?;
    let previous = conn
        .query_row(
            "SELECT event_id FROM desktop_stop_fence WHERE agent=?1",
            [&key.pubkey],
            |r| r.get(0),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    Ok(ResumeTicket { previous })
}

fn connection(
    app: &AppHandle,
    key: &ManagedAgentRuntimeKey,
    owner: &str,
) -> Result<Connection, String> {
    let path =
        scoped_retention_db_path(&super::managed_agents_base_dir(app)?, &key.relay_url, owner);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    open_retention_db(&path)
}

/// Every ordinary spawn passes here, including restore/config/reconcile.
/// Caller holds the existing transition lock through child registration.
pub(crate) fn check_launch(
    app: &AppHandle,
    key: &ManagedAgentRuntimeKey,
    owner: Option<&str>,
    resume: Option<&ResumeTicket>,
) -> Result<(), String> {
    let state = app.state::<crate::app_state::AppState>();
    let current_owner = state.signing_keys()?.public_key().to_hex();
    if owner != Some(current_owner.as_str()) {
        return Err("Desktop launch owner changed".into());
    }
    let conn = connection(app, key, &current_owner)?;
    schema(&conn)?;
    let row: Option<(String, bool)> = conn
        .query_row(
            "SELECT event_id, blocked FROM desktop_stop_fence WHERE agent=?1",
            [&key.pubkey],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    allow_launch(row.as_ref(), resume)
}

fn allow_launch(row: Option<&(String, bool)>, resume: Option<&ResumeTicket>) -> Result<(), String> {
    if let Some(ticket) = resume {
        if ticket.previous.as_ref() != row.map(|(id, _)| id) {
            return Err("A newer Stop interrupted this Start".into());
        }
    } else if row.is_some_and(|(_, blocked)| *blocked) {
        return Err(
            "Stopped from another Desktop. Use Start agent to start it again explicitly.".into(),
        );
    }
    Ok(())
}

/// A failed spawn must not unblock config/restore. Commit only after the child
/// has its ordinary receipt and tracked handle, still under the transition lock.
pub(crate) fn finish_resume(
    app: &AppHandle,
    key: &ManagedAgentRuntimeKey,
    owner: Option<&str>,
    ticket: Option<&ResumeTicket>,
) -> Result<(), String> {
    if ticket.is_none() {
        return Ok(());
    }
    let owner = owner.ok_or("Desktop launch owner unavailable")?;
    check_launch(app, key, Some(owner), ticket)?;
    connection(app, key, owner)?
        .execute(
            "UPDATE desktop_stop_fence SET blocked=0 WHERE agent=?1",
            [&key.pubkey],
        )
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn launch_fence_requires_explicit_start_and_rejects_delayed_preflight() {
        let stopped = ("stop-a".to_owned(), true);
        let resumed = ("stop-a".to_owned(), false);
        let new_stop = ("stop-b".to_owned(), true);
        assert!(allow_launch(None, None).is_ok());
        assert!(allow_launch(Some(&stopped), None).is_err());
        assert!(allow_launch(Some(&resumed), None).is_ok());
        let ticket = ResumeTicket {
            previous: Some("stop-a".to_owned()),
        };
        assert!(allow_launch(Some(&stopped), Some(&ticket)).is_ok());
        assert!(allow_launch(Some(&new_stop), Some(&ticket)).is_err());
        let before_any_stop = ResumeTicket { previous: None };
        assert!(allow_launch(Some(&stopped), Some(&before_any_stop)).is_err());
        assert!(allow_launch(None, Some(&before_any_stop)).is_ok());
    }
}
