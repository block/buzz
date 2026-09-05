//! Exact, scope-local cleanup obligations. Never infer deletion from disk absence.
use rusqlite::{params, Connection, OptionalExtension};

pub(super) fn initialize(conn: &Connection) -> Result<(), String> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS managed_agent_deletions (
            owner TEXT NOT NULL,
            agent TEXT NOT NULL,
            deleted_at INTEGER NOT NULL,
            PRIMARY KEY (owner, agent)
        );",
    )
    .map_err(|error| format!("failed to initialize agent deletion journal: {error}"))
}

/// Caller includes this in the transaction that retains tombstones/purges heads.
pub(crate) fn record(
    conn: &Connection,
    owner: &str,
    agent: &str,
    deleted_at: i64,
) -> Result<(), String> {
    conn.execute(
        "INSERT INTO managed_agent_deletions(owner, agent, deleted_at) VALUES (?1, ?2, ?3)
         ON CONFLICT(owner, agent) DO UPDATE SET deleted_at = MAX(deleted_at, excluded.deleted_at)",
        params![owner, agent, deleted_at],
    )
    .map_err(|error| format!("failed to record agent deletion intent: {error}"))?;
    Ok(())
}

pub(crate) fn pending(conn: &Connection, owner: &str, agent: &str) -> Result<bool, String> {
    conn.query_row(
        "SELECT 1 FROM managed_agent_deletions WHERE owner = ?1 AND agent = ?2",
        params![owner, agent],
        |_| Ok(()),
    )
    .optional()
    .map(|row| row.is_some())
    .map_err(|error| format!("failed to read agent deletion intent: {error}"))
}

pub(crate) fn agents(conn: &Connection, owner: &str) -> Result<Vec<String>, String> {
    let mut stmt = conn
        .prepare("SELECT agent FROM managed_agent_deletions WHERE owner = ?1 ORDER BY agent")
        .map_err(|error| format!("failed to read agent deletion journal: {error}"))?;
    let rows = stmt
        .query_map([owner], |row| row.get(0))
        .map_err(|error| format!("failed to query agent deletion journal: {error}"))?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("failed to decode agent deletion journal: {error}"))
}

/// Clear only after process, JSON, overlay, and key cleanup all succeed.
pub(crate) fn finish(conn: &Connection, owner: &str, agent: &str) -> Result<(), String> {
    conn.execute(
        "DELETE FROM managed_agent_deletions WHERE owner = ?1 AND agent = ?2",
        params![owner, agent],
    )
    .map_err(|error| format!("failed to finish agent deletion intent: {error}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::managed_agents::retention::{
        commit_inbound_tombstone_covering, get_retained_event, open_retention_db, retain_event,
        retain_inbound_event, InboundOutcome, RetainedEvent,
    };

    #[test]
    fn inbound_failure_keeps_exact_cleanup_intent_across_restart_and_retries() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("retention.db");
        let head = RetainedEvent {
            kind: 30179,
            pubkey: "owner".into(),
            d_tag: "agent".into(),
            content: "head".into(),
            created_at: 10,
            raw_event: "{}".into(),
            pending_sync: false,
        };
        let tombstone = RetainedEvent {
            kind: 5,
            d_tag: "30177:agent".into(),
            created_at: 20,
            ..head.clone()
        };
        {
            let conn = open_retention_db(&path).unwrap();
            retain_event(&conn, &head).unwrap();
            let error = commit_inbound_tombstone_covering(
                &conn,
                &tombstone,
                &[30177, 30179],
                "owner",
                "agent",
                || {
                    assert!(
                        pending(&conn, "owner", "agent")?,
                        "intent precedes destructive cleanup"
                    );
                    assert!(get_retained_event(&conn, 30179, "owner", "agent")?.is_none());
                    Err("injected key cleanup failure".into())
                },
            )
            .unwrap_err();
            assert!(error.contains("injected key"));
        }
        let conn = open_retention_db(&path).unwrap();
        assert_eq!(agents(&conn, "owner").unwrap(), vec!["agent"]);
        assert!(agents(&conn, "other-owner").unwrap().is_empty());
        let newer_head = RetainedEvent {
            created_at: 30,
            ..head
        };
        assert_eq!(
            retain_inbound_event(&conn, &newer_head).unwrap(),
            InboundOutcome::Skipped,
            "new materialization cannot race old key cleanup"
        );
        assert_eq!(
            commit_inbound_tombstone_covering(
                &conn,
                &tombstone,
                &[30177, 30179],
                "owner",
                "agent",
                || Ok(()),
            )
            .unwrap(),
            InboundOutcome::Applied
        );
        assert!(!pending(&conn, "owner", "agent").unwrap());
        assert_eq!(
            retain_inbound_event(&conn, &newer_head).unwrap(),
            InboundOutcome::Applied
        );
    }
}
